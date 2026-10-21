//! Sharing from the editor, and the account it shares from: the service
//! from `settings.txt`, signed in with the device token the keychain holds
//! for it. Signing in, making an account and changing its password ask
//! through forms each shell shows its own way.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use spooch_platform::app::Signed;
use spooch_platform::log;
use spooch_render::SHARE_SHORTCUT;
use spooch_share::{Client, Options};

/// The shell keeps one, in `Carry`, and sessions share it. Clones share
/// their state, so a sign-in on another thread shows everywhere.
#[derive(Clone, Default)]
pub struct Sharing {
    server: String,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    client: Option<Client>,
    /// The account's name, once the service has said.
    account: Option<String>,
}

/// A form a shell shows: a title, what to say, and fields to fill in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub title: &'static str,
    pub prompt: String,
    pub fields: Vec<Field>,
}

/// One field of a form, filled in with `value` to start with; `secret`
/// fields hide what's typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub secret: bool,
    pub value: String,
}

impl Field {
    fn text(label: &'static str, value: &str) -> Field {
        Field {
            label,
            secret: false,
            value: value.to_string(),
        }
    }

    fn secret(label: &'static str) -> Field {
        Field {
            label,
            secret: true,
            value: String::new(),
        }
    }
}

const SIGN_IN_PROMPT: &str = "Sign in to your Spooch account.";
const CREATE_PROMPT: &str = "Make a Spooch account to share and sync captures. A name is 3 to 32 \
                             lowercase letters, digits, dots, dashes or underscores; a password, \
                             at least 10 characters. The sign-up code comes from whoever runs the \
                             service.";
const PASSWORD_PROMPT: &str = "Set your account's password, to sign in on other devices. Leave \
                               the current one empty if it has none yet.";

/// What this device is called on the service: the computer's name.
pub fn device_label() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let os = if cfg!(windows) { "Windows" } else { "Linux" };
    match name {
        Some(n) => format!("{n} ({os})").chars().take(64).collect(),
        None => format!("Spooch on {os}"),
    }
}

/// Whether `name` could be an account name, as the service has them.
fn good_name(name: &str) -> bool {
    let b = name.as_bytes();
    (3..=32).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(c))
}

/// Why a new password won't do, if it won't.
fn bad_password(password: &str, again: &str) -> Option<&'static str> {
    if password.len() < 10 {
        Some("a password is at least 10 characters")
    } else if password != again {
        Some("the two passwords differ")
    } else {
        None
    }
}

