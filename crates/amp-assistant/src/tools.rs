//! The tools Claude can call. Reading is free, and writing never touches the rig:
//! a suggestion becomes a card, and a picker request becomes a button.

use crate::{
    Proposal,
    prompt::kind_name,
    proposal::{ToneLookup, build_proposal, origin_of},
    session::Snapshot,
};
use amp_preset::ToneOrigin;
use amp_tone3000::{Client, List, Model, Tone};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Marks tool input that arrived broken, keeping the raw text.
pub(crate) const UNREADABLE_INPUT: &str = "unreadable_input";
const NOTE_LENGTH: usize = 160;

/// Where a block in a suggestion gets its sound.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToneSource {
    /// Keeps the block already in this slot, with its capture and settings.
    Current { slot: usize },
    /// A capture already in the player's gear rack.
    Capture { sha256: String },
    /// A model on TONE3000, downloaded when the player presses Try.
    Tone3000 { tone_id: u64, model_id: u64 },
}

/// A button to the TONE3000 picker, which Claude can't search for itself.
#[derive(Clone, Debug, PartialEq)]
pub struct PickerRequest {
    /// What to look for, shown beside the button.
    pub looking_for: String,
    pub gear: PickerGear,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickerGear {
    Amp,
    Pedal,
    Ir,
}

/// TONE3000 as the tools need it, so tests can stand in for the site.
pub trait ToneCatalog: Send + Sync {
    fn signed_in(&self) -> bool;
    fn list(&self, list: List, page: u32) -> Result<Vec<Tone>, String>;
    fn tone_and_models(&self, tone_id: u64) -> Result<(Tone, Vec<Model>), String>;
}

impl ToneCatalog for Client {
    fn signed_in(&self) -> bool {
        Client::signed_in(self)
    }

    fn list(&self, list: List, page: u32) -> Result<Vec<Tone>, String> {
        Client::list(self, list, page)
            .map(|page| page.data)
            .map_err(|error| error.to_string())
    }

    fn tone_and_models(&self, tone_id: u64) -> Result<(Tone, Vec<Model>), String> {
        let tone = self.tone(tone_id).map_err(|error| error.to_string())?;
        let models = self.models(&tone).map_err(|error| error.to_string())?;
        Ok((tone, models))
    }
}

/// Finds a model's credit, and that the tone really offers that model.
impl ToneLookup for &dyn ToneCatalog {
    fn origin(&self, tone_id: u64, model_id: u64) -> Result<ToneOrigin, String> {
        let (tone, models) = self.tone_and_models(tone_id)?;
        if !tone.can_load() {
            return Err(format!("this app can't run {} files", tone.format_label()));
        }
        let model = models
            .iter()
            .find(|model| model.id == model_id)
            .ok_or_else(|| format!("tone {tone_id} has no model {model_id}"))?;
        Ok(origin_of(&tone, model))
    }
}

/// The tool list, sent with every request. It never changes, so it caches.
pub(crate) fn definitions() -> Value {
    let tool = |name: &str, description: &str, properties: Value, required: Value| {
        json!({
            "name": name,
            "description": description,
            "eager_input_streaming": true,
            "input_schema": {
                "type": "object",
                "properties": properties,
                "required": required,
            },
        })
    };
    let source = json!({
        "type": "object",
        "description": "Where the block gets its sound. Omit it for built-in blocks.",
        "properties": {
            "type": {"type": "string", "enum": ["current", "capture", "tone3000"]},
            "slot": {"type": "integer", "description": "For current: the rig slot to keep."},
            "sha256": {"type": "string", "description": "For capture: from list_gear."},
            "tone_id": {"type": "integer", "description": "For tone3000."},
            "model_id": {"type": "integer", "description": "For tone3000."},
        },
        "required": ["type"],
    });
    let kinds = [
        "gate",
        "compressor",
        "drive",
        "pedal",
        "tube",
        "amp",
        "cab",
        "chorus",
        "delay",
        "reverb",
    ];
    json!([
        tool(
            "suggest_tone",
            "Show the player a tone to try. Give the whole chain in signal order. They see what \
             changed and can try it with one press, so this is how every tone reaches them.",
            json!({
                "name": {"type": "string", "description": "A short, original name."},
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "One to three lowercase words, such as clean, crunch, or lead.",
                },
                "notes": {"type": "string", "description": "Two sentences on how it plays."},
                "why": {"type": "string", "description": "One sentence on why this fits the ask."},
                "blocks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "enum": kinds},
                            "enabled": {"type": "boolean"},
                            "params": {
                                "type": "object",
                                "description": "Param ids to numbers. Anything left out keeps its \
                                                current or default value.",
                            },
                            "source": source,
                        },
                        "required": ["kind"],
                    },
                },
            }),
            json!(["name", "tags", "notes", "why", "blocks"]),
        ),
        tool(
            "list_tones",
            "List the tones saved in the player's library, optionally only those matching a word \
             in the name, tags, or notes.",
            json!({"query": {"type": "string"}}),
            json!([]),
        ),
        tool(
            "get_tone",
            "Read one saved tone's full chain and settings, to learn from it or build on it.",
            json!({"name": {"type": "string"}}),
            json!(["name"]),
        ),
        tool(
            "list_gear",
            "List the NAM captures in the player's gear rack.",
            json!({}),
            json!([]),
        ),
        tool(
            "list_tone3000",
            "List tones from the player's TONE3000 account or feeds. This doesn't search.",
            json!({
                "list": {
                    "type": "string",
                    "enum": ["favorites", "downloaded", "created", "trending", "latest"],
                },
                "page": {"type": "integer", "description": "From 1. Feeds have one page."},
            }),
            json!(["list"]),
        ),
        tool(
            "tone3000_models",
            "List the models in one TONE3000 tone. Call this before using a tone as a source.",
            json!({"tone_id": {"type": "integer"}}),
            json!(["tone_id"]),
        ),
        tool(
            "find_on_tone3000",
            "Give the player a button that opens the TONE3000 picker, for gear not in any list \
             you can read. Their pick comes back as a message, so end your turn after this.",
            json!({
                "looking_for": {
                    "type": "string",
                    "description": "What to search for there, such as Marshall JCM800.",
                },
                "gear": {"type": "string", "enum": ["amp", "pedal", "ir"]},
            }),
            json!(["looking_for", "gear"]),
        ),
        tool(
            "remember",
            "Keep a lasting fact the player told you about themselves, in one short sentence.",
            json!({"fact": {"type": "string"}}),
            json!(["fact"]),
        ),
    ])
}

