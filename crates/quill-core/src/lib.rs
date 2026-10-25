//! Core of Quill: domain types (meetings, transcripts, summaries), the
//! meeting and dictation pipelines, and SQLite storage.
//!
//! Wires the other library crates together; front ends (`quill-cli`, the
//! Tauri app) call into this crate rather than into the pieces directly.

pub mod action_items;
pub mod calls;
pub mod corrections;
pub mod dictation;
pub mod insights;
pub mod library;
pub mod live;
pub mod live_meeting;
pub mod llm_choices;
pub mod meeting;
pub mod overview;
pub mod paths;
pub mod processing;
pub mod samples;
pub mod settings;
pub mod speakers;
pub mod stats;
pub mod summary;
pub mod voices;
pub mod wer;
