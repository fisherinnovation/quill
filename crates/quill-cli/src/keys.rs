//! `quill-cli keys ...`: manage API keys in the OS keychain.

use std::io::{IsTerminal, Read};

use anyhow::{Context, bail};
use clap::Subcommand;
use quill_llm::keys;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Store an API key (prompted for, never echoed or passed as an argument)
    Set {
        /// Provider; currently only `openrouter`
        provider: String,
    },
    /// Show whether a key is configured and where it comes from
    Status,
    /// Remove a stored key
    Delete { provider: String },
}

pub fn run(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Set { provider } => {
            check_provider(&provider)?;
            // Interactive: hidden prompt. Piped (`… | quill-cli keys set`):
            // read stdin. Never take the key as an argument, which would
            // leave it in shell history.
            let key = if std::io::stdin().is_terminal() {
                rpassword::prompt_password(format!("{provider} API key: "))?
            } else {
                let mut key = String::new();
                std::io::stdin().read_to_string(&mut key)?;
                key
            };
            let key = key.trim();
            if key.is_empty() {
                bail!("no key entered");
            }
            keys::set(&provider, key).context("saving key to the keychain")?;
            eprintln!("Saved {provider} key to the keychain.");
        }
        Command::Status => match keys::openrouter_key_source()? {
            Some(source) => println!("openrouter: configured ({source})"),
            None => println!("openrouter: not set (run `quill-cli keys set openrouter`)"),
        },
        Command::Delete { provider } => {
            check_provider(&provider)?;
            if keys::delete(&provider)? {
                eprintln!("Deleted {provider} key.");
            } else {
                eprintln!("No {provider} key was stored.");
            }
        }
    }
    Ok(())
}

fn check_provider(provider: &str) -> anyhow::Result<()> {
    if provider != keys::OPENROUTER {
        bail!(
            "unknown provider `{provider}`; supported: {}",
            keys::OPENROUTER
        );
    }
    Ok(())
}
