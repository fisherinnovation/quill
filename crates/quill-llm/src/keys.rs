//! API keys in the OS secure store (macOS Keychain, Windows Credential
//! Manager), never in config files or logs.

use crate::LlmError;

/// Keychain service name; the "user" is the provider name.
const SERVICE: &str = "com.fisherinnovation.quill";
pub const OPENROUTER: &str = "openrouter";

/// Environment variable that overrides the keychain (handy for CI and for
/// trying a key without storing it).
pub const OPENROUTER_ENV: &str = "QUILL_OPENROUTER_API_KEY";

/// Stores a key, replacing any existing one.
///
/// Replacing (delete, then create) rather than updating matters on macOS:
/// the program that *creates* a keychain item is the one allowed to read it
/// without a prompt. So whichever of the app or CLI saved the key last can
/// use it silently. (Unsigned development builds can't be added to an
/// item's allow-list with "Always Allow"; signed builds can.)
pub fn set(provider: &str, key: &str) -> Result<(), LlmError> {
    let entry = keyring::Entry::new(SERVICE, provider)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.into()),
    }
    entry.set_password(key.trim())?;
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

/// Where the OpenRouter key would come from, without reading it: checking
/// only the item's attributes never triggers a keychain access prompt.
pub fn openrouter_key_source() -> Result<Option<&'static str>, LlmError> {
    if std::env::var(OPENROUTER_ENV).is_ok_and(|k| !k.is_empty()) {
        return Ok(Some("environment"));
    }
    match keyring::Entry::new(SERVICE, OPENROUTER)?
        .inner
        .get_attributes()
    {
        Ok(_) => Ok(Some("keychain")),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Reads the OpenRouter key: the environment variable if set, else the
/// keychain (which may show a macOS access prompt).
pub fn openrouter_key() -> Result<String, LlmError> {
    if let Ok(key) = std::env::var(OPENROUTER_ENV)
        && !key.is_empty()
    {
        return Ok(key);
    }
    match keyring::Entry::new(SERVICE, OPENROUTER)?.get_password() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => Err(LlmError::MissingKey(OPENROUTER)),
        // Clicking Deny/Cancel on the macOS prompt surfaces as a platform
        // failure; say what actually happened.
        Err(keyring::Error::PlatformFailure(e))
            if e.to_string().to_lowercase().contains("cancel") =>
        {
            Err(LlmError::KeychainDenied)
        }
        Err(e) => Err(e.into()),
    }
}
