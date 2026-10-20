//! The device token, kept in the OS keychain (Windows Credential Manager,
//! the Secret Service on Linux), never in a file. One per server, with the
//! device's id on the service when it signed in with a password.

use keyring::Entry;

const SERVICE: &str = "Spooch sharing";

fn entry(server: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, server).map_err(|e| e.to_string())
}

/// The token saved for `server`, if there is one.
pub fn load_token(server: &str) -> Option<String> {
    entry(server).ok()?.get_password().ok()
}

pub fn save_token(server: &str, token: &str) -> Result<(), String> {
    entry(server)?
        .set_password(token)
        .map_err(|e| format!("couldn't save the token in the keychain: {e}"))
}

/// Forget `server`'s token. Not having one is fine.
pub fn forget_token(server: &str) -> Result<(), String> {
    match entry(server)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// The keychain entry for the device's id on `server`.
fn device_entry(server: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, &format!("{server}#device")).map_err(|e| e.to_string())
}

/// This device's id on `server`, if it signed in with a password.
pub fn load_device(server: &str) -> Option<i64> {
    device_entry(server).ok()?.get_password().ok()?.parse().ok()
}

pub fn save_device(server: &str, id: i64) -> Result<(), String> {
    device_entry(server)?
        .set_password(&id.to_string())
        .map_err(|e| format!("couldn't save the device in the keychain: {e}"))
}

pub fn forget_device(server: &str) -> Result<(), String> {
    match device_entry(server)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
