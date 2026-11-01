//! The Claude Messages API, streamed over HTTP. Rust has no official SDK, so
//! this speaks the wire format directly, and every call blocks.

use crate::ModelChoice;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};
use ureq::{Agent, Body, http::Response};
use url::Url;

const ORIGIN: &str = "https://api.anthropic.com";
/// `AMP_GPUI_ANTHROPIC` points the client at a test server. Only loopback
/// addresses count, so a stray variable can't send the key anywhere.
const ORIGIN_OVERRIDE: &str = "AMP_GPUI_ANTHROPIC";
const API_VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const RETRIES: u32 = 3;
const RETRY_STEP: Duration = Duration::from_millis(250);

/// What can go wrong, in words for the player. The key never appears in these.
#[derive(Clone, Debug, thiserror::Error, PartialEq)]
pub enum Error {
    #[error("Anthropic didn't accept that API key. Check it in the assistant's settings")]
    BadKey,
    #[error("This API key is out of credit. Add some in the Claude Console")]
    NoCredit,
    #[error("Claude is getting too many requests from this key. Try again in a minute")]
    RateLimited,
    #[error("Claude is busy right now. Try again in a moment")]
    Overloaded,
    #[error("Claude declined to answer that. Try asking in a different way")]
    Refused,
    #[error("That reply ran too long and was cut off. Try asking for less at once")]
    TooLong,
    #[error("Couldn't reach Claude. Check your connection and try again")]
    Network(String),
    #[error("Claude answered with an error ({0}): {1}")]
    Api(u16, String),
    #[error("Stopped")]
    Canceled,
}

impl From<ureq::Error> for Error {
    fn from(error: ureq::Error) -> Self {
        Self::Network(error.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Network(error.to_string())
    }
}

/// Tokens a reply used, which is what Anthropic bills.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
    }
}

/// One finished reply from Claude.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    /// The content blocks as they arrived, to send back unchanged.
    pub content: Vec<Value>,
    pub stop_reason: String,
    pub usage: Usage,
}

impl Reply {
    pub fn tool_calls(&self) -> impl Iterator<Item = &Value> {
        self.content
            .iter()
            .filter(|block| block["type"] == "tool_use")
    }
}

/// Builds a [`Reply`] from stream events.
#[derive(Default)]
struct Assembler {
    reply: Reply,
    /// The tool input text arriving for each content block.
    inputs: Vec<String>,
}

impl Assembler {
    fn block(&mut self, index: usize) -> Option<&mut Value> {
        self.reply.content.get_mut(index)
    }

    fn apply(&mut self, event: &Value, on_text: &mut dyn FnMut(&str)) -> Result<(), Error> {
        let index = event["index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                self.reply.usage = usage_of(&event["message"]["usage"]);
            }
            "content_block_start" => {
                self.reply.content.resize(index + 1, Value::Null);
                self.inputs.resize(index + 1, String::new());
                self.reply.content[index] = event["content_block"].clone();
            }
            "content_block_delta" => self.delta(index, &event["delta"], on_text),
            "content_block_stop" => self.finish_block(index),
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    reason.clone_into(&mut self.reply.stop_reason);
                }
                let usage = usage_of(&event["usage"]);
                self.reply.usage.output_tokens = usage.output_tokens;
            }
            "error" => return Err(stream_error(&event["error"])),
            _ => {}
        }
        Ok(())
    }

    fn delta(&mut self, index: usize, delta: &Value, on_text: &mut dyn FnMut(&str)) {
        let text = |key: &str| delta[key].as_str().unwrap_or_default();
        match delta["type"].as_str().unwrap_or_default() {
            "text_delta" => {
                on_text(text("text"));
                append(self.block(index), "text", text("text"));
            }
            "thinking_delta" => append(self.block(index), "thinking", text("thinking")),
            "signature_delta" => append(self.block(index), "signature", text("signature")),
            "input_json_delta" => {
                if let Some(input) = self.inputs.get_mut(index) {
                    input.push_str(text("partial_json"));
                }
            }
            _ => {}
        }
    }

    /// Tool input is parsed strictly once it's whole. If it can't be, the raw
    /// text stays on the block, so the tool can tell Claude what went wrong.
    fn finish_block(&mut self, index: usize) {
        let Some(raw) = self.inputs.get(index).filter(|raw| !raw.is_empty()) else {
            return;
        };
        let parsed = serde_json::from_str(raw);
        let raw = raw.clone();
        if let Some(block) = self.block(index) {
            block["input"] = match parsed {
                Ok(input) => input,
                Err(_) => json!({ crate::tools::UNREADABLE_INPUT: raw }),
            };
        }
    }
}

