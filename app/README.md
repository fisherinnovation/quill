# app/

Quill's desktop app: Tauri v2 (Rust, `src-tauri/`) with a React + TypeScript
UI (`src/`, built by Vite). All the work happens in the library crates in
`crates/`; the UI only displays state and calls the commands in
`src-tauri/src/commands.rs`.

```sh
cd app
npm install
npm run app       # run with hot reload (scripts/dev.sh: no keychain prompts on rebuilds)
npx tauri build   # release bundle (.app / .dmg on macOS, .msi / .exe on Windows)
```

The app and `quill-cli` share one meeting library in the data directory
(`~/Library/Application Support/com.fisherinnovation.Quill` on macOS). Audio
lives in its `recordings/` folder unless moved in Settings → Storage.

To see the first-run screen again, delete the `onboarded = true` line from
`settings.toml` in the data directory.

`app-icon.svg` is the icon source; regenerate the icon set with
`npx tauri icon app-icon.svg -o src-tauri/icons` (then delete the android/ios
and Square* files, which a desktop app doesn't need).
