//! A tone assistant built on the Claude API, with no UI of its own.
//!
//! Claude works in a loop on a background thread, reading a snapshot of the rig
//! and handing back suggestions as [`Event`]s. It changes nothing by itself: a
//! suggestion is a [`Proposal`] that the player tries with a press. That press is
//! also what downloads any TONE3000 model it needs, because their terms allow
//! downloads only when a user asks for one.
//!
//! The person's own API key pays for this. It's kept sealed with `amp-secret` and
//! goes only to Anthropic. Third-party apps can't offer a Claude account sign-in.

mod api;
mod chat;
mod prompt;
mod proposal;
mod session;
mod taste;
mod tools;

pub use api::{Client, Error, Reply, Usage};
pub use chat::{Chat, ChatStore, ChatSummary, Role, Turn};
pub use prompt::STARTERS;
pub use proposal::{Credit, Proposal, change_list, needs_download, origin_of, tone_block_kind};
pub use session::{Event, Host, Request, Session, Snapshot, run_turn};
pub use taste::{Fact, Taste};
pub use tools::{PickerGear, PickerRequest, ToneCatalog, ToneSource};

use amp_secret::SealedFile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const KEY_FILE: &str = "anthropic.key";
const FOLDER: &str = "assistant";

/// Where the assistant keeps chats and taste, inside the app's data folder.
pub fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join(FOLDER)
}

/// The sealed file holding the player's API key.
pub fn key_file(data_dir: &Path) -> SealedFile {
    SealedFile::new(data_dir.join(KEY_FILE))
}

/// Which Claude model answers. Prices are per million tokens, for the running
/// cost shown in the panel.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ModelChoice {
    #[default]
    #[serde(rename = "claude-opus-5-5")]
    Opus,
    #[serde(rename = "claude-sonnet-5-5")]
    Sonnet,
    #[serde(rename = "claude-haiku-5-5")]
    Haiku,
}

impl ModelChoice {
    pub const ALL: [Self; 3] = [Self::Opus, Self::Sonnet, Self::Haiku];

    pub fn id(self) -> &'static str {
        match self {
            Self::Opus => "claude-opus-5-5",
            Self::Sonnet => "claude-sonnet-5-5",
            Self::Haiku => "claude-haiku-5-5",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Opus => "Claude Opus 5.5",
            Self::Sonnet => "Claude Sonnet 5.5",
            Self::Haiku => "Claude Haiku 5.5",
        }
    }

    /// Only these can hand a declined request to another model.
    fn has_fallback(self) -> bool {
        !matches!(self, Self::Haiku)
    }

    /// Input and output prices in dollars.
    fn prices(self) -> (f64, f64) {
        match self {
            Self::Opus => (4.0, 20.0),
            Self::Sonnet => (2.0, 10.0),
            Self::Haiku => (0.10, 0.50),
        }
    }

    /// Roughly what `usage` costs, in dollars. Cache writes run a quarter over
    /// the input price and reads about a tenth of it.
    pub fn cost(self, usage: Usage) -> f64 {
        let (input, output) = self.prices();
        let tokens = input * usage.input_tokens as f64
            + input * 1.25 * usage.cache_creation_input_tokens as f64
            + input * 0.1 * usage.cache_read_input_tokens as f64
            + output * usage.output_tokens as f64;
        tokens / 1_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_costs_what_its_tokens_say() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..Usage::default()
        };
        assert!((ModelChoice::Opus.cost(usage) - 24.0).abs() < 1e-9);
        assert!((ModelChoice::Haiku.cost(usage) - 0.6).abs() < 1e-9);
    }

    #[test]
    fn model_choices_save_as_their_api_ids() {
        for model in ModelChoice::ALL {
            let json = serde_json::to_string(&model).unwrap();
            assert_eq!(json, format!("\"{}\"", model.id()));
        }
    }
}
