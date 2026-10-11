//! API keys in the OS secure store (macOS Keychain, Windows Credential
//! Manager), never in config files or logs.

use crate::LlmError;

/// Keychain service name; the "user" is the provider name.
const SERVICE: &str = "com.fisherinnovation.quill";
pub const OPENROUTER: &str = "openrouter";

/// Environment variable that overrides the keychain (handy for CI and for
/// trying a key without storing it).
pub const OPENROUTER_ENV: &str = "QUILL_OPENROUTER_API_KEY";

pub fn set(provider: &str, key: &str) -> Result<(), LlmError> {
    keyring::Entry::new(SERVICE, provider)?.set_password(key.trim())?;
    Ok(())
}

/// Removes a stored key. Returns `false` if there was none.
pub fn delete(provider: &str) -> Result<bool, LlmError> {
    match keyring::Entry::new(SERVICE, provider)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Where the OpenRouter key would come from, without revealing it.
pub fn openrouter_key_source() -> Result<Option<&'static str>, LlmError> {
    if std::env::var(OPENROUTER_ENV).is_ok_and(|k| !k.is_empty()) {
        return Ok(Some("environment"));
    }
    match keyring::Entry::new(SERVICE, OPENROUTER)?.get_password() {
        Ok(_) => Ok(Some("keychain")),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn openrouter_key() -> Result<String, LlmError> {
    if let Ok(key) = std::env::var(OPENROUTER_ENV)
        && !key.is_empty()
    {
        return Ok(key);
    }
    match keyring::Entry::new(SERVICE, OPENROUTER)?.get_password() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => Err(LlmError::MissingKey(OPENROUTER)),
        Err(e) => Err(e.into()),
    }
}
