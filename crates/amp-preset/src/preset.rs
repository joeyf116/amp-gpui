use amp_dsp::BlockKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

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
    /// A blank signal path: gate, amp, and cab on, effects off.
    pub fn starter(name: impl Into<String>) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            name: name.into(),
            author: String::new(),
            tags: Vec::new(),
            notes: String::new(),
            blocks: vec![
                BlockPreset::new(BlockKind::Gate, true),
                BlockPreset::new(BlockKind::Drive, false),
                BlockPreset::new(BlockKind::Amp, true),
                BlockPreset::new(BlockKind::Cab, true),
                BlockPreset::new(BlockKind::Delay, false),
            ],
        }
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
}
