//! Suggestions. Claude's input is checked here before anything reaches the rig,
//! so a bad value becomes a note back to Claude rather than a surprise.

use crate::{session::Snapshot, tools::ToneSource};
use amp_dsp::BlockKind;
use amp_preset::{BlockPreset, CaptureEntry, Preset, ToneOrigin};
use amp_tone3000::{Model, Tone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_BLOCKS: usize = 12;
const MAX_NAME: usize = 60;
const MAX_TAGS: usize = 5;

/// A TONE3000 tone to credit on a suggestion, shown before it downloads.
pub type Credit = ToneOrigin;

/// A tone Claude suggests. Nothing here has touched the rig.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Proposal {
    pub preset: Preset,
    /// Why it should sound right, in a sentence or two.
    pub why: String,
    /// What's different from the tone that was loaded.
    pub changes: Vec<String>,
    /// Whether this edits the loaded tone instead of starting a new one.
    pub edits_current: bool,
}

impl Proposal {
    /// The TONE3000 tones this suggestion uses, for their credit.
    pub fn credits(&self) -> impl Iterator<Item = &Credit> {
        self.preset
            .blocks
            .iter()
            .filter_map(|block| block.origin.as_ref())
    }

    /// The chain in signal order, in a line.
    pub fn chain_line(&self) -> String {
        self.preset
            .blocks
            .iter()
            .map(|block| block.kind.title())
            .collect::<Vec<_>>()
            .join(" > ")
    }

    pub fn downloads(&self) -> usize {
        self.preset
            .blocks
            .iter()
            .filter(|block| needs_download(block))
            .count()
    }
}

/// Whether a block is waiting for a model file from TONE3000.
pub fn needs_download(block: &BlockPreset) -> bool {
    block.origin.is_some() && block.asset.is_none()
}

/// The block a tone's models run in.
pub fn tone_block_kind(tone: &Tone) -> BlockKind {
    if tone.is_ir() {
        BlockKind::Cab
    } else if matches!(tone.gear.as_deref(), Some("pedal" | "outboard")) {
        BlockKind::Pedal
    } else {
        BlockKind::Amp
    }
}

/// The credit that travels with a downloaded model.
pub fn origin_of(tone: &Tone, model: &Model) -> ToneOrigin {
    let creator = tone.creator();
    ToneOrigin {
        tone_id: tone.id,
        model_id: model.id,
        title: tone.title.clone(),
        model: model.name.clone(),
        gear: tone.gear_label().to_owned(),
        format: tone.format_label().to_owned(),
        creator: creator.username,
        creator_avatar: creator.avatar_url,
        image: tone.image().map(str::to_owned),
        license: tone.license_label().map(str::to_owned),
    }
}

/// Pairs up blocks of the same kind, in order, so a tweak to the second delay
/// compares with the second delay.
pub(crate) fn paired<'a>(
    before: &'a [BlockPreset],
    after: &'a [BlockPreset],
) -> Vec<(&'a BlockPreset, &'a BlockPreset)> {
    let mut used = vec![false; after.len()];
    before
        .iter()
        .filter_map(|block| {
            let found = after
                .iter()
                .enumerate()
                .position(|(slot, other)| !used[slot] && other.kind == block.kind)?;
            used[found] = true;
            Some((block, &after[found]))
        })
        .collect()
}

/// What changed between two chains, in words a player would use.
pub fn change_list(before: &[BlockPreset], after: &[BlockPreset]) -> Vec<String> {
    let pairs = paired(before, after);
    let matched = |block: &BlockPreset| {
        pairs
            .iter()
            .any(|(a, b)| std::ptr::eq(*a, block) || std::ptr::eq(*b, block))
    };
    let mut changes: Vec<String> = after
        .iter()
        .filter(|block| !matched(block))
        .map(|block| format!("Add {}", block.kind.title()))
        .collect();
    changes.extend(
        before
            .iter()
            .filter(|block| !matched(block))
            .map(|block| format!("Remove {}", block.kind.title())),
    );
    for (old, new) in &pairs {
        let title = new.kind.title();
        if old.enabled != new.enabled {
            let state = if new.enabled { "Turn on" } else { "Bypass" };
            changes.push(format!("{state} {title}"));
        }
        if old.origin.as_ref().map(|origin| origin.model_id)
            != new.origin.as_ref().map(|origin| origin.model_id)
            || old.asset != new.asset && new.origin.is_none()
        {
            changes.push(format!("Change the capture in {title}"));
        }
        for (index, spec) in new.kind.params().iter().enumerate() {
            let (from, to) = (old.param(index), new.param(index));
            if (from - to).abs() >= 0.05 {
                changes.push(format!(
                    "{title} {}: {from:.1} to {to:.1}{}{}",
                    spec.name,
                    if spec.unit.is_empty() { "" } else { " " },
                    spec.unit
                ));
            }
        }
    }
    changes
}

