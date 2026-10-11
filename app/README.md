# app/

The Tauri v2 desktop app (`src-tauri/` + `frontend/`) is scaffolded in **M5**.
Svelte vs React is still an open decision (see `docs/PROJECT_PLAN.md`, section 8).

When it is added, list `app/src-tauri` in the root `Cargo.toml` `members`.
Tauri depends on the library crates in `crates/`, never the other way round.
