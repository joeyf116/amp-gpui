//! What the assistant knows about this player. It stays on this PC, under this
//! Windows account, and all of it can be read and deleted in the panel.
//!
//! Facts are things the player said. The rest comes from what they do with
//! suggestions: which they keep, which they back out of, and how they move the
//! knobs after trying one, which says more than anything they could put in words.

use crate::prompt::kind_name;
use amp_preset::BlockPreset;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Write, fs, io, path::Path};

const FILE: &str = "taste.json";
const MAX_FACTS: usize = 40;
const MAX_FACT_LENGTH: usize = 200;
/// A habit counts once it's happened this often.
const HABIT_COUNT: u32 = 2;
/// Smaller average moves are just noise.
const HABIT_SHARE: f32 = 0.05;
const SHOWN_TAGS: usize = 6;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Fact {
    pub text: String,
}

/// How often a knob was moved after trying a suggestion, and which way overall,
/// as a share of the knob's range.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
struct Habit {
    count: u32,
    total: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Taste {
    pub facts: Vec<Fact>,
    /// Set by the player. While off, nothing new is learned or sent.
    pub learning: bool,
    kept: BTreeMap<String, u32>,
    rejected: BTreeMap<String, u32>,
    habits: BTreeMap<String, Habit>,
}

impl Default for Taste {
    fn default() -> Self {
        Self {
            facts: Vec::new(),
            learning: true,
            kept: BTreeMap::new(),
            rejected: BTreeMap::new(),
            habits: BTreeMap::new(),
        }
    }
}

fn add_tags(counts: &mut BTreeMap<String, u32>, tags: &[String]) {
    for tag in tags {
        *counts.entry(tag.to_lowercase()).or_default() += 1;
    }
}

/// The most frequent tags, most frequent first.
fn top_tags(counts: &BTreeMap<String, u32>) -> Vec<String> {
    let mut tags: Vec<(&String, &u32)> = counts.iter().collect();
    tags.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    tags.iter()
        .take(SHOWN_TAGS)
        .map(|(tag, _)| (*tag).clone())
        .collect()
}

impl Taste {
    pub fn load(folder: &Path) -> Self {
        fs::read_to_string(folder.join(FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, folder: &Path) -> io::Result<()> {
        fs::create_dir_all(folder)?;
        let path = folder.join(FILE);
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_string_pretty(self)?)?;
        fs::rename(temporary, path)
    }

    /// Adds a fact the player stated. Repeats are dropped, and when the list is
    /// full the oldest goes.
    pub fn remember(&mut self, text: &str) -> bool {
        let text: String = text.trim().chars().take(MAX_FACT_LENGTH).collect();
        let repeated = self
            .facts
            .iter()
            .any(|fact| fact.text.eq_ignore_ascii_case(&text));
        if text.is_empty() || repeated {
            return false;
        }
        self.facts.push(Fact { text });
        if self.facts.len() > MAX_FACTS {
            self.facts.remove(0);
        }
        true
    }

    pub fn forget(&mut self, index: usize) {
        if index < self.facts.len() {
            self.facts.remove(index);
        }
    }

    pub fn clear(&mut self) {
        *self = Self {
            learning: self.learning,
            ..Self::default()
        };
    }

    pub fn note_kept(&mut self, tags: &[String]) {
        if self.learning {
            add_tags(&mut self.kept, tags);
        }
    }

    pub fn note_rejected(&mut self, tags: &[String]) {
        if self.learning {
            add_tags(&mut self.rejected, tags);
        }
    }

    /// Learns from how the player moved the knobs of a suggestion they went on
    /// to keep. Blocks are paired up by position within their kind.
    pub fn note_adjustments(&mut self, suggested: &[BlockPreset], kept: &[BlockPreset]) {
        if !self.learning {
            return;
        }
        for (before, after) in crate::proposal::paired(suggested, kept) {
            for (index, spec) in before.kind.params().iter().enumerate() {
                let moved = after.param(index) - before.param(index);
                if moved.abs() > f32::EPSILON {
                    let key = format!("{}.{}", kind_name(before.kind), spec.id);
                    let habit = self.habits.entry(key).or_default();
                    habit.count += 1;
                    habit.total += moved / (spec.max - spec.min);
                }
            }
        }
    }

    /// Whether there's anything beyond the stated facts.
    pub fn has_signals(&self) -> bool {
        !self.kept.is_empty() || !self.rejected.is_empty() || !self.habits.is_empty()
    }

    /// One line per thing learned from what the player did, for the panel and
    /// for Claude.
    pub fn signal_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let kept = top_tags(&self.kept);
        if !kept.is_empty() {
            lines.push(format!("Keeps tones like: {}", kept.join(", ")));
        }
        let rejected = top_tags(&self.rejected);
        if !rejected.is_empty() {
            lines.push(format!("Backs out of tones like: {}", rejected.join(", ")));
        }
        for (key, habit) in &self.habits {
            let average = habit.total / habit.count as f32;
            if habit.count >= HABIT_COUNT && average.abs() >= HABIT_SHARE {
                let way = if average < 0.0 { "lowers" } else { "raises" };
                let (block, param) = key.split_once('.').unwrap_or((key, ""));
                lines.push(format!(
                    "Usually {way} {block} {param} after trying a suggestion, by about {:.0}% of \
                     its range",
                    average.abs() * 100.0
                ));
            }
        }
        lines
    }

