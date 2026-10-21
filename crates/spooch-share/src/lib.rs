//! Sharing a capture through Spooch's service (`service/`), and the only
//! code in Spooch that sends pixels off the machine.
//!
//! A share's link is made here first, so it can go on the clipboard at
//! once; the upload follows on a background thread. The service only ever
//! learns a hash of the link's secret.

mod account;
mod device;
mod sync;

use std::io::Read;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub use account::{SignedIn, create_account, sign_in};
pub use device::{forget_device, forget_token, load_device, load_token, save_device, save_token};
pub use sync::{Change, Changes, Pushed, new_capture_id};

const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The most of a reply the service is trusted to send.
const MAX_REPLY: u64 = 64 << 10;

/// `n` random base62 characters.
fn random(n: usize) -> String {
    let mut out = String::with_capacity(n);
    let mut buf = [0u8; 64];
    while out.len() < n {
        getrandom::getrandom(&mut buf).expect("the system has randomness");
        // 248 = 4 * 62: keep the choice even.
        for &b in buf.iter().filter(|&&b| b < 248).take(n - out.len()) {
            out.push(char::from(ALPHABET[usize::from(b % 62)]));
        }
    }
    out
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A share's link: a public id and a 128-bit secret.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Link {
    id: String,
    secret: String,
}

impl Link {
    fn new() -> Link {
        Link {
            id: random(10),
            secret: random(22),
        }
    }

    /// The page anyone with the link opens.
    fn url(&self, base: &str) -> String {
        format!("{}/{}{}", base.trim_end_matches('/'), self.id, self.secret)
    }
}

/// Who may see a share, and for how long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Anyone with the link.
    pub public: bool,
    /// Accounts it's shared with, by name.
    pub recipients: Vec<String>,
    /// 0 for the service's default (30 days), at most 90.
    pub expiry_days: i32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            public: true,
            recipients: Vec::new(),
            expiry_days: 0,
        }
    }
}

/// A signed-in connection to the service.
#[derive(Clone)]
pub struct Client {
    server: String,
    token: String,
    agent: ureq::Agent,
    /// Where links point, once `me` has said; the server until then,
    /// which answers them too.
    links: Arc<OnceLock<String>>,
}

#[derive(Deserialize)]
struct Created {
    upload_url: String,
}

#[derive(Deserialize)]
struct Me {
    name: String,
    links: String,
}

#[derive(Deserialize)]
struct Problem {
    error: String,
}

impl Client {
    pub fn new(server: &str, token: &str) -> Client {
        Client {
            server: server.trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent: agent(),
            links: Arc::default(),
        }
    }

    fn api(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(method, &format!("{}/v1{path}", self.server))
            .set("Authorization", &format!("Bearer {}", self.token))
    }

    /// The account the token belongs to. Also learns where links point.
    pub fn me(&self) -> Result<String, String> {
        let me: Me = read(self.api("GET", "/me").call())?;
        let _ = self.links.set(me.links.trim_end_matches('/').to_string());
        Ok(me.name)
    }

    /// Share `png`: the link comes back at once, and the upload runs on a
    /// thread that calls `done` with the link or why it failed.
    pub fn share(
        &self,
        png: Vec<u8>,
        options: Options,
        done: impl FnOnce(Result<String, String>) + Send + 'static,
    ) -> String {
        let link = Link::new();
        let url = link.url(self.links.get().unwrap_or(&self.server));
        let client = self.clone();
        let shown = url.clone();
        thread::spawn(move || done(client.upload(&link, &png, &options).map(|()| shown)));
        url
    }

    /// Create, upload and finalize, in that order.
    fn upload(&self, link: &Link, png: &[u8], o: &Options) -> Result<(), String> {
        let created: Created = read(self.api("POST", "/shares").send_json(serde_json::json!({
            "id": link.id,
            "secret_hash": sha256_hex(&link.secret),
            "size": png.len(),
            "type": "image/png",
            "expiry_days": o.expiry_days,
            "public": o.public,
            "recipients": o.recipients,
        })))?;
        self.agent
            .put(&created.upload_url)
            .set("Content-Type", "image/png")
            .send_bytes(png)
            .map_err(|e| format!("the upload failed: {}", brief(e)))?;
        read::<serde_json::Value>(
            self.api("POST", &format!("/shares/{}/finalize", link.id))
                .call(),
        )?;
        Ok(())
    }
}

/// How Spooch talks to the service.
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        // Per read and write, so a big upload on a slow link works.
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(30))
        // A token or password must never follow a redirect elsewhere.
        .redirects(0)
        .build()
}

/// What went wrong with a request: the status, when the service
/// answered, and its own words (or ours, when it didn't say).
struct Failure {
    status: Option<u16>,
    message: String,
}

/// For requests made with the device token, where a 401 means the token
/// was refused. Sign-in, the one 401 without a token, keeps `message`.
impl From<Failure> for String {
    fn from(f: Failure) -> String {
        match f.status {
            Some(401) => "the device token isn't valid; sign in again".into(),
            _ => f.message,
        }
    }
}

/// A JSON reply, or what went wrong.
fn read<T: serde::de::DeserializeOwned>(
    reply: Result<ureq::Response, ureq::Error>,
) -> Result<T, Failure> {
    match reply {
        Ok(r) => json(r).map_err(|e| Failure {
            status: None,
            message: format!("the service's reply: {e}"),
        }),
        Err(e) => Err(problem(e)),
    }
}

/// A reply with nothing to read, like 204, or what went wrong.
fn done(reply: Result<ureq::Response, ureq::Error>) -> Result<(), Failure> {
    reply.map(|_| ()).map_err(problem)
}

fn problem(e: ureq::Error) -> Failure {
    match e {
        ureq::Error::Status(code, r) => Failure {
            status: Some(code),
            message: json::<Problem>(r)
                .map_or_else(|_| format!("the service said {code}"), |p| p.error),
        },
        e => Failure {
            status: None,
            message: brief(e),
        },
    }
}

/// A reply's JSON, read no further than `MAX_REPLY`.
fn json<T: serde::de::DeserializeOwned>(r: ureq::Response) -> serde_json::Result<T> {
    serde_json::from_reader(r.into_reader().take(MAX_REPLY))
}

/// What went wrong, without the URL: an upload's is signed.
fn brief(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("status {code}"),
        ureq::Error::Transport(t) => format!("couldn't reach the service ({})", t.kind()),
    }
}

#[cfg(test)]
mod tests;
