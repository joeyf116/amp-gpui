use amp_dsp::BlockKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

/// The most scenes a tone holds, one per number key.
pub const MAX_SCENES: usize = 4;

/// The range of a tone's or scene's Level, in dB.
pub const LEVEL_RANGE: (f32, f32) = (-24.0, 12.0);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Preset {
    pub schema: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    pub blocks: Vec<BlockPreset>,
    /// The tone's volume after the last block, in dB. With scenes, this is the
    /// level of the scene that's playing.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub level_db: f32,
    /// Other settings of the same chain. Empty for a tone with one sound.
    /// `blocks` and `level_db` always hold the scene that's playing, so an app
    /// that doesn't know about scenes still plays the one that was saved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<Scene>,
    /// Which of `scenes` is playing.
    #[serde(default, skip_serializing_if = "is_first")]
    pub scene: usize,
}

/// One setting of a tone's chain: what's on and where every knob sits. Scenes
/// share the chain itself, so switching between them never reloads anything.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Scene {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub level_db: f32,
    /// One entry per block, in chain order.
    pub blocks: Vec<SceneBlock>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SceneBlock {
    pub enabled: bool,
    pub params: BTreeMap<String, f32>,
}

impl From<&BlockPreset> for SceneBlock {
    fn from(block: &BlockPreset) -> Self {
        Self {
            enabled: block.enabled,
            params: block.params.clone(),
        }
    }
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

fn is_first(value: &usize) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BlockPreset {
    pub kind: BlockKind,
    pub enabled: bool,
    /// Keyed by [`ParamSpec::id`](amp_dsp::ParamSpec), so a param added in a
    /// later version falls back to its default on old presets.
    pub params: BTreeMap<String, f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ToneOrigin>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    NamModel,
    ImpulseResponse,
}

impl AssetKind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::NamModel => "nam",
            Self::ImpulseResponse => "wav",
        }
    }

    pub fn for_block(kind: BlockKind) -> Option<Self> {
        match kind {
            BlockKind::Amp | BlockKind::Pedal => Some(Self::NamModel),
            BlockKind::Cab => Some(Self::ImpulseResponse),
            _ => None,
        }
    }
}

/// A file in the library, identified by its contents.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AssetRef {
    pub kind: AssetKind,
    pub sha256: String,
    /// The file's original name, for display only.
    pub name: String,
}

/// Where a downloaded model came from. It keeps the creator's credit with the
/// model, and lets a preset that travels without the file fetch it again.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToneOrigin {
    pub tone_id: u64,
    pub model_id: u64,
    pub title: String,
    pub model: String,
    /// As shown on the site, such as `Amp + Cab`.
    pub gear: String,
    pub format: String,
    pub creator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

impl BlockPreset {
    pub fn new(kind: BlockKind, enabled: bool) -> Self {
        Self {
            kind,
            enabled,
            params: kind
                .params()
                .iter()
                .map(|spec| (spec.id.to_owned(), spec.default))
                .collect(),
            asset: None,
            origin: None,
        }
    }

    /// The stored value of a param, or its default, always within range.
    pub fn param(&self, index: usize) -> f32 {
        let spec = &self.kind.params()[index];
        self.params
            .get(spec.id)
            .copied()
            .unwrap_or(spec.default)
            .clamp(spec.min, spec.max)
    }
}

