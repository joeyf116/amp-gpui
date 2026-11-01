use crate::Error;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
use url::Url;

/// The port registered with TONE3000. Any other loopback port is accepted too,
/// so a busy port falls back to one Windows picks.
const REDIRECT_PORT: u16 = 53710;
const REDIRECT_PATH: &str = "/callback";
/// Long enough to go and find the email with the sign-in code.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POLL: Duration = Duration::from_millis(100);
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Amp GPUI</title>\
    <body style=\"background:#0c1410;color:#d6e0da;font-family:sans-serif;\
    display:grid;place-items:center;height:90vh\">\
    <p>All done. You can close this tab and go back to Amp GPUI.</p>";

/// Narrows the catalog to what fits the slot being filled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Filter {
    /// Gear types joined by `_`, such as `amp_amp-cab`.
    pub gears: Option<&'static str>,
    pub format: Option<&'static str>,
    /// Left out of a list, the API leaves A2 models out too.
    pub architecture: Option<&'static str>,
}

/// What came back from the catalog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picked {
    pub tone_id: Option<u64>,
}

/// A sign-in under way: a local listener waiting for the browser to return.
#[derive(Debug)]
pub struct SignIn {
    listener: TcpListener,
    pub(crate) redirect_uri: String,
    pub(crate) verifier: String,
    state: String,
    url: String,
}

fn random_text(bytes: usize) -> Result<String, Error> {
    let mut buffer = vec![0; bytes];
    getrandom::fill(&mut buffer).map_err(|error| Error::Random(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier))
}