fn append(block: Option<&mut Value>, key: &str, text: &str) {
    if let Some(block) = block {
        let joined = format!("{}{text}", block[key].as_str().unwrap_or_default());
        block[key] = joined.into();
    }
}

fn usage_of(value: &Value) -> Usage {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

fn stream_error(error: &Value) -> Error {
    match error["type"].as_str().unwrap_or_default() {
        "overloaded_error" => Error::Overloaded,
        "rate_limit_error" => Error::RateLimited,
        "authentication_error" => Error::BadKey,
        kind => Error::Api(0, error["message"].as_str().unwrap_or(kind).to_owned()),
    }
}

/// Talks to Claude with one person's API key. The key only ever goes to
/// Anthropic.
pub struct Client {
    origin: String,
    key: String,
    agent: Agent,
}

fn agent() -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(120)))
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .build()
        .into()
}

fn origin() -> String {
    std::env::var(ORIGIN_OVERRIDE)
        .ok()
        .filter(|origin| {
            Url::parse(origin).is_ok_and(|url| {
                matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            })
        })
        .unwrap_or_else(|| ORIGIN.to_owned())
}

impl Client {
    pub fn new(key: &str) -> Self {
        Self {
            origin: origin(),
            key: key.to_owned(),
            agent: agent(),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(origin: String, key: &str) -> Self {
        Self {
            origin,
            key: key.to_owned(),
            agent: agent(),
        }
    }

    /// Checks the key with a free request.
    pub fn check_key(&self) -> Result<(), Error> {
        let url = format!("{}/v1/models?limit=1", self.origin);
        let response = self
            .agent
            .get(&url)
            .header("x-api-key", &self.key)
            .header("anthropic-version", API_VERSION)
            .call()?;
        checked(response).map(|_| ())
    }

    fn post(&self, body: &Value, model: ModelChoice) -> Result<Response<Body>, Error> {
        let url = format!("{}/v1/messages", self.origin);
        let mut request = self
            .agent
            .post(&url)
            .header("x-api-key", &self.key)
            .header("anthropic-version", API_VERSION);
        if model.has_fallback() {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }
        Ok(request.send_json(body)?)
    }

    /// Sends `body` and streams the reply, calling `on_text` for each piece of
    /// text. Busy and rate-limited answers are retried, since nothing has been
    /// shown yet at that point.
    pub fn stream(
        &self,
        body: &Value,
        model: ModelChoice,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Reply, Error> {
        let mut attempt = 0;
        let response = loop {
            let response = self.post(body, model)?;
            let wait = retry_after(&response);
            match checked(response) {
                Err(Error::RateLimited | Error::Overloaded) if attempt < RETRIES => {
                    attempt += 1;
                    pause(wait.unwrap_or(Duration::from_secs(1 << attempt)), cancel)?;
                }
                other => break other?,
            }
        };
        read_stream(response, cancel, on_text)
    }
}

fn retry_after(response: &Response<Body>) -> Option<Duration> {
    let seconds = response.headers().get("retry-after")?.to_str().ok()?;
    seconds.parse().ok().map(Duration::from_secs)
}

/// Waits, but gives up the moment Stop is pressed.
fn pause(length: Duration, cancel: &AtomicBool) -> Result<(), Error> {
    let mut left = length;
    while !left.is_zero() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Canceled);
        }
        thread::sleep(RETRY_STEP.min(left));
        left = left.saturating_sub(RETRY_STEP);
    }
    Ok(())
}