/// What a tool call came to.
pub(crate) enum Outcome {
    Read(String),
    Failed(String),
    Suggested(Box<Proposal>, Vec<String>),
    Picker(PickerRequest),
    Remembered(String),
}

fn list_of(name: &str) -> Option<List> {
    Some(match name {
        "favorites" => List::Favorites,
        "downloaded" => List::Downloaded,
        "created" => List::Created,
        "trending" => List::Trending,
        "latest" => List::Latest,
        _ => return None,
    })
}

fn shortened(text: &str) -> String {
    text.chars().take(NOTE_LENGTH).collect()
}

/// A tone as Claude sees it. Only what's useful for choosing, and credit stays
/// attached.
fn tone_summary(tone: &Tone) -> Value {
    let names = |labels: &[amp_tone3000::Label]| {
        labels
            .iter()
            .map(|label| label.name.clone())
            .collect::<Vec<_>>()
    };
    json!({
        "tone_id": tone.id,
        "title": tone.title,
        "gear": tone.gear_label(),
        "format": tone.format_label(),
        "creator": tone.creator().username,
        "makes": names(&tone.makes),
        "tags": names(&tone.tags),
        "about": shortened(tone.description.as_deref().unwrap_or_default()),
    })
}

fn list_tones(input: &Value, snapshot: &Snapshot) -> String {
    let query = input["query"].as_str().unwrap_or_default().to_lowercase();
    let tones: Vec<Value> = snapshot
        .tones
        .iter()
        .filter(|tone| {
            let text = format!("{} {} {}", tone.name, tone.tags.join(" "), tone.notes);
            text.to_lowercase().contains(&query)
        })
        .map(|tone| json!({"name": tone.name, "tags": tone.tags, "notes": tone.notes}))
        .collect();
    Value::Array(tones).to_string()
}

fn get_tone(input: &Value, snapshot: &Snapshot) -> Outcome {
    let name = input["name"].as_str().unwrap_or_default();
    let tone = snapshot
        .tones
        .iter()
        .find(|tone| tone.name.eq_ignore_ascii_case(name));
    match tone {
        Some(tone) => {
            let blocks: Vec<Value> = tone
                .blocks
                .iter()
                .map(|block| {
                    json!({
                        "kind": kind_name(block.kind),
                        "enabled": block.enabled,
                        "params": block.params,
                    })
                })
                .collect();
            Outcome::Read(
                json!({"name": tone.name, "tags": tone.tags, "notes": tone.notes,
                       "blocks": blocks})
                .to_string(),
            )
        }
        None => Outcome::Failed(format!("There's no tone called {name}.")),
    }
}

