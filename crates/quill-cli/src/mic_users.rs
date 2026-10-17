//! `quill-cli mic-users`: which apps are using a microphone, to check what
//! a call looks like to Quill (Zoom as Zoom? Meet as the browser?).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use quill_audio::mic_users::{MicUser, mic_users};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Keep watching, printing apps as they start and stop using the mic
    #[arg(long)]
    watch: bool,
    /// With --watch, stop after this many seconds [default: until Ctrl-C]
    #[arg(long, value_name = "SECONDS")]
    r#for: Option<u64>,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    if !args.watch {
        let users = mic_users()?;
        if users.is_empty() {
            println!("Nothing is using a microphone.");
        }
        for user in &users {
            println!("{}", describe(user));
        }
        return Ok(());
    }

    let started = Instant::now();
    let until = args.r#for.map(Duration::from_secs);
    println!(
        "Watching which apps use a microphone{}…",
        until.map_or(String::new(), |d| format!(" for {} s", d.as_secs()))
    );
    let mut seen: BTreeMap<i32, MicUser> = BTreeMap::new();
    loop {
        let now: BTreeMap<i32, MicUser> = mic_users()?.into_iter().map(|u| (u.pid, u)).collect();
        let at = started.elapsed().as_secs();
        for (pid, user) in &now {
            if !seen.contains_key(pid) {
                println!("{:>4}s  + {}", at, describe(user));
            }
        }
        for (pid, user) in &seen {
            if !now.contains_key(pid) {
                println!("{:>4}s  - {}", at, describe(user));
            }
        }
        seen = now;
        if until.is_some_and(|d| started.elapsed() >= d) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// "us.zoom.xos (zoom.us, pid 812)".
fn describe(user: &MicUser) -> String {
    let name = process_name(user.pid);
    let mine = if user.pid == i32::try_from(std::process::id()).unwrap_or(-1) {
        ", this process"
    } else {
        ""
    };
    format!(
        "{} ({}pid {}{mine})",
        user.bundle_id.as_deref().unwrap_or("(no bundle id)"),
        name.map_or(String::new(), |n| format!("{n}, ")),
        user.pid
    )
}

/// The executable's name, from `ps` (a diagnostic only).
fn process_name(pid: i32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    (!name.is_empty()).then_some(name)
}
