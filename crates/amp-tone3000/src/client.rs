use crate::{
    Filter, Model, Page, Picked, SignIn, Tone, User, WEBSITE,
    store::{TokenStore, Tokens},
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, atomic::AtomicBool},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use ureq::{Agent, Body, http::Response};
use url::Url;

const TOKEN_FILE: &str = "tone3000.tokens";
/// Refresh this long before a token runs out, so a call never dies mid-way.
const REFRESH_MARGIN: u64 = 60;
const MAX_MODEL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;
const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
/// The most a list endpoint serves at once, and all a tone's models can be.
const MODEL_PAGE: u32 = 300;
const LIST_PAGE: u32 = 25;
/// `AMP_GPUI_TONE3000` points the client at a test server.
const ORIGIN_OVERRIDE: &str = "AMP_GPUI_TONE3000";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't reach TONE3000: {0}")]
    Http(#[from] ureq::Error),
    #[error("TONE3000 answered with an error ({0})")]
    Status(u16),
    #[error("TONE3000 is limiting requests. Try again in a minute")]
    RateLimited,
    #[error("sign in to TONE3000 again")]
    SignedOut,
    #[error("sign-in was canceled")]
    Canceled,
    #[error("TONE3000 sign-in failed: {0}")]
    Denied(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("couldn't read TONE3000's reply: {0}")]
    Url(#[from] url::ParseError),
    #[error("couldn't read the image: {0}")]
    Image(#[from] image::ImageError),
    #[error("couldn't set up a secure sign-in: {0}")]
    Random(String),
}

/// The lists the free API tier offers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum List {
    Favorites,
    Created,
    Downloaded,
    Trending,
    Latest,
}

impl List {
    fn path(self) -> &'static str {
        match self {
            Self::Favorites => "favorited",
            Self::Created => "created",
            Self::Downloaded => "downloaded",
            Self::Trending => "trending",
            Self::Latest => "latest",
        }
    }

    /// Trending and Latest are fixed feeds of ten, so those lists are partial.
    pub fn is_feed(self) -> bool {
        matches!(self, Self::Trending | Self::Latest)
    }
}

/// The feeds aren't paged, so they may come as a bare list.
#[derive(Deserialize)]
#[serde(untagged)]
enum Feed {
    Page(Page<Tone>),
    Plain(Vec<Tone>),
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn agent() -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(300)))
        .build()
        .into()
}

/// Talks to TONE3000 on behalf of the signed-in user. It holds the session, so
/// share one between threads.
pub struct Client {
    origin: String,
    client_id: String,
    agent: Agent,
    store: TokenStore,
    tokens: Mutex<Option<Tokens>>,
}

impl Client {
    /// `client_id` is the publishable key. Tokens are kept in `data_dir`.
    pub fn new(client_id: &str, data_dir: &Path) -> Self {
        let origin = std::env::var(ORIGIN_OVERRIDE).unwrap_or_else(|_| WEBSITE.to_owned());
        Self::at(origin, client_id, data_dir.join(TOKEN_FILE))
    }

    fn at(origin: String, client_id: &str, token_file: PathBuf) -> Self {
        let store = TokenStore::new(token_file);
        Self {
            origin,
            client_id: client_id.to_owned(),
            agent: agent(),
            tokens: Mutex::new(store.load()),
            store,
        }
    }

