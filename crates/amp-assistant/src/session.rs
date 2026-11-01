//! One turn of the conversation: Claude replies, tools run, and the loop goes on
//! until Claude is done. It runs on a background thread and reports through
//! [`Event`]s, so the UI only ever draws what has arrived.

use crate::{
    Client, Error, ModelChoice, Proposal, Taste, Usage,
    prompt::{rig_context, system},
    tools::{self, Outcome, PickerRequest, ToneCatalog},
};
use amp_preset::{CaptureEntry, Preset};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

/// Cheap enough to keep a chat affordable, and long enough for a full chain and
/// a few sentences, thinking included.
const MAX_TOKENS: u32 = 16_000;
const EFFORT: &str = "medium";
/// How many rounds of tools one question can take.
const MAX_ROUNDS: usize = 10;
/// Lets the API route a declined request by what was declined.
const FALLBACK_MODE: &str = "default";

/// The rig as it stood when the player pressed Send. Claude works from this
/// copy, so nothing on the UI thread is ever shared with it.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub preset: Preset,
    pub selected: Option<usize>,
    pub tones: Vec<Preset>,
    pub captures: Vec<CaptureEntry>,
    pub tone3000_signed_in: bool,
}

/// What a running turn reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(String),
    Status(&'static str),
    Suggested(Box<Proposal>),
    Picker(PickerRequest),
    Remembered(String),
    Usage(Usage),
    /// The whole conversation so far, ready for the next message.
    Done(Vec<Value>),
    Failed(Error),
}

/// The API history of one chat. Claude's blocks are kept exactly as they came,
/// because thinking blocks only stay valid if they're sent back unchanged.
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub messages: Vec<Value>,
    /// Where each of the player's messages starts, for Regenerate and Edit.
    starts: Vec<usize>,
}

impl Session {
    /// Carries on from a saved chat, which holds only plain words.
    pub fn resume(messages: Vec<Value>) -> Self {
        let starts = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message["role"] == "user")
            .map(|(index, _)| index)
            .collect();
        Self { messages, starts }
    }

    /// Adds the player's words, with the rig beside them. The rig comes first,
    /// so each message reads as "here's the state, here's the ask".
    pub fn add_user(&mut self, text: &str, snapshot: &Snapshot) {
        self.starts.push(self.messages.len());
        self.messages.push(json!({
            "role": "user",
            "content": [
                {"type": "text", "text": rig_context(snapshot)},
                {"type": "text", "text": text},
            ],
        }));
    }

    /// Goes back to just before the last message the player sent. Cutting the
    /// end off leaves what remains unchanged, so its thinking blocks stay valid.
    pub fn rewind(&mut self) {
        if let Some(start) = self.starts.pop() {
            self.messages.truncate(start);
        }
    }

    /// Keeps what arrived before Stop, so the next message has it for context.
    pub fn keep_partial(&mut self, text: &str) {
        if !text.is_empty() {
            self.messages
                .push(json!({"role": "assistant", "content": text}));
        }
    }

    pub fn take(&mut self, messages: Vec<Value>) {
        self.messages = messages;
    }
}

/// What a turn needs from the app.
pub struct Request {
    pub model: ModelChoice,
    pub taste: Taste,
    pub messages: Vec<Value>,
    pub snapshot: Snapshot,
}

/// Where a turn sends its results and reads TONE3000 from.
pub trait Host {
    fn event(&self, event: Event);
    fn catalog(&self) -> &dyn ToneCatalog;
    fn canceled(&self) -> &AtomicBool;
}

fn request_body(request: &Request, messages: &[Value]) -> Value {
    let mut body = json!({
        "model": request.model.id(),
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "system": system(&request.taste),
        "tools": tools::definitions(),
        "messages": messages,
        "output_config": {"effort": EFFORT},
        "cache_control": {"type": "ephemeral"},
    });
    if request.model.has_fallback() {
        body["fallbacks"] = FALLBACK_MODE.into();
    }
    body
}

fn tool_result(call: &Value, content: String, is_error: bool) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": call["id"],
        "content": content,
        "is_error": is_error,
    })
}

/// Runs the calls in a reply, reports what the player should see, and returns
/// the results for Claude. Results go back together in one message.
fn run_tools(calls: &[Value], snapshot: &Snapshot, host: &dyn Host) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            let name = call["name"].as_str().unwrap_or_default();
            host.event(Event::Status(tools::status_of(name)));
            match tools::run(name, &call["input"], snapshot, host.catalog()) {
                Outcome::Read(text) => tool_result(call, text, false),
                Outcome::Failed(text) => tool_result(call, text, true),
                Outcome::Suggested(proposal, notes) => {
                    host.event(Event::Suggested(proposal));
                    let mut text = "Shown to the player, who can try it with one press.".to_owned();
                    for note in notes {
                        text.push(' ');
                        text.push_str(&note);
                        text.push('.');
                    }
                    tool_result(call, text, false)
                }
                Outcome::Picker(picker) => {
                    host.event(Event::Picker(picker));
                    tool_result(
                        call,
                        "The button is showing. Wait for their pick.".into(),
                        false,
                    )
                }
                Outcome::Remembered(fact) => {
                    host.event(Event::Remembered(fact));
                    tool_result(call, "Remembered.".into(), false)
                }
            }
        })
        .collect()
}