fn checked(mut response: Response<Body>) -> Result<Response<Body>, Error> {
    match response.status().as_u16() {
        200..=299 => Ok(response),
        401 | 403 => Err(Error::BadKey),
        402 => Err(Error::NoCredit),
        429 => Err(Error::RateLimited),
        500 | 502 | 503 | 504 | 529 => Err(Error::Overloaded),
        status => {
            let text = response.body_mut().read_to_string().unwrap_or_default();
            let error: Value = serde_json::from_str(&text).unwrap_or_default();
            let message = error["error"]["message"].as_str().unwrap_or("no details");
            if message.to_lowercase().contains("credit balance") {
                return Err(Error::NoCredit);
            }
            Err(Error::Api(status, message.to_owned()))
        }
    }
}

fn read_stream(
    mut response: Response<Body>,
    cancel: &AtomicBool,
    on_text: &mut dyn FnMut(&str),
) -> Result<Reply, Error> {
    let mut assembler = Assembler::default();
    let body = response.body_mut().as_reader();
    for line in BufReader::new(body).lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Canceled);
        }
        let line = line?;
        if let Some(data) = line.strip_prefix("data:")
            && let Ok(event) = serde_json::from_str::<Value>(data.trim_start())
        {
            assembler.apply(&event, on_text)?;
        }
    }
    if assembler.reply.stop_reason.is_empty() {
        return Err(Error::Network("the reply ended early".to_owned()));
    }
    Ok(assembler.reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assemble(events: &[&str]) -> (Result<Reply, Error>, String) {
        let mut assembler = Assembler::default();
        let mut text = String::new();
        for event in events {
            let event: Value = serde_json::from_str(event).unwrap();
            if let Err(error) = assembler.apply(&event, &mut |piece| text.push_str(piece)) {
                return (Err(error), text);
            }
        }
        (Ok(assembler.reply), text)
    }

    #[test]
    fn text_streams_and_the_reply_keeps_every_block_whole() {
        let (reply, text) = assemble(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":9,"cache_read_input_tokens":4}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hel"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"lo"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
        ]);
        let reply = reply.unwrap();
        assert_eq!(text, "Hello");
        assert_eq!(reply.content[0]["signature"], "abc");
        assert_eq!(reply.content[1]["text"], "Hello");
        assert_eq!(reply.stop_reason, "end_turn");
        assert_eq!(reply.usage.input_tokens, 9);
        assert_eq!(reply.usage.cache_read_input_tokens, 4);
        assert_eq!(reply.usage.output_tokens, 3);
    }

    #[test]
    fn tool_input_is_put_back_together() {
        let (reply, _) = assemble(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"get_tone","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"name\":"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"Moss\"}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}"#,
        ]);
        let reply = reply.unwrap();
        let call = reply.tool_calls().next().unwrap();
        assert_eq!(call["input"]["name"], "Moss");
    }

    #[test]
    fn broken_tool_input_is_kept_for_the_error_reply() {
        let (reply, _) = assemble(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"get_tone","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"name\":"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
        ]);
        let reply = reply.unwrap();
        let input = &reply.content[0]["input"];
        assert_eq!(input[crate::tools::UNREADABLE_INPUT], "{\"name\":");
    }

    #[test]
    fn an_unknown_block_passes_through_for_replay() {
        let (reply, _) = assemble(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"fallback","to":{"model":"x"}}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
        ]);
        assert_eq!(reply.unwrap().content[0]["to"]["model"], "x");
    }

    #[test]
    fn a_mid_stream_overload_reads_as_one() {
        let (reply, _) = assemble(&[r#"{"type":"error","error":{"type":"overloaded_error"}}"#]);
        assert_eq!(reply, Err(Error::Overloaded));
    }

    #[test]
    fn errors_never_mention_a_key_value() {
        let error = Client {
            origin: "http://127.0.0.1:1".into(),
            key: "sk-ant-secret".into(),
            agent: agent(),
        }
        .check_key()
        .unwrap_err();
        assert!(!error.to_string().contains("sk-ant-secret"));
    }

    #[test]
    fn only_a_loopback_override_is_honored() {
        let loopback = |origin: &str| {
            Url::parse(origin).is_ok_and(|url| {
                matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            })
        };
        assert!(loopback("http://127.0.0.1:9000"));
        assert!(!loopback("https://evil.example"));
    }
}