fn bind() -> Result<TcpListener, Error> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, REDIRECT_PORT))
        .or_else(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

impl SignIn {
    pub(crate) fn new(origin: &str, client_id: &str, filter: Filter) -> Result<Self, Error> {
        let listener = bind()?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}{REDIRECT_PATH}");
        let verifier = random_text(32)?;
        let state = random_text(16)?;

        let mut url = Url::parse(&format!("{origin}/api/v1/oauth/authorize"))?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("client_id", client_id)
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("response_type", "code")
                .append_pair("code_challenge", &challenge(&verifier))
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &state)
                .append_pair("prompt", "select_tone")
                .append_pair("preview", "true");
            for (key, value) in [
                ("gears", filter.gears),
                ("format", filter.format),
                ("architecture", filter.architecture),
            ] {
                if let Some(value) = value {
                    query.append_pair(key, value);
                }
            }
        }
        Ok(Self {
            listener,
            redirect_uri,
            verifier,
            state,
            url: url.into(),
        })
    }

    /// Where to send the browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Waits for the browser to come back with the code to trade for tokens.
    /// Setting `cancel` gives up. A sign-in that was closed partway can still
    /// carry a code, with no tone picked.
    pub(crate) fn wait(&self, cancel: &AtomicBool) -> Result<(String, Picked), Error> {
        let deadline = Instant::now() + SIGN_IN_TIMEOUT;
        loop {
            if cancel.load(Ordering::Relaxed) || Instant::now() > deadline {
                return Err(Error::Canceled);
            }
            match self.listener.accept() {
                Ok((stream, address)) => {
                    if let Some(outcome) = self.answer(stream, address) {
                        return outcome;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(POLL);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Reads one request. Browsers also ask for things like a favicon, which
    /// get a 404 and leave the wait running.
    fn answer(
        &self,
        stream: TcpStream,
        address: SocketAddr,
    ) -> Option<Result<(String, Picked), Error>> {
        if !address.ip().is_loopback() {
            return None;
        }
        stream.set_nonblocking(false).ok()?;
        stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        let target = line.split_whitespace().nth(1)?;
        let url = Url::parse("http://127.0.0.1").ok()?.join(target).ok()?;
        let mut stream = stream;
        if url.path() != REDIRECT_PATH {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n");
            return None;
        }
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{DONE_PAGE}",
            DONE_PAGE.len()
        );
        Some(self.read_callback(&url))
    }

    /// Nothing else on the callback is trusted until `state` matches, because
    /// any web page can make a browser request a loopback address.
    fn read_callback(&self, url: &Url) -> Result<(String, Picked), Error> {
        let value = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        if value("state").as_deref() != Some(&self.state) {
            return Err(Error::Denied("the sign-in reply didn't match".to_owned()));
        }
        if let Some(error) = value("error") {
            return Err(Error::Denied(value("error_description").unwrap_or(error)));
        }
        let Some(code) = value("code") else {
            return Err(Error::Canceled);
        };
        let tone_id = value("tone_id").and_then(|id| id.parse().ok());
        Ok((code, Picked { tone_id }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, sync::Arc};

    fn sign_in(filter: Filter) -> SignIn {
        SignIn::new("https://example.test", "t3k_pub_key", filter).unwrap()
    }

    fn query(sign_in: &SignIn, name: &str) -> Option<String> {
        Url::parse(sign_in.url())
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }

    /// Plays the browser: requests `target`, and reads the reply.
    fn visit(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    #[test]
    fn the_challenge_matches_the_rfc_7636_example() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn the_authorize_url_carries_pkce_the_redirect_and_the_filter() {
        let flow = sign_in(Filter {
            gears: Some("amp_amp-cab"),
            format: Some("nam"),
            architecture: Some("2"),
        });
        assert!(
            flow.url()
                .starts_with("https://example.test/api/v1/oauth/authorize?")
        );
        assert_eq!(query(&flow, "client_id").unwrap(), "t3k_pub_key");
        assert_eq!(query(&flow, "code_challenge_method").unwrap(), "S256");
        assert_eq!(
            query(&flow, "code_challenge").unwrap(),
            challenge(&flow.verifier)
        );
        assert_eq!(query(&flow, "prompt").unwrap(), "select_tone");
        assert_eq!(query(&flow, "gears").unwrap(), "amp_amp-cab");
        assert_eq!(query(&flow, "architecture").unwrap(), "2");
        assert_eq!(query(&flow, "redirect_uri").unwrap(), flow.redirect_uri);
        assert!(flow.redirect_uri.starts_with("http://127.0.0.1:"));
        assert!(flow.redirect_uri.ends_with("/callback"));
        assert!(!flow.url().contains("t3k_cs_"));
    }

    #[test]
    fn a_busy_port_falls_back_to_another() {
        let first = sign_in(Filter::default());
        let second = sign_in(Filter::default());
        assert_ne!(first.redirect_uri, second.redirect_uri);
        assert_eq!(query(&second, "redirect_uri").unwrap(), second.redirect_uri);
    }

    #[test]
    fn the_listener_ignores_strays_and_returns_the_code() {
        let flow = Arc::new(sign_in(Filter::default()));
        let port = flow.listener.local_addr().unwrap().port();
        let state = flow.state.clone();
        let waiting = Arc::clone(&flow);
        let wait = thread::spawn(move || waiting.wait(&AtomicBool::new(false)));

        assert!(visit(port, "/favicon.ico").contains("404"));
        let reply = visit(
            port,
            &format!("/callback?code=abc&state={state}&tone_id=42"),
        );
        assert!(reply.contains("close this tab"));
        let (code, picked) = wait.join().unwrap().unwrap();
        assert_eq!(code, "abc");
        assert_eq!(picked.tone_id, Some(42));
    }

    #[test]
    fn a_reply_with_the_wrong_state_is_refused() {
        let flow = sign_in(Filter::default());
        let url = Url::parse("http://127.0.0.1/callback?code=stolen&state=wrong").unwrap();
        assert!(matches!(flow.read_callback(&url), Err(Error::Denied(_))));
    }

    #[test]
    fn closing_the_flow_counts_as_canceled_unless_a_code_came_with_it() {
        let flow = sign_in(Filter::default());
        let state = &flow.state;
        let closed = Url::parse(&format!(
            "http://127.0.0.1/callback?canceled=true&state={state}"
        ))
        .unwrap();
        assert!(matches!(flow.read_callback(&closed), Err(Error::Canceled)));
        let signed_in = Url::parse(&format!(
            "http://127.0.0.1/callback?canceled=true&code=abc&state={state}"
        ))
        .unwrap();
        let (code, picked) = flow.read_callback(&signed_in).unwrap();
        assert_eq!(code, "abc");
        assert_eq!(picked.tone_id, None);
    }

    #[test]
    fn cancel_stops_the_wait() {
        let flow = sign_in(Filter::default());
        assert!(matches!(
            flow.wait(&AtomicBool::new(true)),
            Err(Error::Canceled)
        ));
    }
}
