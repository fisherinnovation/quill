//! Quill desktop app: a Tauri v2 shell around the library crates.
//!
//! Rust owns all the work (library, recording, transcription, summaries);
//! the React UI in `app/src` only displays state and calls the commands in
//! `commands.rs` through Tauri's IPC.

// On Windows, release builds shouldn't open a console window alongside the
// app. (No effect on macOS.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod dictation;
mod onboarding;
mod pipeline;
mod recorder;
mod storage;
mod tray;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use quill_core::library::Library;
use quill_core::settings::Settings;
use tauri::Manager;
use tracing_subscriber::EnvFilter;

/// Shared state, available to every command via `tauri::State<AppState>`.
pub struct AppState {
    pub data_dir: PathBuf,
    pub library: Library,
    pub settings: Mutex<Settings>,
    /// The recording in progress, if any.
    pub recording: Mutex<Option<recorder::ActiveRecording>>,
    /// Whisper model kept in memory between meetings.
    pub transcriber: Mutex<Option<pipeline::LoadedTranscriber>>,
    /// Meetings currently being transcribed/summarized.
    pub processing: Mutex<HashSet<String>>,
    /// Set once the tray icon exists (in `setup`).
    pub tray: Mutex<Option<tray::Tray>>,
    /// OpenRouter key, read from the keychain once per launch (each read
    /// of an unsigned dev build's item can show a macOS prompt).
    pub openrouter_key: Mutex<Option<String>>,
    /// Set while recordings are moving to a new folder; recording and
    /// processing wait for it.
    pub moving: AtomicBool,
}

// `expect` is allowed here only: if the data directory or database can't be
// opened at startup there's nothing useful the app can do, and the panic
// message says why.
#[allow(clippy::expect_used)]
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,whisper_rs=error")),
        )
        .init();

    let data_dir = quill_core::paths::data_dir().expect("no home directory");
    let settings = Settings::load(&data_dir).unwrap_or_else(|e| {
        tracing::warn!("using default settings: {e}");
        Settings::default()
    });
    let library = Library::open(&data_dir, &settings.recordings_dir(&data_dir))
        .expect("could not open the meeting library");

    tauri::Builder::default()
        .manage(AppState {
            data_dir,
            library,
            settings: Mutex::new(settings),
            recording: Mutex::new(None),
            transcriber: Mutex::new(None),
            processing: Mutex::new(HashSet::new()),
            tray: Mutex::new(None),
            openrouter_key: Mutex::new(None),
            moving: AtomicBool::new(false),
        })
        // Native open/save dialogs, used from Rust only (see storage.rs).
        .plugin(tauri_plugin_dialog::init())
        // The dictation hotkey. The handler runs on the main thread and only
        // forwards to the dictation controller.
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if let Some(dictation) = app.try_state::<dictation::Dictation>() {
                        dictation.on_shortcut(shortcut, event);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let tray = tray::create(app.handle())?;
            *commands::lock(&app.state::<AppState>().tray) = Some(tray);
            app.manage(dictation::Dictation::spawn(app.handle().clone()));
            // Binding the hotkey waits on the main thread, which is busy
            // running this setup, so do it (and the model preload) from
            // another thread.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = handle.state::<dictation::Dictation>().register() {
                    tracing::error!("dictation shortcut: {e}");
                }
                pipeline::preload(&handle);
            });
            Ok(())
        })
        // Closing the window keeps Quill running in the menu bar, so a
        // recording in progress isn't cut off. Quit from the tray menu.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        // `generate_handler!` registers the functions the UI may `invoke()`.
        .invoke_handler(tauri::generate_handler![
            commands::list_meetings,
            commands::get_meeting,
            commands::search_transcripts,
            commands::rename_meeting,
            commands::delete_meeting,
            commands::get_settings,
            commands::save_settings,
            commands::recording_state,
            commands::start_recording,
            commands::stop_recording,
            commands::process_meeting,
            commands::openrouter_key_status,
            commands::set_openrouter_key,
            commands::delete_openrouter_key,
            storage::storage_info,
            storage::choose_recordings_dir,
            storage::reset_recordings_dir,
            storage::show_recordings_dir,
            storage::export_summary,
            storage::reveal_file,
            onboarding::check_audio_access,
            onboarding::open_privacy_settings,
            onboarding::finish_onboarding,
            dictation::dictation_state,
            dictation::save_dictation_settings,
            dictation::list_dictations,
            dictation::delete_dictation,
            dictation::clear_dictations,
            dictation::paste_permission,
            dictation::request_paste_permission,
            dictation::system_language,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Quill");
}