/// Runs one turn to the end, on the caller's thread. Everything comes out
/// through `host`, ending in [`Event::Done`] or [`Event::Failed`].
pub fn run_turn(request: Request, client: &Client, host: &dyn Host) {
    match turn(&request, client, host) {
        Ok(messages) => host.event(Event::Done(messages)),
        Err(error) => host.event(Event::Failed(error)),
    }
}

fn turn(request: &Request, client: &Client, host: &dyn Host) -> Result<Vec<Value>, Error> {
    let mut messages = request.messages.clone();
    for _ in 0..MAX_ROUNDS {
        host.event(Event::Status("Thinking"));
        let body = request_body(request, &messages);
        let reply = client.stream(&body, request.model, host.canceled(), &mut |text| {
            host.event(Event::Text(text.to_owned()));
        })?;
        host.event(Event::Usage(reply.usage));
        match reply.stop_reason.as_str() {
            "refusal" => return Err(Error::Refused),
            // A cut-off reply may hold half a tool call, so nothing in it runs.
            "max_tokens" => return Err(Error::TooLong),
            _ => {}
        }
        let calls: Vec<Value> = reply.tool_calls().cloned().collect();
        messages.push(json!({"role": "assistant", "content": reply.content}));
        if calls.is_empty() {
            return Ok(messages);
        }
        let results = run_tools(&calls, &request.snapshot, host);
        messages.push(json!({"role": "user", "content": results}));
    }
    Err(Error::TooLong)
}

#[cfg(test)]
mod tests {
    use super::*;
    use amp_tone3000::{List, Model, Tone};
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    struct NoCatalog;

