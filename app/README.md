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
npx tauri build    # release bundle; see below for the .dmg as CI makes it
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

macOS remembers permissions by the app's designated requirement. `app:bundle`
signs ad hoc with a fixed requirement (Quill's bundle identifier), so
Accessibility and microphone approval survive rebuilds. If a code-signing
certificate named **Quill Development** exists (Keychain Access → Certificate
Assistant → Create a Certificate…, Self-Signed Root, Code Signing), it's used
instead.

A release build like CI's (a `.dmg` in `dist-release/`):

```sh
export SHERPA_ONNX_LIB_DIR="$(../scripts/fetch-sherpa-onnx.sh)"
npx tauri build --bundles app && sh scripts/package-macos.sh
```

To try the UI in a browser with sample data, run `npm run dev` and open
http://localhost:1420 (flags: `?onboarding`, `?recording`, `?listening`,
`?processing`, `?failed`, `?nopaste`, `?empty`, `?light`, `?indicator`,
`?others`, `?nomodels`, `?fn`; see `src/dev/mock.ts`).
