// Tauri's build step: reads tauri.conf.json, generates the app context
// (icons, config, permissions) that `tauri::generate_context!()` embeds.
fn main() {
    tauri_build::build();
}