    fn session(&self) -> MutexGuard<'_, Option<Tokens>> {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn signed_in(&self) -> bool {
        self.session().is_some()
    }

    /// Forgets the session here. Models already downloaded stay in the library.
    pub fn sign_out(&self) -> Result<(), Error> {
        *self.session() = None;
        Ok(self.store.clear()?)
    }

    /// Opens a listener and builds the address to send the browser to.
    pub fn start_sign_in(&self, filter: Filter) -> Result<SignIn, Error> {
        SignIn::new(&self.origin, &self.client_id, filter)
    }

    /// Waits for the browser, then signs in with what it brought back. This
    /// blocks until then, or until `cancel` is set.
    pub fn finish_sign_in(&self, flow: &SignIn, cancel: &AtomicBool) -> Result<Picked, Error> {
        let (code, picked) = flow.wait(cancel)?;
        let tokens = self.request_tokens(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &flow.verifier),
            ("redirect_uri", &flow.redirect_uri),
            ("client_id", &self.client_id),
        ])?;
        self.keep(&mut self.session(), tokens)?;
        Ok(picked)
    }

    fn keep(&self, session: &mut Option<Tokens>, tokens: Tokens) -> Result<(), Error> {
        let saved = self.store.save(&tokens);
        *session = Some(tokens);
        // A session that can't be saved still works until the app closes.
        if let Err(error) = saved
            && error.kind() != io::ErrorKind::Unsupported
        {
            return Err(error.into());
        }
        Ok(())
    }

    fn request_tokens(&self, form: &[(&str, &str)]) -> Result<Tokens, Error> {
        let url = format!("{}/api/v1/oauth/token", self.origin);
        let mut response = self.agent.post(&url).send_form(form.iter().copied())?;
        match response.status().as_u16() {
            200 => {
                let reply: TokenReply = response.body_mut().read_json()?;
                Ok(Tokens {
                    access_token: reply.access_token,
                    refresh_token: reply.refresh_token,
                    expires_at: now() + reply.expires_in,
                })
            }
            400 | 401 => Err(Error::SignedOut),
            status => Err(Error::Status(status)),
        }
    }

    /// Trades the refresh token for a new pair. The lock is held throughout, so
    /// two calls can't spend the same token. Only a refused token ends the
    /// session: a network failure leaves it for the next try.
    fn refresh(&self, session: &mut Option<Tokens>) -> Result<(), Error> {
        let Some(tokens) = session.as_ref() else {
            return Err(Error::SignedOut);
        };
        let refresh_token = tokens.refresh_token.clone();
        match self.request_tokens(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("client_id", &self.client_id),
        ]) {
            Ok(tokens) => self.keep(session, tokens),
            Err(Error::SignedOut) => {
                *session = None;
                let _ = self.store.clear();
                Err(Error::SignedOut)
            }
            Err(error) => Err(error),
        }
    }

    fn access_token(&self) -> Result<String, Error> {
        let mut session = self.session();
        let tokens = session.as_ref().ok_or(Error::SignedOut)?;
        if tokens.expires_at <= now() + REFRESH_MARGIN {
            self.refresh(&mut session)?;
        }
        session
            .as_ref()
            .map(|tokens| tokens.access_token.clone())
            .ok_or(Error::SignedOut)
    }

    fn fetch(&self, url: &str, token: &str) -> Result<Response<Body>, Error> {
        Ok(self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .call()?)
    }

    /// Fetches with the session's token, refreshing once if it was refused.
    fn authorized(&self, url: &str) -> Result<Response<Body>, Error> {
        let token = self.access_token()?;
        let response = self.fetch(url, &token)?;
        if response.status().as_u16() != 401 {
            return checked(response);
        }
        {
            let mut session = self.session();
            // Another thread may have refreshed while this one was waiting.
            if session
                .as_ref()
                .is_some_and(|tokens| tokens.access_token == token)
            {
                self.refresh(&mut session)?;
            }
        }
        checked(self.fetch(url, &self.access_token()?)?)
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, Error> {
        let mut url = Url::parse(&format!("{}/api/v1/{path}", self.origin))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut response = self.authorized(url.as_str())?;
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_vec()?;
        serde_json::from_slice(&body).map_err(|error| Error::Io(error.into()))
    }

    pub fn user(&self) -> Result<User, Error> {
        self.get("user", &[])
    }

    pub fn list(&self, list: List, page: u32) -> Result<Page<Tone>, Error> {
        let path = format!("tones/{}", list.path());
        if list.is_feed() {
            return Ok(match self.get(&path, &[])? {
                Feed::Page(page) => page,
                Feed::Plain(data) => Page {
                    data,
                    page: 1,
                    total_pages: 1,
                },
            });
        }
        self.get(
            &path,
            &[
                ("page", &page.to_string()),
                ("page_size", &LIST_PAGE.to_string()),
            ],
        )
    }

    pub fn tone(&self, id: u64) -> Result<Tone, Error> {
        self.get(&format!("tones/{id}"), &[])
    }

    pub fn model(&self, id: u64) -> Result<Model, Error> {
        self.get(&format!("models/{id}"), &[])
    }

    fn models_of(&self, tone: u64, architecture: Option<&str>) -> Result<Vec<Model>, Error> {
        let tone = tone.to_string();
        let size = MODEL_PAGE.to_string();
        let mut query = vec![("tone_id", tone.as_str()), ("page_size", size.as_str())];
        query.extend(architecture.map(|version| ("architecture", version)));
        Ok(self.get::<Page<Model>>("models", &query)?.data)
    }

    /// The tone's models, newest architecture first. The API serves one
    /// architecture per call and leaves out A2 unless asked.
    pub fn models(&self, tone: &Tone) -> Result<Vec<Model>, Error> {
        if !tone.format.as_deref().is_some_and(|format| format == "nam") {
            return self.models_of(tone.id, None);
        }
        let mut models = Vec::new();
        for (count, version) in [(tone.a2_models_count, "2"), (tone.a1_models_count, "1")] {
            if count > 0 {
                models.extend(self.models_of(tone.id, Some(version))?);
            }
        }
        Ok(models)
    }

    /// The bearer token only goes to TONE3000 itself, whatever a URL says.
    fn is_ours(&self, url: &str) -> bool {
        let origin = |text: &str| Url::parse(text).map(|url| url.origin());
        matches!((origin(url), origin(&self.origin)), (Ok(a), Ok(b)) if a == b)
    }

    /// Saves a model's file to `destination`.
    pub fn download(&self, model: &Model, destination: &Path) -> Result<(), Error> {
        let mut response = if self.is_ours(&model.model_url) {
            self.authorized(&model.model_url)?
        } else {
            checked(self.agent.get(&model.model_url).call()?)?
        };
        let mut body = response
            .body_mut()
            .with_config()
            .limit(MAX_MODEL_BYTES)
            .reader();
        io::copy(&mut body, &mut fs::File::create(destination)?)?;
        Ok(())
    }

    /// A PNG of the picture at `url`, no wider or taller than `size`. Pictures
    /// are public and creators upload full-size photos, so this sends no token
    /// and shrinks them before they reach the UI.
    pub fn thumbnail(&self, url: &str, size: u32) -> Result<Vec<u8>, Error> {
        let mut response = checked(self.agent.get(url).call()?)?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_IMAGE_BYTES)
            .read_to_vec()?;
        let mut png = io::Cursor::new(Vec::new());
        image::load_from_memory(&bytes)?
            .thumbnail(size, size)
            .write_to(&mut png, image::ImageFormat::Png)?;
        Ok(png.into_inner())
    }
}

