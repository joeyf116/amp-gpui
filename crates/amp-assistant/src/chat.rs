//! Saved chats. Only what was said and suggested is kept. TONE3000's terms bar
//! caching its catalog, so the lists Claude read aren't written down, and a
//! reopened chat carries on from the words alone.

use crate::{Proposal, prompt::kind_name};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const TITLE_LENGTH: usize = 48;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// One message in a chat, as the player sees it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Turn {
    pub role: Role,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposals: Vec<Proposal>,
    /// Things Claude remembered during this turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remembered: Vec<String>,
}

impl Turn {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
            proposals: Vec::new(),
            remembered: Vec::new(),
        }
    }

    pub fn assistant() -> Self {
        Self {
            role: Role::Assistant,
            ..Self::user("")
        }
    }

    /// The words of this turn, with any suggestions spelled out so a reopened
    /// chat still knows what they were.
    fn replay_text(&self) -> String {
        let mut text = self.text.clone();
        for proposal in &self.proposals {
            let blocks: Vec<String> = proposal
                .preset
                .blocks
                .iter()
                .map(|block| kind_name(block.kind))
                .collect();
            text.push_str(&format!(
                "\n\n(I suggested a tone called {}, with these blocks: {}.)",
                proposal.preset.name,
                blocks.join(", ")
            ));
        }
        text
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Chat {
    pub id: u64,
    pub title: String,
    pub turns: Vec<Turn>,
}

impl Chat {
    pub fn new() -> Self {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        Self {
            id,
            title: String::new(),
            turns: Vec::new(),
        }
    }

    /// Takes a title from the first thing the player said.
    pub fn name_from(&mut self, text: &str) {
        if self.title.is_empty() {
            let line = text.lines().next().unwrap_or_default().trim();
            self.title = match line.char_indices().nth(TITLE_LENGTH) {
                Some((end, _)) => format!("{}...", line[..end].trim_end()),
                None => line.to_owned(),
            };
        }
    }

    /// The whole chat as plain API messages.
    pub fn replay(&self) -> Vec<Value> {
        self.turns
            .iter()
            .map(|turn| {
                let role = match turn.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                json!({"role": role, "content": turn.replay_text()})
            })
            .filter(|message| message["content"] != "")
            .collect()
    }
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

/// A saved chat in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatSummary {
    pub id: u64,
    pub title: String,
}

/// The folder of saved chats.
#[derive(Clone, Debug)]
pub struct ChatStore {
    folder: PathBuf,
}

impl ChatStore {
    pub fn new(folder: &Path) -> Self {
        Self {
            folder: folder.join("chats"),
        }
    }

    fn path(&self, id: u64) -> PathBuf {
        self.folder.join(format!("{id}.json"))
    }

    pub fn save(&self, chat: &Chat) -> io::Result<()> {
        fs::create_dir_all(&self.folder)?;
        let path = self.path(chat.id);
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_string(chat)?)?;
        fs::rename(temporary, path)
    }

    pub fn load(&self, id: u64) -> Option<Chat> {
        let text = fs::read_to_string(self.path(id)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn delete(&self, id: u64) -> io::Result<()> {
        match fs::remove_file(self.path(id)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Saved chats, newest first. A damaged file is skipped.
    pub fn list(&self) -> Vec<ChatSummary> {
        let Ok(files) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut chats: Vec<ChatSummary> = files
            .flatten()
            .filter_map(|file| {
                let id = file.path().file_stem()?.to_str()?.parse().ok()?;
                let chat = self.load(id)?;
                Some(ChatSummary {
                    id,
                    title: chat.title,
                })
            })
            .collect();
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.id));
        chats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (ChatStore, PathBuf) {
        let folder = std::env::temp_dir().join(format!("amp-chat-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        (ChatStore::new(&folder), folder)
    }

    #[test]
    fn titles_come_from_the_first_line_and_stay_short() {
        let mut chat = Chat::new();
        chat.name_from("Warm blues lead\nwith a long second line");
        assert_eq!(chat.title, "Warm blues lead");
        chat.name_from("Something later");
        assert_eq!(chat.title, "Warm blues lead");

        let mut long = Chat::new();
        long.name_from(&"é".repeat(80));
        assert_eq!(long.title.chars().count(), TITLE_LENGTH + 3);
    }

    #[test]
    fn chats_save_list_newest_first_and_delete() {
        let (store, folder) = store("list");
        for id in [10, 30, 20] {
            let mut chat = Chat::new();
            chat.id = id;
            chat.title = format!("Chat {id}");
            store.save(&chat).unwrap();
        }
        fs::write(folder.join("chats").join("99.json"), "{").unwrap();
        let ids: Vec<u64> = store.list().iter().map(|chat| chat.id).collect();
        assert_eq!(ids, [30, 20, 10]);
        store.delete(20).unwrap();
        store.delete(20).unwrap();
        assert_eq!(store.list().len(), 2);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_reopened_chat_replays_as_plain_words() {
        let mut chat = Chat::new();
        chat.turns.push(Turn::user("More mids"));
        chat.turns.push(Turn {
            text: "Done.".into(),
            ..Turn::assistant()
        });
        chat.turns.push(Turn::assistant());
        let messages = chat.replay();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["content"], "Done.");
    }
}
