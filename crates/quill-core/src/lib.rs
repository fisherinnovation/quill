//! Core of Quill: domain types (meetings, transcripts, summaries), the
//! meeting and dictation pipelines, and SQLite storage.
//!
//! Wires the other library crates together; front ends (`quill-cli`, the
//! Tauri app) call into this crate rather than into the pieces directly.
