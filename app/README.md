# app/

Quill's desktop app: Tauri v2 (Rust, `src-tauri/`) with a React + TypeScript
UI (`src/`, built by Vite). All the work happens in the library crates in
`crates/`; the UI only displays state and calls the commands in
`src-tauri/src/commands.rs`.

```sh
cd app
npm install
npm run app        # run with hot reload (scripts/dev.sh: no keychain prompts on rebuilds)
npm run app:bundle # build ~/Applications/Quill.app and open it (needed for dictation's paste)
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

## Permissions while developing

`npm run app` runs a bare executable, which macOS won't let you add to
Privacy & Security → Accessibility (needed for dictation to paste). To test
dictation, use `npm run app:bundle`. It builds a real `Quill.app` into
`~/Applications`, signs it and opens it.

macOS remembers permissions by code signature. To keep them across rebuilds,
create a signing certificate once: Keychain Access → Certificate Assistant →
Create a Certificate…, name it **Quill Development**, set Identity Type to
*Self-Signed Root* and Certificate Type to *Code Signing*. `app:bundle`
signs with it whenever it exists. Without it, builds are signed ad hoc, and
after each rebuild you must remove Quill from Accessibility and add it again.

To try the UI in a browser with sample data, run `npm run dev` and open
http://localhost:1420 (flags: `?onboarding`, `?recording`, `?listening`,
`?processing`, `?failed`, `?nopaste`, `?empty`, `?light`).