    /// Everything Claude should know, or nothing while learning is off.
    pub fn summary(&self) -> String {
        if !self.learning {
            return String::new();
        }
        let mut text = String::new();
        for fact in &self.facts {
            let _ = writeln!(text, "- {}", fact.text);
        }
        for line in self.signal_lines() {
            let _ = writeln!(text, "- {line}");
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amp_dsp::BlockKind;

    fn tube(gain: f32) -> Vec<BlockPreset> {
        let mut block = BlockPreset::new(BlockKind::Tube, true);
        block.params.insert("gain".into(), gain);
        vec![block]
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|tag| (*tag).to_owned()).collect()
    }

    #[test]
    fn facts_stay_short_and_unrepeated() {
        let mut taste = Taste::default();
        assert!(taste.remember("Plays a Strat"));
        assert!(!taste.remember("plays a strat"));
        assert!(!taste.remember("   "));
        assert_eq!(taste.facts.len(), 1);
        for index in 0..MAX_FACTS + 5 {
            taste.remember(&format!("Fact {index}"));
        }
        assert_eq!(taste.facts.len(), MAX_FACTS);
        assert_eq!(taste.facts[0].text, "Fact 5");
    }

    #[test]
    fn a_repeated_adjustment_becomes_a_habit() {
        let mut taste = Taste::default();
        taste.note_adjustments(&tube(6.0), &tube(4.0));
        assert!(taste.signal_lines().is_empty());
        taste.note_adjustments(&tube(7.0), &tube(5.0));
        let lines = taste.signal_lines();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("lowers tube gain"), "{}", lines[0]);
        assert!(lines[0].contains("20%"), "{}", lines[0]);
    }

    #[test]
    fn tiny_moves_are_not_a_habit() {
        let mut taste = Taste::default();
        taste.note_adjustments(&tube(5.0), &tube(5.1));
        taste.note_adjustments(&tube(5.0), &tube(5.1));
        assert!(taste.signal_lines().is_empty());
    }

    #[test]
    fn keeps_and_rejects_are_ranked() {
        let mut taste = Taste::default();
        taste.note_kept(&tags(&["clean", "Blues"]));
        taste.note_kept(&tags(&["blues"]));
        taste.note_rejected(&tags(&["high gain"]));
        let lines = taste.signal_lines();
        assert_eq!(lines[0], "Keeps tones like: blues, clean");
        assert_eq!(lines[1], "Backs out of tones like: high gain");
    }

    #[test]
    fn with_learning_off_nothing_is_recorded_or_sent() {
        let mut taste = Taste::default();
        taste.remember("Plays a Les Paul");
        taste.learning = false;
        taste.note_kept(&tags(&["clean"]));
        taste.note_adjustments(&tube(6.0), &tube(4.0));
        assert!(!taste.has_signals());
        assert_eq!(taste.summary(), "");
    }

    #[test]
    fn clearing_forgets_everything_but_the_switch() {
        let mut taste = Taste::default();
        taste.remember("Drop D");
        taste.learning = false;
        taste.clear();
        assert!(taste.facts.is_empty());
        assert!(!taste.learning);
    }

    #[test]
    fn taste_survives_a_save_and_a_bad_file_reads_as_empty() {
        let folder = std::env::temp_dir().join(format!("amp-taste-{}", std::process::id()));
        let mut taste = Taste::default();
        taste.remember("Seven strings");
        taste.note_kept(&tags(&["metal"]));
        taste.save(&folder).unwrap();
        assert_eq!(Taste::load(&folder), taste);
        fs::write(folder.join(FILE), "{").unwrap();
        assert_eq!(Taste::load(&folder), Taste::default());
        fs::remove_dir_all(folder).unwrap();
    }
}