/// Claude's tool input for one block.
#[derive(Debug, Deserialize)]
struct BlockInput {
    kind: BlockKind,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    params: serde_json::Map<String, Value>,
    #[serde(default)]
    source: Option<ToneSource>,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct ProposalInput {
    name: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    why: String,
    blocks: Vec<BlockInput>,
}

/// What a suggestion needs to know that only TONE3000 can say.
pub(crate) trait ToneLookup {
    fn origin(&self, tone_id: u64, model_id: u64) -> Result<ToneOrigin, String>;
}

fn capture_by_hash<'a>(captures: &'a [CaptureEntry], hash: &str) -> Option<&'a CaptureEntry> {
    captures
        .iter()
        .find(|capture| capture.asset.sha256.eq_ignore_ascii_case(hash))
}

fn check_slot_kind(kind: BlockKind, source: BlockKind) -> Result<(), String> {
    let fits = kind == source || (kind == BlockKind::Amp && source == BlockKind::Tube);
    if fits {
        Ok(())
    } else {
        Err(format!(
            "that slot holds {}, which can't go in a {} block",
            source.title(),
            kind.title()
        ))
    }
}

/// Turns one block of input into a real block, or says what's wrong with it.
fn build_block(
    input: &BlockInput,
    snapshot: &Snapshot,
    lookup: &dyn ToneLookup,
    adjusted: &mut Vec<String>,
) -> Result<BlockPreset, String> {
    let kind = input.kind;
    let mut block = BlockPreset::new(kind, input.enabled);
    match &input.source {
        Some(ToneSource::Current { slot }) => {
            let current = snapshot
                .preset
                .blocks
                .get(*slot)
                .ok_or_else(|| format!("the rig has no slot {slot}"))?;
            check_slot_kind(kind, current.kind)?;
            if kind == current.kind {
                block = current.clone();
                block.enabled = input.enabled;
            }
        }
        Some(ToneSource::Capture { sha256 }) => {
            let capture = capture_by_hash(&snapshot.captures, sha256)
                .ok_or_else(|| format!("no capture in the gear rack has hash {sha256}"))?;
            if !matches!(kind, BlockKind::Amp | BlockKind::Pedal) {
                return Err(format!("a capture can't run in a {} block", kind.title()));
            }
            block.asset = Some(capture.asset.clone());
            block.origin = capture.origin.clone();
        }
        Some(ToneSource::Tone3000 { tone_id, model_id }) => {
            if !matches!(kind, BlockKind::Amp | BlockKind::Pedal | BlockKind::Cab) {
                return Err(format!(
                    "a TONE3000 model can't run in a {} block",
                    kind.title()
                ));
            }
            // Already downloaded means nothing to fetch.
            let owned = snapshot.captures.iter().find(|capture| {
                capture
                    .origin
                    .as_ref()
                    .is_some_and(|origin| origin.model_id == *model_id)
            });
            match owned {
                Some(capture) => {
                    block.asset = Some(capture.asset.clone());
                    block.origin = capture.origin.clone();
                }
                None => block.origin = Some(lookup.origin(*tone_id, *model_id)?),
            }
        }
        None if matches!(kind, BlockKind::Amp | BlockKind::Pedal) => {
            return Err(format!(
                "a {} block needs a source. Use tube for the built-in amp",
                kind.title()
            ));
        }
        None => {}
    }
    for (id, value) in &input.params {
        let spec = kind
            .params()
            .iter()
            .find(|spec| spec.id == id)
            .ok_or_else(|| {
                let known: Vec<&str> = kind.params().iter().map(|spec| spec.id).collect();
                format!(
                    "{} has no param {id}. It has: {}",
                    kind.title(),
                    known.join(", ")
                )
            })?;
        let number = value
            .as_f64()
            .ok_or_else(|| format!("{id} has to be a number"))? as f32;
        let clamped = number.clamp(spec.min, spec.max);
        if clamped != number {
            adjusted.push(format!(
                "{} {id} was out of range, so it's set to {clamped}",
                kind.title()
            ));
        }
        block.params.insert(spec.id.to_owned(), clamped);
    }
    Ok(block)
}