fn checked(response: Response<Body>) -> Result<Response<Body>, Error> {
    match response.status().as_u16() {
        200..=299 => Ok(response),
        401 => Err(Error::SignedOut),
        429 => Err(Error::RateLimited),
        status => Err(Error::Status(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::Arc,
        thread,
    };

    /// What a test server saw.
    #[derive(Clone, Debug)]
    struct Seen {
        line: String,
        bearer: Option<String>,
        form: String,
    }

    /// Answers each request with `reply`, which returns a status and body.
    fn serve(
        reply: impl Fn(&Seen) -> (u16, String) + Send + 'static,
    ) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut bearer = None;
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim();
                    if header.is_empty() {
                        break;
                    }
                    let lower = header.to_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    if lower.starts_with("authorization:") {
                        bearer = header.split_once("Bearer ").map(|(_, t)| t.to_owned());
                    }
                }
                let mut form = vec![0; length];
                reader.read_exact(&mut form).unwrap();
                let request = Seen {
                    line,
                    bearer,
                    form: String::from_utf8(form).unwrap(),
                };
                let (status, body) = reply(&request);
                seen.lock().unwrap().push(request);
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (origin, log)
    }

    fn client(origin: String, name: &str, tokens: Option<Tokens>) -> (Client, PathBuf) {
        let file =
            std::env::temp_dir().join(format!("amp-t3k-client-{name}-{}", std::process::id()));
        let _ = fs::remove_file(&file);
        let client = Client::at(origin, "t3k_pub_key", file.clone());
        *client.session() = tokens;
        (client, file)
    }

    fn tokens(access: &str, expires_at: u64) -> Option<Tokens> {
        Some(Tokens {
            access_token: access.into(),
            refresh_token: "old-refresh".into(),
            expires_at,
        })
    }

    const NEW_TOKENS: &str =
        r#"{"access_token":"new","refresh_token":"new-refresh","expires_in":3600}"#;

    #[test]
    fn an_expiring_token_is_refreshed_first_and_the_new_pair_is_kept() {
        let (origin, log) = serve(|seen| {
            if seen.line.contains("/oauth/token") {
                (200, NEW_TOKENS.to_owned())
            } else {
                (200, r#"{"username":"joey"}"#.to_owned())
            }
        });
        let (client, file) = client(origin, "refresh", tokens("old", now() + 10));
        assert_eq!(client.user().unwrap().username, "joey");

        let log = log.lock().unwrap();
        assert!(log[0].form.contains("grant_type=refresh_token"));
        assert!(log[0].form.contains("refresh_token=old-refresh"));
        assert!(log[0].form.contains("client_id=t3k_pub_key"));
        assert!(!log[0].form.contains("secret"));
        assert_eq!(log[1].bearer.as_deref(), Some("new"));
        assert_eq!(
            client.session().as_ref().unwrap().refresh_token,
            "new-refresh"
        );
        let _ = fs::remove_file(file);
    }

    #[test]
    fn a_refused_token_refreshes_once_and_retries() {
        let (origin, log) = serve(|seen| {
            if seen.line.contains("/oauth/token") {
                (200, NEW_TOKENS.to_owned())
            } else if seen.bearer.as_deref() == Some("old") {
                (401, String::new())
            } else {
                (200, r#"{"username":"joey"}"#.to_owned())
            }
        });
        let (client, file) = client(origin, "retry", tokens("old", now() + 3600));
        assert_eq!(client.user().unwrap().username, "joey");
        assert_eq!(log.lock().unwrap().len(), 3);
        let _ = fs::remove_file(file);
    }

    #[test]
    fn a_dead_refresh_token_ends_the_session() {
        let (origin, _) = serve(|_| (400, r#"{"error":"invalid_grant"}"#.to_owned()));
        let (client, _) = client(origin, "dead", tokens("old", 0));
        assert!(matches!(client.user(), Err(Error::SignedOut)));
        assert!(!client.signed_in());
    }

    #[test]
    fn a_server_error_during_refresh_keeps_the_session() {
        let (origin, _) = serve(|_| (503, String::new()));
        let (client, _) = client(origin, "outage", tokens("old", 0));
        assert!(matches!(client.user(), Err(Error::Status(503))));
        assert!(client.signed_in());
    }

    #[test]
    fn nothing_is_sent_when_signed_out() {
        let (origin, log) = serve(|_| (200, String::new()));
        let (client, _) = client(origin, "out", None);
        assert!(matches!(client.user(), Err(Error::SignedOut)));
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn signing_in_trades_the_code_with_the_verifier_and_no_secret() {
        let (origin, log) = serve(|_| (200, NEW_TOKENS.to_owned()));
        let (client, file) = client(origin, "code", None);
        let flow = client.start_sign_in(Filter::default()).unwrap();
        let state = Url::parse(flow.url())
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let address = Url::parse(&flow.redirect_uri).unwrap();
        let port = address.port().unwrap();
        thread::spawn(move || {
            let mut browser = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(
                browser,
                "GET /callback?code=abc&tone_id=9&state={state} HTTP/1.1\r\n\r\n"
            )
            .unwrap();
            browser.read_to_end(&mut Vec::new()).unwrap();
        });

        let picked = client
            .finish_sign_in(&flow, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(picked.tone_id, Some(9));
        assert!(client.signed_in());
        let log = log.lock().unwrap();
        assert!(log[0].form.contains("grant_type=authorization_code"));
        assert!(log[0].form.contains("code=abc"));
        assert!(log[0].form.contains("code_verifier="));
        assert!(!log[0].form.contains("t3k_cs_"));
        drop(flow);
        let _ = fs::remove_file(file);
    }

    #[test]
    fn nam_tones_list_a2_models_first_and_skip_empty_architectures() {
        let (origin, log) = serve(|seen| {
            let id = if seen.line.contains("architecture=2") {
                2
            } else {
                1
            };
            (200, format!(r#"{{"data":[{{"id":{id}}}]}}"#))
        });
        let (client, _) = client(origin, "models", tokens("t", now() + 3600));
        let tone = |json| serde_json::from_str::<Tone>(json).unwrap();

        let both = tone(r#"{"id":5,"format":"nam","a1_models_count":3,"a2_models_count":3}"#);
        let ids: Vec<u64> = client.models(&both).unwrap().iter().map(|m| m.id).collect();
        assert_eq!(ids, [2, 1]);

        let older = tone(r#"{"id":5,"format":"nam","a1_models_count":3}"#);
        assert_eq!(client.models(&older).unwrap().len(), 1);

        let ir = tone(r#"{"id":5,"format":"ir"}"#);
        log.lock().unwrap().clear();
        assert_eq!(client.models(&ir).unwrap().len(), 1);
        assert!(!log.lock().unwrap()[0].line.contains("architecture"));
    }

    #[test]
    fn feeds_read_as_a_page_or_a_bare_list() {
        let (origin, _) = serve(|seen| {
            if seen.line.contains("trending") {
                (200, r#"{"data":[{"id":1}]}"#.to_owned())
            } else {
                (200, r#"[{"id":2},{"id":3}]"#.to_owned())
            }
        });
        let (client, _) = client(origin, "feeds", tokens("t", now() + 3600));
        assert_eq!(client.list(List::Trending, 1).unwrap().data.len(), 1);
        let latest = client.list(List::Latest, 1).unwrap();
        assert_eq!(latest.data.len(), 2);
        assert!(!latest.has_next());
    }

    #[test]
    fn library_lists_ask_for_the_page_they_want() {
        let (origin, log) = serve(|_| {
            (
                200,
                r#"{"data":[],"page":2,"page_size":25,"total_pages":3}"#.to_owned(),
            )
        });
        let (client, _) = client(origin, "lists", tokens("t", now() + 3600));
        assert!(client.list(List::Favorites, 2).unwrap().has_next());
        let line = log.lock().unwrap()[0].line.clone();
        assert!(line.contains("/api/v1/tones/favorited?page=2&page_size=25"));
    }

    #[test]
    fn the_token_never_goes_to_another_host() {
        let (other, log) = serve(|_| (200, "model".to_owned()));
        let (ours, _) = serve(|_| (200, "ours".to_owned()));
        let (client, _) = client(ours.clone(), "foreign", tokens("t", now() + 3600));
        let destination =
            std::env::temp_dir().join(format!("amp-t3k-download-{}", std::process::id()));
        let model = Model {
            id: 1,
            tone_id: 1,
            name: String::new(),
            model_url: format!("{other}/file.nam"),
            size: None,
            architecture_version: None,
        };
        client.download(&model, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"model");
        assert_eq!(log.lock().unwrap()[0].bearer, None);

        let local = Model {
            model_url: format!("{ours}/file.nam"),
            ..model
        };
        client.download(&local, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"ours");
        fs::remove_file(destination).unwrap();
    }

    #[test]
    fn a_rate_limit_reads_as_one() {
        let (origin, _) = serve(|_| (429, String::new()));
        let (client, _) = client(origin, "limit", tokens("t", now() + 3600));
        assert!(matches!(client.user(), Err(Error::RateLimited)));
    }

    #[test]
    fn signing_out_forgets_the_session() {
        let (origin, _) = serve(|_| (200, String::new()));
        let (client, _) = client(origin, "signout", tokens("t", now() + 3600));
        client.sign_out().unwrap();
        assert!(!client.signed_in());
    }
}
