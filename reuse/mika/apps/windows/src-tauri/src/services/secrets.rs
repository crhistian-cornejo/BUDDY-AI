// API keys live in the Windows Credential Manager, never on disk and never in
// the front end — the island can only ask whether a key is present.

use keyring::Entry;

const SERVICE: &str = "io.github.crhistian-cornejo.mika";

/// Keys that belong to MIKA itself rather than to an integration.
const CORE_KEYS: &[&str] = &["anthropic-api-key", super::odds::KEY, super::sgo::KEY];

/// Keys only Rust writes, never a webview: the Telegram session (an authorisation key for the user's account) comes
/// from the Telegram helper and goes nowhere else.
const INTERNAL_KEYS: &[&str] = &["telegram-session"];

/// Every key MIKA may store: its own, plus whatever the integration registry
/// declares. Anything else is refused, so a compromised webview cannot use the
/// Credential Manager as a general-purpose store.
pub fn is_known_key(key: &str) -> bool {
    is_user_key(key) || INTERNAL_KEYS.contains(&key)
}

/// The keys the settings window may set, clear or ask about.
pub fn is_user_key(key: &str) -> bool {
    CORE_KEYS.contains(&key) || crate::integrations::secret_keys().any(|k| k == key)
}

fn entry(key: &str) -> Option<Entry> {
    if !is_known_key(key) {
        return None;
    }
    Entry::new(SERVICE, key).ok()
}

pub fn get(key: &str) -> Option<String> {
    entry(key)?.get_password().ok().filter(|v| !v.is_empty())
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    if value.is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry.set_password(value).map_err(|e| e.to_string())
}

pub fn clear(key: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn present(key: &str) -> bool {
    get(key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_integration_key_and_the_api_key_are_accepted() {
        assert!(is_known_key("anthropic-api-key"));
        for key in crate::integrations::secret_keys() {
            assert!(is_known_key(key), "{key} is declared by an integration");
        }
    }

    #[test]
    fn the_telegram_session_is_never_a_webview_key() {
        assert!(is_known_key("telegram-session"));
        assert!(!is_user_key("telegram-session"));
        assert!(is_user_key("telegram-api-hash"));
    }

    #[test]
    fn anything_else_is_refused() {
        for key in ["", "password", "n8n-api-key", "calcom-api-key", "../x", "ANTHROPIC-API-KEY"] {
            assert!(!is_known_key(key), "{key:?} must not be storable");
            assert!(entry(key).is_none());
        }
    }
}
