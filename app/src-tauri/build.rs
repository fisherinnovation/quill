// Tauri's build step: reads tauri.conf.json, generates the app context
// (icons, config, permissions) that `tauri::generate_context!()` embeds.
//
// Also records which commit the app was built from, for the About window
// and bug reports: QUILL_COMMIT (short hash) and QUILL_COMMIT_DATE. Both are
// empty outside a git checkout. They update when HEAD moves, so a build with
// uncommitted changes shows the commit it's based on.
use std::process::Command;

fn main() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let commit = git(&["rev-parse", "--short=9", "HEAD"]);
    println!("cargo:rustc-env=QUILL_COMMIT={commit}");
    println!(
        "cargo:rustc-env=QUILL_COMMIT_DATE={}",
        git(&["log", "-1", "--format=%cs"])
    );
    // Rebuild when HEAD moves (a commit, or a checkout).
    let head = git(&["rev-parse", "--git-path", "HEAD"]);
    if !head.is_empty() {
        println!("cargo:rerun-if-changed={head}");
    }
    let refs = git(&["rev-parse", "--git-path", "refs/heads"]);
    if !refs.is_empty() {
        println!("cargo:rerun-if-changed={refs}");
    }
    tauri_build::build();
}
