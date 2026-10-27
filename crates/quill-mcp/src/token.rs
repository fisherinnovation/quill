//! The token an assistant needs to use the [`http`](crate::http) server:
//! random, made the first time it's needed, and kept in the OS secure store
//! (macOS Keychain, Windows Credential Manager) beside the OpenRouter key,
//! never in a file or a log. Renewing it locks out every assistant set up
//! with the old one.

/// The keychain service Quill's secrets share, and this one's account.
const SERVICE: &str = "com.fisherinnovation.quill";
const ACCOUNT: &str = "assistant-token";

/// Random bytes in a token: 256 bits.
const BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("keychain: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("no randomness for a token: {0}")]
    Random(getrandom::Error),
}

/// The stored token, made and stored first if there isn't one. On macOS,
/// the app that stored it reads it without a prompt.
pub fn load_or_create() -> Result<String, TokenError> {
    match keyring::Entry::new(SERVICE, ACCOUNT)?.get_password() {
        Ok(token) => Ok(token),
        Err(keyring::Error::NoEntry) => renew(),
        Err(e) => Err(e.into()),
    }
}

/// Replaces the token with a new one and returns it.
pub fn renew() -> Result<String, TokenError> {
    let token = generate()?;
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)?;
    // Replace rather than update, as for API keys (`quill_llm::keys::set`):
    // on macOS, the program that creates an item reads it without a prompt.
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.into()),
    }
    entry.set_password(&token)?;
    Ok(token)
}

/// A new random token, as hex.
fn generate() -> Result<String, TokenError> {
    let mut bytes = [0u8; BYTES];
    getrandom::fill(&mut bytes).map_err(TokenError::Random)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compares a presented token with the real one in time that doesn't depend
/// on where they differ, so it can't be guessed a character at a time.
pub fn matches(presented: &str, token: &str) -> bool {
    let (a, b) = (presented.as_bytes(), token.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_random_hex() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        assert_eq!(a.len(), BYTES * 2);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn only_the_same_token_matches() {
        assert!(matches("abc123", "abc123"));
        assert!(!matches("abc124", "abc123"));
        assert!(!matches("abc12", "abc123"));
        assert!(!matches("", "abc123"));
    }
}
