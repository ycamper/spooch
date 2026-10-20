//! Accounts: making one, signing a device in with a name and password,
//! and changing the password. Signing in gives the device its token, as
//! pasting one did.

use serde::Deserialize;

use crate::{Client, agent, done, read};

/// A device just signed in: its token (shown this once), its id on the
/// service, and the account's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedIn {
    pub token: String,
    pub device: i64,
    pub name: String,
}

#[derive(Deserialize)]
struct Reply {
    name: String,
    token: String,
    device: i64,
}

impl From<Reply> for SignedIn {
    fn from(r: Reply) -> Self {
        SignedIn {
            token: r.token,
            device: r.device,
            name: r.name,
        }
    }
}

fn url(server: &str, path: &str) -> String {
    format!("{}/v1{path}", server.trim_end_matches('/'))
}

/// Make account `name` on `server` with the sign-up code it's set with,
/// and sign this device (`device` names it) in.
pub fn create_account(
    server: &str,
    name: &str,
    password: &str,
    code: &str,
    device: &str,
) -> Result<SignedIn, String> {
    let reply: Reply = read(agent().post(&url(server, "/accounts")).send_json(
        serde_json::json!({"name": name, "password": password, "code": code, "device": device}),
    ))
    .map_err(|f| f.message)?;
    Ok(reply.into())
}

/// Sign this device in to account `name`.
pub fn sign_in(server: &str, name: &str, password: &str, device: &str) -> Result<SignedIn, String> {
    let reply: Reply = read(
        agent()
            .post(&url(server, "/sign-in"))
            .send_json(serde_json::json!({"name": name, "password": password, "device": device})),
    )
    // A 401 here is a wrong name or password, not the token.
    .map_err(|f| f.message)?;
    Ok(reply.into())
}

impl Client {
    /// Set the account's password, or change it: `current` is the one it
    /// has, if any.
    pub fn set_password(&self, current: Option<&str>, password: &str) -> Result<(), String> {
        done(self.api("PUT", "/password").send_json(
            serde_json::json!({"current": current.unwrap_or(""), "password": password}),
        ))?;
        Ok(())
    }

    /// Sign device `id` out of the account: this one, or another.
    pub fn sign_out_device(&self, id: i64) -> Result<(), String> {
        Ok(done(self.api("DELETE", &format!("/devices/{id}")).call())?)
    }
}