impl Preset {
    /// A tone with `blocks` and nothing else set.
    pub fn new(name: impl Into<String>, blocks: Vec<BlockPreset>) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            name: name.into(),
            author: String::new(),
            tags: Vec::new(),
            notes: String::new(),
            blocks,
            level_db: 0.0,
            scenes: Vec::new(),
            scene: 0,
        }
    }

    /// A blank signal path: gate, amp, and cab on, effects off.
    pub fn starter(name: impl Into<String>) -> Self {
        Self::new(
            name,
            vec![
                BlockPreset::new(BlockKind::Gate, true),
                BlockPreset::new(BlockKind::Drive, false),
                BlockPreset::new(BlockKind::Amp, true),
                BlockPreset::new(BlockKind::Cab, true),
                BlockPreset::new(BlockKind::Delay, false),
            ],
        )
    }

    /// The tone's Level, always within range.
    pub fn level(&self) -> f32 {
        self.level_db.clamp(LEVEL_RANGE.0, LEVEL_RANGE.1)
    }

    /// The scene that's playing, as it stands now.
    fn current_scene(&self) -> Scene {
        Scene {
            name: self
                .scenes
                .get(self.scene)
                .map(|scene| scene.name.clone())
                .unwrap_or_default(),
            level_db: self.level_db,
            blocks: self.blocks.iter().map(SceneBlock::from).collect(),
        }
    }

    /// Writes what's playing back into its scene, so the scene list is whole.
    pub fn store_scene(&mut self) {
        if self.scene < self.scenes.len() {
            self.scenes[self.scene] = self.current_scene();
        }
    }

    /// Moves to scene `index`. Only switches and knobs change, never the chain,
    /// so the engine can follow with param and bypass changes alone. Returns
    /// whether anything changed.
    pub fn switch_scene(&mut self, index: usize) -> bool {
        if index >= self.scenes.len() || index == self.scene {
            return false;
        }
        self.store_scene();
        self.apply_scene(index);
        true
    }

    fn apply_scene(&mut self, index: usize) {
        self.scene = index;
        let scene = &self.scenes[index];
        self.level_db = scene.level_db;
        for (block, stored) in self.blocks.iter_mut().zip(&scene.blocks) {
            block.enabled = stored.enabled;
            block.params.clone_from(&stored.params);
        }
    }

    /// Adds a scene that starts as a copy of the one playing, and moves to it.
    /// A tone with one sound becomes scenes 1 and 2. Returns the new scene, or
    /// `None` when the tone is full.
    pub fn add_scene(&mut self) -> Option<usize> {
        if self.scenes.len() >= MAX_SCENES {
            return None;
        }
        if self.scenes.is_empty() {
            self.scenes.push(self.current_scene());
            self.scene = 0;
        }
        self.store_scene();
        let mut copy = self.current_scene();
        copy.name.clear();
        self.scenes.push(copy);
        self.scene = self.scenes.len() - 1;
        Some(self.scene)
    }

    /// Removes the scene that's playing and moves to the one after it, or the
    /// last. Removing the second to last scene leaves a tone with one sound.
    pub fn remove_scene(&mut self) {
        if self.scene >= self.scenes.len() {
            return;
        }
        self.scenes.remove(self.scene);
        if let Some(last) = self.scenes.len().checked_sub(1) {
            self.apply_scene(self.scene.min(last));
        }
        if self.scenes.len() <= 1 {
            self.scenes.clear();
            self.scene = 0;
        }
    }

    /// Adds a block to the chain. Every scene gets it as it's set now.
    pub fn insert_block(&mut self, slot: usize, block: BlockPreset) {
        let slot = slot.min(self.blocks.len());
        for scene in &mut self.scenes {
            scene.blocks.insert(slot, SceneBlock::from(&block));
        }
        self.blocks.insert(slot, block);
    }

    /// Puts `block` in place of the one at `slot`. Scenes keep their settings
    /// for a block of the same kind, and take the new block's for another.
    pub fn replace_block(&mut self, slot: usize, block: BlockPreset) {
        let Some(old) = self.blocks.get(slot) else {
            return;
        };
        if old.kind != block.kind {
            for scene in &mut self.scenes {
                if let Some(stored) = scene.blocks.get_mut(slot) {
                    *stored = SceneBlock::from(&block);
                }
            }
        }
        self.blocks[slot] = block;
    }

    /// Takes a block out of the chain and out of every scene.
    pub fn remove_block(&mut self, slot: usize) -> Option<BlockPreset> {
        if slot >= self.blocks.len() {
            return None;
        }
        for scene in &mut self.scenes {
            scene.blocks.remove(slot);
        }
        Some(self.blocks.remove(slot))
    }

    /// Moves a block along the chain, taking its scene settings with it.
    pub fn move_block(&mut self, from: usize, to: usize) {
        if from >= self.blocks.len() || to >= self.blocks.len() {
            return;
        }
        let block = self.blocks.remove(from);
        self.blocks.insert(to, block);
        for scene in &mut self.scenes {
            let stored = scene.blocks.remove(from);
            scene.blocks.insert(to, stored);
        }
    }

    /// Puts the chain in a new order, where `order[new] = old`.
    pub fn reorder(&mut self, order: &[usize]) {
        if order.len() != self.blocks.len() {
            return;
        }
        self.blocks = order.iter().map(|old| self.blocks[*old].clone()).collect();
        for scene in &mut self.scenes {
            scene.blocks = order.iter().map(|old| scene.blocks[*old].clone()).collect();
        }
    }

    /// Makes the scenes agree with the chain. A file from someone else, or a
    /// chain swapped wholesale, can leave a scene describing blocks that
    /// aren't there, and those scenes start again from what's playing. Names
    /// and levels are kept.
    pub fn align_scenes(&mut self) {
        self.scenes.truncate(MAX_SCENES);
        if self.scenes.len() <= 1 {
            self.scenes.clear();
            self.scene = 0;
            return;
        }
        self.scene = self.scene.min(self.scenes.len() - 1);
        let count = self.blocks.len();
        let current: Vec<SceneBlock> = self.blocks.iter().map(SceneBlock::from).collect();
        for scene in &mut self.scenes {
            if scene.blocks.len() != count {
                scene.blocks.clone_from(&current);
            }
        }
        self.store_scene();
    }

    /// Carries this tone's scenes over to `blocks`, a new chain for the same
    /// tone, such as an edit the assistant suggested. Where the chain still
    /// lines up kind by kind, the other scenes keep their settings, so only the
    /// scene that's playing takes the edit. Otherwise every scene starts again
    /// from the new chain.
    pub fn with_chain(&self, blocks: Vec<BlockPreset>, level_db: f32) -> Self {
        let mut tone = self.clone();
        tone.store_scene();
        let same_chain = tone.blocks.len() == blocks.len()
            && tone
                .blocks
                .iter()
                .zip(&blocks)
                .all(|(old, new)| old.kind == new.kind);
        tone.blocks = blocks;
        tone.level_db = level_db;
        if !same_chain {
            let current: Vec<SceneBlock> = tone.blocks.iter().map(SceneBlock::from).collect();
            for scene in &mut tone.scenes {
                scene.blocks.clone_from(&current);
                scene.level_db = level_db;
            }
        }
        tone.store_scene();
        tone
    }

    /// Whether two tones would save the same sound, whichever scene each is
    /// showing. Moving between scenes isn't an edit.
    pub fn sounds_like(&self, other: &Self) -> bool {
        self.settled() == other.settled()
    }

    /// The tone with its scenes stored and the first one playing, which is
    /// the same for every scene it could be showing.
    fn settled(&self) -> (Vec<BlockKind>, Vec<Option<&AssetRef>>, Vec<Scene>) {
        let mut scenes = if self.scenes.is_empty() {
            vec![self.current_scene()]
        } else {
            let mut scenes = self.scenes.clone();
            if let Some(scene) = scenes.get_mut(self.scene) {
                *scene = self.current_scene();
            }
            scenes
        };
        for scene in &mut scenes {
            scene.level_db = scene.level_db.clamp(LEVEL_RANGE.0, LEVEL_RANGE.1);
        }
        (
            self.blocks.iter().map(|block| block.kind).collect(),
            self.blocks
                .iter()
                .map(|block| block.asset.as_ref())
                .collect(),
            scenes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let preset = Preset::starter("Clean");
        let json = serde_json::to_string(&preset).unwrap();
        assert_eq!(serde_json::from_str::<Preset>(&json).unwrap(), preset);
    }

    #[test]
    fn missing_and_out_of_range_params_stay_valid() {
        let mut block = BlockPreset::new(BlockKind::Delay, true);
        block.params.remove("time");
        block.params.insert("mix".into(), 900.0);
        assert_eq!(block.param(0), 380.0);
        assert_eq!(block.param(2), 100.0);
    }

    #[test]
    fn presets_from_before_tone_origins_still_load() {
        let block = BlockPreset::new(BlockKind::Amp, true);
        let json = serde_json::to_string(&block).unwrap();
        assert!(!json.contains("origin"));
        assert_eq!(serde_json::from_str::<BlockPreset>(&json).unwrap(), block);
    }

    fn delay_mix(preset: &Preset) -> f32 {
        let slot = preset
            .blocks
            .iter()
            .position(|block| block.kind == BlockKind::Delay)
            .unwrap();
        preset.blocks[slot].param(2)
    }

    fn set_delay(preset: &mut Preset, on: bool, mix: f32) {
        let delay = preset
            .blocks
            .iter_mut()
            .find(|block| block.kind == BlockKind::Delay)
            .unwrap();
        delay.enabled = on;
        delay.params.insert("mix".into(), mix);
    }

    #[test]
    fn a_new_scene_copies_the_one_playing_and_each_keeps_its_own_settings() {
        let mut tone = Preset::starter("Song");
        set_delay(&mut tone, false, 20.0);
        tone.level_db = -2.0;
        assert_eq!(tone.add_scene(), Some(1));
        assert_eq!(tone.scenes.len(), 2);
        set_delay(&mut tone, true, 35.0);
        tone.level_db = 3.0;

        assert!(tone.switch_scene(0));
        assert_eq!(delay_mix(&tone), 20.0);
        assert_eq!(tone.level_db, -2.0);
        assert!(!tone.blocks[4].enabled);

        assert!(tone.switch_scene(1));
        assert_eq!(delay_mix(&tone), 35.0);
        assert_eq!(tone.level_db, 3.0);
        assert!(tone.blocks[4].enabled);
        assert!(!tone.switch_scene(1));
        assert!(!tone.switch_scene(7));
    }

    #[test]
    fn a_tone_holds_four_scenes_at_most() {
        let mut tone = Preset::starter("Full");
        for _ in 0..3 {
            assert!(tone.add_scene().is_some());
        }
        assert_eq!(tone.scenes.len(), MAX_SCENES);
        assert_eq!(tone.add_scene(), None);
    }

    #[test]
    fn removing_down_to_one_scene_leaves_a_plain_tone_on_the_last_sound() {
        let mut tone = Preset::starter("Song");
        tone.add_scene();
        set_delay(&mut tone, true, 50.0);
        tone.switch_scene(0);
        tone.remove_scene();
        assert!(tone.scenes.is_empty());
        assert_eq!(tone.scene, 0);
        assert_eq!(delay_mix(&tone), 50.0);
        assert!(tone.blocks[4].enabled);
    }

    #[test]
    fn chain_edits_reach_every_scene() {
        let mut tone = Preset::starter("Song");
        tone.add_scene();
        tone.insert_block(0, BlockPreset::new(BlockKind::Compressor, true));
        tone.move_block(5, 1);
        tone.remove_block(3);
        tone.reorder(&[1, 0, 2, 3, 4]);
        let kinds: Vec<_> = tone.blocks.iter().map(|block| block.kind).collect();
        for scene in &tone.scenes {
            assert_eq!(scene.blocks.len(), tone.blocks.len());
        }
        tone.switch_scene(0);
        assert_eq!(
            tone.blocks
                .iter()
                .map(|block| block.kind)
                .collect::<Vec<_>>(),
            kinds
        );
        // The delay moved with its settings: it's the only block with a mix.
        let delay = tone
            .blocks
            .iter()
            .position(|block| block.kind == BlockKind::Delay)
            .unwrap();
        assert!(tone.scenes[0].blocks[delay].params.contains_key("mix"));
    }

    #[test]
    fn a_block_of_another_kind_takes_its_own_settings_into_every_scene() {
        let mut tone = Preset::starter("Song");
        tone.add_scene();
        tone.replace_block(1, BlockPreset::new(BlockKind::Chorus, true));
        tone.switch_scene(0);
        assert_eq!(tone.blocks[1].kind, BlockKind::Chorus);
        assert!(tone.blocks[1].params.contains_key("rate"));
        assert!(!tone.blocks[1].params.contains_key("drive"));
    }

    #[test]
    fn scenes_that_no_longer_fit_the_chain_start_again_from_it() {
        let mut tone = Preset::starter("Shared");
        tone.add_scene();
        tone.scenes[0].blocks.pop();
        tone.scenes[0].name = "Verse".into();
        tone.align_scenes();
        assert_eq!(tone.scenes[0].blocks.len(), tone.blocks.len());
        assert_eq!(tone.scenes[0].name, "Verse");

        let mut lonely = Preset::starter("One");
        lonely.scenes.push(Scene::default());
        lonely.scene = 3;
        lonely.align_scenes();
        assert!(lonely.scenes.is_empty());
        assert_eq!(lonely.scene, 0);
    }

    #[test]
    fn an_edit_to_the_same_chain_leaves_the_other_scenes_alone() {
        let mut tone = Preset::starter("Song");
        tone.add_scene();
        set_delay(&mut tone, true, 40.0);
        let mut edited = tone.blocks.clone();
        edited[4].params.insert("mix".into(), 10.0);
        let mut kept = tone.with_chain(edited, 1.0);
        assert_eq!(delay_mix(&kept), 10.0);
        kept.switch_scene(0);
        assert_eq!(delay_mix(&kept), 20.0);

        let rebuilt = tone.with_chain(vec![BlockPreset::new(BlockKind::Tube, true)], 0.0);
        assert!(rebuilt.scenes.iter().all(|scene| scene.blocks.len() == 1));
    }

    #[test]
    fn moving_between_scenes_is_not_an_edit_but_turning_a_knob_is() {
        let mut saved = Preset::starter("Song");
        saved.add_scene();
        set_delay(&mut saved, true, 45.0);
        saved.store_scene();
        let mut playing = saved.clone();
        playing.switch_scene(0);
        assert!(playing.sounds_like(&saved));
        playing.level_db = 2.0;
        assert!(!playing.sounds_like(&saved));
    }

    #[test]
    fn a_plain_tone_saves_without_scene_fields() {
        let json = serde_json::to_string(&Preset::starter("Plain")).unwrap();
        assert!(!json.contains("scene"));
        assert!(!json.contains("level_db"));
        let mut tone = Preset::starter("Scenes");
        tone.add_scene();
        tone.level_db = -3.0;
        let json = serde_json::to_string(&tone).unwrap();
        assert_eq!(serde_json::from_str::<Preset>(&json).unwrap(), tone);
    }
}