fn check_chain(blocks: &[BlockPreset]) -> Result<(), String> {
    if blocks.len() > MAX_BLOCKS {
        return Err(format!("a chain can hold {MAX_BLOCKS} blocks at most"));
    }
    if !blocks.iter().any(|block| block.kind.is_amp()) {
        return Err("a tone needs one amp, either tube or amp".to_owned());
    }
    Ok(())
}

/// Checks Claude's suggestion against the rig. The reply is either the proposal
/// with any notes for Claude, or what to fix.
pub(crate) fn build_proposal(
    input: &Value,
    snapshot: &Snapshot,
    lookup: &dyn ToneLookup,
) -> Result<(Proposal, Vec<String>), String> {
    let input: ProposalInput =
        serde_json::from_value(input.clone()).map_err(|error| error.to_string())?;
    let name = input.name.trim();
    if !name.chars().any(char::is_alphanumeric) {
        return Err("a tone name needs at least one letter or number".to_owned());
    }
    let mut adjusted = Vec::new();
    let mut blocks = Vec::with_capacity(input.blocks.len());
    let mut keeps_current = false;
    for block in &input.blocks {
        keeps_current |= matches!(block.source, Some(ToneSource::Current { .. }));
        blocks.push(build_block(block, snapshot, lookup, &mut adjusted)?);
    }
    check_chain(&blocks)?;
    let mut preset = Preset::new(name.chars().take(MAX_NAME).collect::<String>(), blocks);
    preset.author = "Amp GPUI assistant".to_owned();
    preset.tags = input.tags.into_iter().take(MAX_TAGS).collect();
    preset.notes = input.notes;
    // A new tone isn't an edit of what's loaded, so a diff would only be noise.
    let changes = if keeps_current {
        change_list(&snapshot.preset.blocks, &preset.blocks)
    } else {
        Vec::new()
    };
    Ok((
        Proposal {
            why: input.why,
            changes,
            edits_current: keeps_current,
            preset,
        },
        adjusted,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use amp_preset::{AssetKind, AssetRef};
    use serde_json::json;

    struct NoLookup;

    impl ToneLookup for NoLookup {
        fn origin(&self, tone_id: u64, model_id: u64) -> Result<ToneOrigin, String> {
            Ok(ToneOrigin {
                tone_id,
                model_id,
                title: "Plexi".into(),
                creator: "brucew".into(),
                ..ToneOrigin::default()
            })
        }
    }

    fn snapshot() -> Snapshot {
        let mut preset = Preset::starter("Loaded");
        preset.blocks = vec![
            BlockPreset::new(BlockKind::Tube, true),
            BlockPreset::new(BlockKind::Cab, true),
        ];
        Snapshot {
            preset,
            selected: None,
            tones: Vec::new(),
            captures: Vec::new(),
            tone3000_signed_in: true,
        }
    }

    fn build(input: Value) -> Result<(Proposal, Vec<String>), String> {
        build_proposal(&input, &snapshot(), &NoLookup)
    }

    #[test]
    fn a_new_tone_fills_in_defaults_and_reads_as_a_chain() {
        let (proposal, notes) = build(json!({
            "name": "Thaw",
            "tags": ["clean"],
            "blocks": [
                {"kind": "tube", "params": {"gain": 2.0}},
                {"kind": "cab"},
                {"kind": "reverb", "params": {"mix": 15}},
            ],
        }))
        .unwrap();
        assert!(notes.is_empty());
        assert!(!proposal.edits_current);
        assert!(proposal.changes.is_empty());
        assert_eq!(proposal.preset.blocks[0].param(1), 5.0);
        assert_eq!(proposal.chain_line(), "Understory amp > Cabinet > Reverb");
    }

    #[test]
    fn out_of_range_values_clamp_and_say_so() {
        let (proposal, notes) = build(json!({
            "name": "Hot",
            "blocks": [{"kind": "tube", "params": {"gain": 99}}],
        }))
        .unwrap();
        assert_eq!(proposal.preset.blocks[0].param(0), 10.0);
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn mistakes_explain_themselves() {
        let unknown = build(json!({
            "name": "X", "blocks": [{"kind": "tube", "params": {"sparkle": 1}}]
        }));
        assert!(unknown.unwrap_err().contains("gain, bass"));
        let no_amp = build(json!({"name": "X", "blocks": [{"kind": "reverb"}]}));
        assert!(no_amp.unwrap_err().contains("needs one amp"));
        let no_source = build(json!({"name": "X", "blocks": [{"kind": "amp"}]}));
        assert!(no_source.unwrap_err().contains("needs a source"));
        let bad_slot = build(json!({
            "name": "X",
            "blocks": [{"kind": "tube", "source": {"type": "current", "slot": 9}}],
        }));
        assert!(bad_slot.unwrap_err().contains("no slot 9"));
        let nameless = build(json!({"name": "!!", "blocks": [{"kind": "tube"}]}));
        assert!(nameless.is_err());
        let bad_kind = build(json!({"name": "X", "blocks": [{"kind": "wah"}]}));
        assert!(bad_kind.is_err());
    }

    #[test]
    fn editing_keeps_a_slot_and_changes_only_what_was_asked() {
        let mut snapshot = snapshot();
        snapshot.preset.blocks[0]
            .params
            .insert("treble".into(), 8.0);
        let (proposal, _) = build_proposal(
            &json!({
                "name": "Loaded",
                "blocks": [
                    {"kind": "tube", "source": {"type": "current", "slot": 0},
                     "params": {"gain": 3.0}},
                    {"kind": "cab", "source": {"type": "current", "slot": 1}},
                ],
            }),
            &snapshot,
            &NoLookup,
        )
        .unwrap();
        assert!(proposal.edits_current);
        assert_eq!(proposal.preset.blocks[0].param(3), 8.0);
        assert_eq!(proposal.changes, ["Understory amp Gain: 5.0 to 3.0"]);
    }

    #[test]
    fn a_tone3000_model_waits_for_a_press_with_its_credit_shown() {
        let (proposal, _) = build(json!({
            "name": "Plexi Night",
            "blocks": [{"kind": "amp",
                        "source": {"type": "tone3000", "tone_id": 7, "model_id": 9}}],
        }))
        .unwrap();
        assert_eq!(proposal.downloads(), 1);
        assert_eq!(proposal.credits().next().unwrap().creator, "brucew");
        assert!(proposal.preset.blocks[0].asset.is_none());
    }

    #[test]
    fn a_model_already_downloaded_needs_no_fetching() {
        let mut snapshot = snapshot();
        let asset = AssetRef {
            kind: AssetKind::NamModel,
            sha256: "a".repeat(64),
            name: "plexi.nam".into(),
        };
        snapshot.captures.push(CaptureEntry {
            asset: asset.clone(),
            title: "Plexi".into(),
            kind: amp_preset::CaptureKind::Amp,
            gear: None,
            tone_type: None,
            quality_tiers: 1,
            origin: Some(ToneOrigin {
                model_id: 9,
                ..ToneOrigin::default()
            }),
        });
        let (proposal, _) = build_proposal(
            &json!({
                "name": "Plexi Night",
                "blocks": [{"kind": "amp",
                            "source": {"type": "tone3000", "tone_id": 7, "model_id": 9}}],
            }),
            &snapshot,
            &NoLookup,
        )
        .unwrap();
        assert_eq!(proposal.downloads(), 0);
        assert_eq!(proposal.preset.blocks[0].asset, Some(asset));
    }

    #[test]
    fn the_change_list_notices_moves_bypasses_and_removals() {
        let mut before = vec![
            BlockPreset::new(BlockKind::Drive, true),
            BlockPreset::new(BlockKind::Tube, true),
            BlockPreset::new(BlockKind::Delay, true),
        ];
        let mut after = before.clone();
        after.remove(2);
        after[0].enabled = false;
        before[1].params.insert("mid".into(), 4.0);
        let changes = change_list(&before, &after);
        assert!(changes.contains(&"Remove Delay".to_owned()));
        assert!(changes.contains(&"Bypass Drive".to_owned()));
        assert!(changes.contains(&"Understory amp Mid: 4.0 to 5.0".to_owned()));
    }
}
