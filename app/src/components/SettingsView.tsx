import { useEffect, useState } from "react";
import { api, errorText, fileManager, fileSize, type Settings, type StorageInfo } from "../api";
import { AccessCheck } from "./AccessCheck";

const STT_MODELS = ["tiny", "base", "small", "medium", "turbo-q5", "turbo", "large-v3"];

export function SettingsView() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    api
      .getSettings()
      .then(setSettings)
      .catch((e) => setStatus(errorText(e)));
  }, []);

  if (!settings) return <div className="muted pad">{status ?? "Loading…"}</div>;

  const update = (patch: Partial<Settings>) => {
    setSettings({ ...settings, ...patch });
    setStatus(null);
  };
  const local = settings.summary_model.startsWith("ollama:");

  return (
    <form
      className="settings"
      onSubmit={async (e) => {
        e.preventDefault();
        try {
          await api.saveSettings(settings);
          setStatus("Saved.");
        } catch (err) {
          setStatus(errorText(err));
        }
      }}
    >
      <h1>Settings</h1>

      <fieldset>
        <legend>Transcription</legend>
        <label>
          Whisper model
          <select value={settings.stt_model} onChange={(e) => update({ stt_model: e.target.value })}>
            {STT_MODELS.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
        </label>
        <label>
          Language
          <input
            placeholder="Auto-detect (or e.g. en)"
            value={settings.language ?? ""}
            onChange={(e) => update({ language: e.target.value.trim() || null })}
          />
        </label>
        <label className="check">
          <input type="checkbox" checked={settings.vad} onChange={(e) => update({ vad: e.target.checked })} />
          Skip silence (voice activity detection) — prevents invented text in quiet stretches
        </label>
      </fieldset>

      <fieldset>
        <legend>Summaries</legend>
        <label>
          Model
          <input
            value={settings.summary_model}
            onChange={(e) => update({ summary_model: e.target.value.trim() })}
            spellCheck={false}
          />
        </label>
        <p className="muted small">
          {local
            ? "Runs locally with Ollama — nothing leaves this Mac."
            : "OpenRouter model id. The transcript (never audio) is sent to OpenRouter, routed only to providers that don't store or train on it."}
        </p>
        {!local && <ApiKey />}
        {!local && (
          <label className="check">
            <input type="checkbox" checked={settings.zdr} onChange={(e) => update({ zdr: e.target.checked })} />
            Zero data retention only (stricter; fewer models)
          </label>
        )}
        <label className="check">
          <input
            type="checkbox"
            checked={settings.auto_summarize}
            onChange={(e) => update({ auto_summarize: e.target.checked })}
          />
          Summarize automatically after transcribing
        </label>
      </fieldset>

      <div className="toolbar">
        <button type="submit" className="primary">
          Save
        </button>
        {status && <span className="muted">{status}</span>}
      </div>

      <Storage />

      <fieldset>
        <legend>Audio access</legend>
        <p className="muted small">
          Records a few seconds from your microphone and the system audio, playing a short chime, to confirm Quill can
          hear both.
        </p>
        <AccessCheck />
      </fieldset>
    </form>
  );
}

/** Where meeting audio lives. Changes apply immediately (no Save needed). */
function Storage() {
  const [info, setInfo] = useState<StorageInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    api
      .storageInfo()
      .then(setInfo)
      .catch((e) => setMessage(errorText(e)));
  }, []);

  async function move(action: () => Promise<StorageInfo | null>) {
    setBusy(true);
    setMessage(null);
    try {
      const moved = await action();
      if (moved) {
        setInfo(moved);
        setMessage("Recordings moved.");
      }
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <fieldset>
      <legend>Storage</legend>
      <div className="storage">
        <span>Recordings folder</span>
        <code className="path">{info?.path ?? "…"}</code>
        {info && (
          <p className="muted small">
            {info.available
              ? `${fileSize(info.bytes)} of recordings${info.is_default ? " · default location" : ""}`
              : "Folder not found. Is the drive connected?"}
          </p>
        )}
      </div>
      <div className="toolbar">
        <button type="button" disabled={busy} onClick={() => move(api.chooseRecordingsDir)}>
          {busy ? "Moving…" : "Change…"}
        </button>
        {info && !info.is_default && (
          <button type="button" className="subtle" disabled={busy} onClick={() => move(api.resetRecordingsDir)}>
            Use default
          </button>
        )}
        <button
          type="button"
          className="subtle"
          disabled={busy || !info?.available}
          onClick={() => api.showRecordingsDir().catch((e) => setMessage(errorText(e)))}
        >
          Show in {fileManager}
        </button>
        {message && <span className="muted small">{message}</span>}
      </div>
      <p className="muted small">
        Audio takes about 2 GB per hour of meeting. Changing the folder moves existing recordings too; transcripts and
        summaries stay in Quill&apos;s library.
      </p>
    </fieldset>
  );
}

/** OpenRouter key: stored in the OS keychain by the app, never shown again. */
function ApiKey() {
  const [source, setSource] = useState<string | null | undefined>(undefined);
  const [draft, setDraft] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = () =>
    api
      .openrouterKeyStatus()
      .then(setSource)
      .catch((e) => setMessage(errorText(e)));
  useEffect(() => {
    void refresh();
  }, []);

  async function run(action: () => Promise<void>, done: string) {
    setBusy(true);
    setMessage(null);
    try {
      await action();
      setDraft("");
      setMessage(done);
      await refresh();
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="api-key">
      <span>OpenRouter API key</span>
      <p className="muted small">
        {source === undefined
          ? "Checking…"
          : source === "keychain"
            ? "Stored in your keychain."
            : source === "environment"
              ? "Provided by the QUILL_OPENROUTER_API_KEY environment variable."
              : "Not set. Create one at openrouter.ai/keys."}
      </p>
      <div className="toolbar">
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder={source ? "Replace key…" : "sk-or-…"}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button
          type="button"
          disabled={busy || !draft.trim()}
          onClick={() => run(() => api.setOpenrouterKey(draft), "Key saved to the keychain.")}
        >
          Save key
        </button>
        {source === "keychain" && (
          <button
            type="button"
            className="subtle"
            disabled={busy}
            onClick={() => run(() => api.deleteOpenrouterKey(), "Key removed.")}
          >
            Remove
          </button>
        )}
      </div>
      {message && <p className="muted small">{message}</p>}
    </div>
  );
}