fn list_gear(snapshot: &Snapshot) -> String {
    let gear: Vec<Value> = snapshot
        .captures
        .iter()
        .map(|capture| {
            json!({
                "sha256": capture.asset.sha256,
                "title": capture.title,
                "kind": capture.kind,
                "gear": capture.gear,
                "creator": capture.origin.as_ref().map(|origin| &origin.creator),
            })
        })
        .collect();
    Value::Array(gear).to_string()
}

fn list_tone3000(input: &Value, catalog: &dyn ToneCatalog) -> Outcome {
    if !catalog.signed_in() {
        return Outcome::Failed(
            "The player isn't signed in to TONE3000. Tell them to press Browse tones to sign in, \
             or use built-in blocks."
                .to_owned(),
        );
    }
    let Some(list) = list_of(input["list"].as_str().unwrap_or_default()) else {
        return Outcome::Failed("Unknown list.".to_owned());
    };
    let page = input["page"].as_u64().unwrap_or(1).max(1) as u32;
    match catalog.list(list, page) {
        Ok(tones) => {
            let loadable: Vec<Value> = tones
                .iter()
                .filter(|tone| tone.can_load())
                .map(tone_summary)
                .collect();
            Outcome::Read(Value::Array(loadable).to_string())
        }
        Err(error) => Outcome::Failed(error),
    }
}

fn tone3000_models(input: &Value, catalog: &dyn ToneCatalog) -> Outcome {
    if !catalog.signed_in() {
        return Outcome::Failed("The player isn't signed in to TONE3000.".to_owned());
    }
    let Some(id) = input["tone_id"].as_u64() else {
        return Outcome::Failed("tone_id has to be a number.".to_owned());
    };
    match catalog.tone_and_models(id) {
        Ok((tone, models)) => {
            let models: Vec<Value> = models
                .iter()
                .map(|model| {
                    json!({"model_id": model.id, "name": model.name,
                                    "detail": model.detail()})
                })
                .collect();
            Outcome::Read(
                json!({"tone": tone_summary(&tone), "license": tone.license_label(),
                       "models": models})
                .to_string(),
            )
        }
        Err(error) => Outcome::Failed(error),
    }
}

fn find_on_tone3000(input: &Value) -> Outcome {
    let gear = match input["gear"].as_str() {
        Some("pedal") => PickerGear::Pedal,
        Some("ir") => PickerGear::Ir,
        _ => PickerGear::Amp,
    };
    Outcome::Picker(PickerRequest {
        looking_for: input["looking_for"].as_str().unwrap_or_default().to_owned(),
        gear,
    })
}

/// Runs one tool call. Broken input goes back to Claude to try again.
pub(crate) fn run(
    name: &str,
    input: &Value,
    snapshot: &Snapshot,
    catalog: &dyn ToneCatalog,
) -> Outcome {
    if input.get(UNREADABLE_INPUT).is_some() {
        return Outcome::Failed(json!({"INVALID_JSON": input[UNREADABLE_INPUT]}).to_string());
    }
    match name {
        "suggest_tone" => match build_proposal(input, snapshot, &catalog) {
            Ok((proposal, notes)) => Outcome::Suggested(Box::new(proposal), notes),
            Err(error) => Outcome::Failed(error),
        },
        "list_tones" => Outcome::Read(list_tones(input, snapshot)),
        "get_tone" => get_tone(input, snapshot),
        "list_gear" => Outcome::Read(list_gear(snapshot)),
        "list_tone3000" => list_tone3000(input, catalog),
        "tone3000_models" => tone3000_models(input, catalog),
        "find_on_tone3000" => find_on_tone3000(input),
        "remember" => match input["fact"].as_str() {
            Some(fact) => Outcome::Remembered(fact.to_owned()),
            None => Outcome::Failed("fact has to be text.".to_owned()),
        },
        other => Outcome::Failed(format!("There's no tool called {other}.")),
    }
}

/// What the status line says while a tool runs.
pub(crate) fn status_of(name: &str) -> &'static str {
    match name {
        "suggest_tone" => "Building the tone",
        "list_tones" | "get_tone" => "Looking through your tones",
        "list_gear" => "Checking your gear",
        "list_tone3000" | "tone3000_models" => "Looking at TONE3000",
        _ => "Working",
    }
}