impl Sharing {
    /// Sharing to `server`. A thread reads the keychain (which can ask to
    /// be unlocked) and, with a token, asks the service who it belongs to
    /// and where links point.
    pub fn new(server: &str) -> Sharing {
        let sharing = Sharing {
            server: server.to_string(),
            state: Arc::default(),
        };
        let loading = sharing.clone();
        std::thread::spawn(move || {
            let Some(token) = spooch_share::load_token(&loading.server) else {
                return;
            };
            let client = Client::new(&loading.server, &token);
            loading.lock().client = Some(client.clone());
            // Spooch often starts before the network is up. Until `me`
            // answers, links would point at the API server, so keep asking.
            let mut wait = Duration::from_secs(5);
            let mut failed = false;
            loop {
                match client.me() {
                    Ok(name) => {
                        if failed {
                            log::line("sharing: reached the service");
                        }
                        loading.lock().account = Some(name);
                        return;
                    }
                    Err(e) if !failed => {
                        log::line(format!("sharing: {e}; trying again"));
                        failed = true;
                    }
                    Err(_) => {}
                }
                std::thread::sleep(wait);
                // Signed out, or in again with another token, meanwhile.
                if !loading
                    .lock()
                    .client
                    .as_ref()
                    .is_some_and(|c| c.is(&client))
                {
                    return;
                }
                wait = (wait * 2).min(Duration::from_secs(300));
            }
        });
        sharing
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The signed-in client, for syncing.
    pub(crate) fn client(&self) -> Option<Client> {
        self.lock().client.clone()
    }

    pub fn signed(&self) -> Signed {
        let s = self.lock();
        match &s.client {
            Some(_) => Signed::In(s.account.clone()),
            None => Signed::Out,
        }
    }

    /// Keep a password sign-in: its token and device in the keychain, and
    /// a client for it. Returns the account's name.
    fn keep(&self, signed: spooch_share::SignedIn) -> Result<String, String> {
        spooch_share::save_token(&self.server, &signed.token)?;
        if let Err(e) = spooch_share::save_device(&self.server, signed.device) {
            log::line(format!("sign-in: {e}"));
        }
        let client = Client::new(&self.server, &signed.token);
        // Learns where links point; the name is known already.
        let _ = client.me();
        let mut s = self.lock();
        s.client = Some(client);
        s.account = Some(signed.name.clone());
        Ok(signed.name)
    }

    /// Check `token` with the service and keep it in the keychain.
    /// Returns the account's name. Waits on the network.
    fn sign_in(&self, token: &str) -> Result<String, String> {
        let token = token.trim();
        if !looks_like_token(token) {
            return Err("that isn't a device token; they start with spd_".into());
        }
        let client = Client::new(&self.server, token);
        let name = client.me()?;
        spooch_share::save_token(&self.server, token)?;
        let mut s = self.lock();
        s.client = Some(client);
        s.account = Some(name.clone());
        Ok(name)
    }

    /// Ask with `form` until `act` takes the answers, telling why it
    /// didn't each time. `None` if cancelled.
    fn asking<T>(
        &self,
        mut form: Form,
        ask: &mut impl FnMut(&Form) -> Option<Vec<String>>,
        mut act: impl FnMut(&[String]) -> Result<T, String>,
    ) -> Option<T> {
        let prompt = form.prompt.clone();
        loop {
            let answers = ask(&form)?;
            match act(&answers) {
                Ok(t) => return Some(t),
                Err(e) => {
                    log::line(format!("{}: {e}", form.title.to_lowercase()));
                    form.prompt = format!("That didn't work: {e}.\n\n{prompt}");
                    // What was typed stays, but not passwords.
                    for (f, a) in form.fields.iter_mut().zip(&answers) {
                        if !f.secret {
                            f.value = a.clone();
                        }
                    }
                }
            }
        }
    }

    /// Sign in: with `token` if one was copied, else with a name and
    /// password `ask` gets from a form (`None` when cancelled). Returns
    /// what to tell the person, or `None` if cancelled. Waits on the
    /// person and the network, so run it on a thread.
    pub fn sign_in_asking(
        &self,
        token: Option<String>,
        mut ask: impl FnMut(&Form) -> Option<Vec<String>>,
    ) -> Option<String> {
        let name = match token {
            Some(t) => match self.sign_in(&t) {
                Ok(name) => name,
                Err(e) => {
                    log::line(format!("sign-in failed: {e}"));
                    return Some(format!("That device token didn't work: {e}."));
                }
            },
            None => {
                let form = Form {
                    title: "Sign in",
                    prompt: SIGN_IN_PROMPT.into(),
                    fields: vec![Field::text("Name", ""), Field::secret("Password")],
                };
                self.asking(form, &mut ask, |a| {
                    let name = a[0].trim();
                    let signed = spooch_share::sign_in(&self.server, name, &a[1], &device_label())?;
                    self.keep(signed)
                })?
            }
        };
        log::line(format!("signed in as {name}"));
        Some(format!(
            "Signed in as {name}. {SHARE_SHORTCUT} in the editor shares a capture and copies \
             its link."
        ))
    }

    /// Make an account, with the details a form asks for, and sign in to
    /// it. Returns what to tell the person, or `None` if cancelled.
    pub fn create_account_asking(
        &self,
        mut ask: impl FnMut(&Form) -> Option<Vec<String>>,
    ) -> Option<String> {
        let form = Form {
            title: "Create account",
            prompt: CREATE_PROMPT.into(),
            fields: vec![
                Field::text("Name", ""),
                Field::secret("Password"),
                Field::secret("Password again"),
                Field::text("Sign-up code", ""),
            ],
        };
        let name = self.asking(form, &mut ask, |a| {
            let name = a[0].trim();
            if !good_name(name) {
                return Err("that name won't do".into());
            }
            if let Some(why) = bad_password(&a[1], &a[2]) {
                return Err(why.into());
            }
            let made = spooch_share::create_account(
                &self.server,
                name,
                &a[1],
                a[3].trim(),
                &device_label(),
            )?;
            self.keep(made)
        })?;
        log::line(format!("made account {name}"));
        Some(format!(
            "Made the account {name}, and signed in. Sign in with it on your other devices."
        ))
    }

    /// Set or change the account's password with a form. Returns what to
    /// tell the person, or `None` if cancelled.
    pub fn set_password_asking(
        &self,
        mut ask: impl FnMut(&Form) -> Option<Vec<String>>,
    ) -> Option<String> {
        let Some(client) = self.lock().client.clone() else {
            return Some("Sign in first.".into());
        };
        let form = Form {
            title: "Set password",
            prompt: PASSWORD_PROMPT.into(),
            fields: vec![
                Field::secret("Current password"),
                Field::secret("New password"),
                Field::secret("New password again"),
            ],
        };
        self.asking(form, &mut ask, |a| {
            if let Some(why) = bad_password(&a[1], &a[2]) {
                return Err(why.into());
            }
            let current = (!a[0].is_empty()).then_some(a[0].as_str());
            client.set_password(current, &a[1])
        })?;
        log::line("set the account's password");
        Some("Password set. Sign in with it on your other devices.".into())
    }

    /// Sign out: forget the token, and tell the service this device is
    /// gone if it knows which it is (on a thread; the network can wait).
    pub fn sign_out(&self) {
        let client = {
            let mut s = self.lock();
            s.account = None;
            s.client.take()
        };
        if let (Some(client), Some(id)) = (client, spooch_share::load_device(&self.server)) {
            std::thread::spawn(move || {
                if let Err(e) = client.sign_out_device(id) {
                    log::line(format!("sign-out on the service: {e}"));
                }
            });
        }
        let forgot = spooch_share::forget_token(&self.server)
            .and_then(|()| spooch_share::forget_device(&self.server));
        match forgot {
            Ok(()) => log::line("signed out"),
            Err(e) => log::line(format!("sign-out: {e}")),
        }
    }

    /// Share the finished image (`Session::export`): the link comes back at
    /// once for the clipboard, and the upload runs in the background;
    /// `done` gets the link if it worked, and the log hears why not. An
    /// error is a few words for the Share button.
    pub fn share(
        &self,
        (width, height, bgra): (u32, u32, Vec<u8>),
        done: impl FnOnce(Option<String>) + Send + 'static,
    ) -> Result<String, String> {
        let client = self
            .lock()
            .client
            .clone()
            .ok_or("Sign in from the tray first")?;
        let png = spooch_platform::encode_png(width, height, &bgra).map_err(|e| {
            log::line(format!("share failed: {e}"));
            "Share failed".to_string()
        })?;
        let kb = png.len() / 1024;
        Ok(client.share(png, Options::default(), move |r| match r {
            Ok(link) => {
                log::line(format!("shared {width}x{height} ({kb} KB)"));
                done(Some(link));
            }
            Err(e) => {
                log::line(format!("share failed: {e}"));
                done(None);
            }
        }))
    }
}

/// Whether `text` could be a device token, so a sign-in can take one from
/// the clipboard.
pub fn looks_like_token(text: &str) -> bool {
    let text = text.trim();
    text.len() == 36
        && text.starts_with("spd_")
        && text[4..].bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_recognised() {
        assert!(looks_like_token(" spd_0123456789abcdefABCDEF0123456789\n"));
        assert!(!looks_like_token("spd_short"));
        assert!(!looks_like_token("xyz_0123456789abcdefABCDEF0123456789"));
    }

    #[test]
    fn a_bad_copied_token_says_so() {
        let sharing = Sharing::default();
        assert_eq!(sharing.signed(), Signed::Out);
        let notice = sharing.sign_in_asking(Some("not a token".into()), |_| {
            panic!("asked, with a token copied")
        });
        assert!(notice.unwrap().contains("isn't a device token"));
    }

    #[test]
    fn a_new_account_is_checked_before_it_goes_anywhere() {
        let sharing = Sharing::default();
        let mut asked: Vec<Form> = Vec::new();
        let answers = [
            ["Bad Name", "long enough pw", "long enough pw", "code"],
            ["dana", "short", "short", "code"],
            ["dana", "long enough pw", "different pw!", "code"],
        ];
        let notice = sharing.create_account_asking(|form| {
            asked.push(form.clone());
            answers
                .get(asked.len() - 1)
                .map(|a| a.map(String::from).to_vec())
        });
        assert_eq!(notice, None, "cancelled after three tries");
        assert_eq!(asked.len(), 4);
        assert!(
            asked[1]
                .prompt
                .starts_with("That didn't work: that name won't do")
        );
        assert!(asked[2].prompt.contains("at least 10 characters"));
        assert!(asked[3].prompt.contains("the two passwords differ"));
        // What was typed is kept, passwords aren't.
        assert_eq!(asked[3].fields[0].value, "dana");
        assert_eq!(asked[3].fields[1].value, "");
    }

    #[test]
    fn names_are_checked_as_the_service_does() {
        assert!(good_name("mark") && good_name("a.b-c_1") && good_name("007"));
        assert!(!good_name("ab") && !good_name("Mark") && !good_name("-x1") && !good_name("a b"));
        assert!(device_label().chars().count() <= 64);
    }
}