    impl ToneCatalog for NoCatalog {
        fn signed_in(&self) -> bool {
            false
        }
        fn list(&self, _: List, _: u32) -> Result<Vec<Tone>, String> {
            Err("signed out".into())
        }
        fn tone_and_models(&self, _: u64) -> Result<(Tone, Vec<Model>), String> {
            Err("signed out".into())
        }
    }

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<Event>>,
        canceled: AtomicBool,
    }

    impl Host for Recorder {
        fn event(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
        fn catalog(&self) -> &dyn ToneCatalog {
            &NoCatalog
        }
        fn canceled(&self) -> &AtomicBool {
            &self.canceled
        }
    }

    fn sse(events: &[Value]) -> String {
        events
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {event}\n\n",
                    event["type"].as_str().unwrap()
                )
            })
            .collect()
    }

    fn text_reply(text: &str) -> String {
        sse(&[
            json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),
            json!({"type":"content_block_start","index":0,
                   "content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,
                   "delta":{"type":"text_delta","text":text}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},
                   "usage":{"output_tokens":2}}),
        ])
    }

    fn tool_reply(name: &str, input: &Value) -> String {
        sse(&[
            json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),
            json!({"type":"content_block_start","index":0,
                   "content_block":{"type":"tool_use","id":"tool-1","name":name,"input":{}}}),
            json!({"type":"content_block_delta","index":0,
                   "delta":{"type":"input_json_delta","partial_json":input.to_string()}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},
                   "usage":{"output_tokens":2}}),
        ])
    }

    /// Plays the API, answering each request with the next reply.
    fn serve(replies: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            for (reply, stream) in replies.into_iter().zip(listener.incoming()) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim().to_lowercase();
                    if header.is_empty() {
                        break;
                    }
                    if let Some(value) = header.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                log.lock()
                    .unwrap()
                    .push(serde_json::from_slice(&body).unwrap());
                let (status, text) = reply;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nretry-after: 0\r\n\
                     Connection: close\r\n\r\n{text}",
                    text.len()
                )
                .unwrap();
            }
        });
        (origin, seen)
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            preset: Preset::starter("Loaded"),
            selected: None,
            tones: vec![Preset::starter("Moss Clean")],
            captures: Vec::new(),
            tone3000_signed_in: false,
        }
    }

    fn request(model: ModelChoice) -> Request {
        let mut session = Session::default();
        session.add_user("Make it warmer", &snapshot());
        Request {
            model,
            taste: Taste::default(),
            messages: session.messages,
            snapshot: snapshot(),
        }
    }

    fn run(origin: String, model: ModelChoice) -> Vec<Event> {
        let client = Client::at(origin, "key");
        let host = Recorder::default();
        run_turn(request(model), &client, &host);
        host.events.into_inner().unwrap()
    }

    #[test]
    fn a_plain_reply_streams_and_finishes() {
        let (origin, seen) = serve(vec![(200, text_reply("Warmer it is."))]);
        let events = run(origin, ModelChoice::Opus);
        assert!(events.contains(&Event::Text("Warmer it is.".into())));
        let Some(Event::Done(messages)) = events.last() else {
            panic!("{events:?}");
        };
        assert_eq!(messages.len(), 2);
        let body = &seen.lock().unwrap()[0];
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["output_config"]["effort"], "medium");
        assert_eq!(body["fallbacks"], "default");
        assert!(body.get("thinking").is_none());
        assert!(body.get("temperature").is_none());
        assert!(
            body["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("<rig>")
        );
    }

    #[test]
    fn haiku_asks_for_no_fallback() {
        let (origin, seen) = serve(vec![(200, text_reply("Hi"))]);
        run(origin, ModelChoice::Haiku);
        assert!(seen.lock().unwrap()[0].get("fallbacks").is_none());
    }

    #[test]
    fn a_suggestion_reaches_the_player_and_claude_hears_back() {
        let input = json!({"name":"Thaw","tags":[],"notes":"","why":"Warmer.",
                           "blocks":[{"kind":"tube"},{"kind":"cab"}]});
        let (origin, seen) = serve(vec![
            (200, tool_reply("suggest_tone", &input)),
            (200, text_reply("Try that.")),
        ]);
        let events = run(origin, ModelChoice::Opus);
        let suggested = events.iter().find_map(|event| match event {
            Event::Suggested(proposal) => Some(proposal),
            _ => None,
        });
        assert_eq!(suggested.unwrap().preset.name, "Thaw");
        let seen = seen.lock().unwrap();
        let result = &seen[1]["messages"][2]["content"][0];
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["tool_use_id"], "tool-1");
        assert_eq!(result["is_error"], false);
    }

    #[test]
    fn a_bad_suggestion_goes_back_to_claude_and_not_to_the_player() {
        let input = json!({"name":"Thaw","blocks":[{"kind":"reverb"}]});
        let (origin, seen) = serve(vec![
            (200, tool_reply("suggest_tone", &input)),
            (200, text_reply("Sorry.")),
        ]);
        let events = run(origin, ModelChoice::Opus);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Suggested(_)))
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen[1]["messages"][2]["content"][0]["is_error"], true);
    }

    #[test]
    fn a_tone3000_list_while_signed_out_explains_itself() {
        let input = json!({"list":"favorites"});
        let (origin, seen) = serve(vec![
            (200, tool_reply("list_tone3000", &input)),
            (200, text_reply("Sign in first.")),
        ]);
        run(origin, ModelChoice::Opus);
        let seen = seen.lock().unwrap();
        let result = &seen[1]["messages"][2]["content"][0];
        assert!(result["content"].as_str().unwrap().contains("signed in"));
    }

    #[test]
    fn being_busy_retries_and_then_succeeds() {
        let (origin, seen) = serve(vec![(529, String::new()), (200, text_reply("Back."))]);
        let events = run(origin, ModelChoice::Opus);
        assert!(matches!(events.last(), Some(Event::Done(_))));
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_bad_key_fails_at_once() {
        let (origin, seen) = serve(vec![(401, String::new())]);
        let events = run(origin, ModelChoice::Opus);
        assert_eq!(events.last(), Some(&Event::Failed(Error::BadKey)));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_refusal_runs_no_tools() {
        let reply = sse(&[
            json!({"type":"message_start","message":{"usage":{}}}),
            json!({"type":"content_block_start","index":0,
                   "content_block":{"type":"tool_use","id":"t","name":"remember","input":{}}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"refusal"},"usage":{}}),
        ]);
        let (origin, _) = serve(vec![(200, reply)]);
        let events = run(origin, ModelChoice::Opus);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Remembered(_)))
        );
        assert_eq!(events.last(), Some(&Event::Failed(Error::Refused)));
    }

    #[test]
    fn stop_ends_the_turn_quietly() {
        let (origin, _) = serve(vec![(200, text_reply("Never seen"))]);
        let client = Client::at(origin, "key");
        let host = Recorder::default();
        host.canceled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        run_turn(request(ModelChoice::Opus), &client, &host);
        let events = host.events.into_inner().unwrap();
        assert_eq!(events.last(), Some(&Event::Failed(Error::Canceled)));
    }

    #[test]
    fn rewinding_cuts_back_to_before_the_last_message() {
        let mut session = Session::default();
        session.add_user("First", &snapshot());
        session.keep_partial("Reply");
        session.add_user("Second", &snapshot());
        session.keep_partial("Another");
        session.rewind();
        assert_eq!(session.messages.len(), 2);
        session.rewind();
        assert!(session.messages.is_empty());
        session.rewind();
    }

    #[test]
    fn a_resumed_chat_can_rewind_too() {
        let mut session = Session::resume(vec![
            json!({"role":"user","content":"One"}),
            json!({"role":"assistant","content":"Two"}),
        ]);
        session.rewind();
        assert!(session.messages.is_empty());
    }
}
