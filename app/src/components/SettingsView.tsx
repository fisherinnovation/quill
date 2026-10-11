import { useEffect, useState } from "react";
import { api, errorText, fileManager, fileSize, type Settings, type StorageInfo } from "../api";
import { AccessCheck } from "./AccessCheck";
import { Icon } from "./Icon";
import { Group, LANGUAGES, Row, SaveIndicator, Switch, useAutosave } from "./ui";

const STT_MODELS: [string, string][] = [
  ["turbo", "Turbo (recommended)"],
  ["turbo-q5", "Turbo, compact"],
  ["large-v3", "Large v3 (most accurate, slower)"],
  ["medium", "Medium"],
  ["small", "Small"],
  ["base", "Base"],
  ["tiny", "Tiny (fastest)"],
];

export function SettingsView() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const saveState = useAutosave(settings, api.saveSettings);

  useEffect(() => {
    api
      .getSettings()
      .then(setSettings)
      .catch((e) => setError(errorText(e)));
  }, []);

  if (!settings) return error ? <div className="page error-text">{error}</div> : null;

  const update = (patch: Partial<Settings>) => setSettings({ ...settings, ...patch });
  const local = settings.summary_model.startsWith("ollama:");

  return (
    <div className="page settings">
      <div className="page-head">
        <h1 className="page-title">Settings</h1>
        <span className="spacer" />
        <SaveIndicator state={saveState} />
      </div>

      <Group title="Transcription">
        <Row label="Speech model" hint="Runs on this Mac. Bigger models are slower and more accurate.">
          <select value={settings.stt_model} onChange={(e) => update({ stt_model: e.target.value })}>
            {STT_MODELS.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </Row>
        <Row label="Meeting language">
          <select value={settings.language ?? ""} onChange={(e) => update({ language: e.target.value || null })}>
            <option value="">Detect automatically</option>
            {LANGUAGES.map(([code, name]) => (
              <option key={code} value={code}>
                {name}
              </option>
            ))}
          </select>
        </Row>
        <Row label="Skip silence" hint="Stops the speech model from inventing text during quiet stretches.">
          <Switch label="Skip silence" checked={settings.vad} onChange={(vad) => update({ vad })} />
        </Row>
      </Group>

      <Group title="Summaries">
        <Row label="Summarize automatically" hint="Write the notes as soon as a recording is transcribed.">
          <Switch
            label="Summarize automatically"
            checked={settings.auto_summarize}
            onChange={(auto_summarize) => update({ auto_summarize })}
          />
        </Row>
        <Row
          label="Model"
          hint={
            local
              ? "Runs locally with Ollama: nothing leaves this computer."
              : "An OpenRouter model id, or ollama:<model> to stay local. Only the transcript is sent, never audio."
          }
        >
          <input
            value={settings.summary_model}
            spellCheck={false}
            onChange={(e) => update({ summary_model: e.target.value.trim() })}
          />
        </Row>
        {!local && (
          <>
            <ApiKey />
            <Row
              label="Zero data retention only"
              hint="Stricter than the default (providers that don't store or train on data); fewer models qualify."
            >
              <Switch label="Zero data retention only" checked={settings.zdr} onChange={(zdr) => update({ zdr })} />
            </Row>
          </>
        )}
      </Group>

      <Storage />

      <section className="group">
        <h2 className="group-title">Audio access</h2>
        <AccessCheck framed />
      </section>

      <Group title="Troubleshooting">
        <Row
          label="Log files"
          hint="What Quill did and any errors, kept for two weeks. Never includes transcripts, notes or dictated text."
        >
          <button onClick={() => void api.showLogs()}>Show in {fileManager}</button>
        </Row>
      </Group>
    </div>
  );
}

/** Where meeting audio lives. Changes apply immediately. */
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
    <Group title="Storage">
      <div className="row">
        <div className="access-icon">
          <Icon name="folder" size={17} />
        </div>
        <div className="row-label">
          <span>Recordings</span>
          <p className="path">{info?.path ?? "…"}</p>
          {info && (
            <p className="row-hint">
              {info.available
                ? `${fileSize(info.bytes)} · about 70 MB per hour of meeting once transcribed`
                : "Folder not found. Is the drive connected?"}
            </p>
          )}
        </div>
      </div>
      <div className="row">
        <div className="row-label">
          <p className="row-hint" style={{ marginTop: 0 }}>
            {message ?? "Changing the folder moves existing recordings too. Notes and transcripts stay in Quill's library."}
          </p>
        </div>
        <div className="row-control">
          {info && !info.is_default && (
            <button className="ghost" disabled={busy} onClick={() => move(api.resetRecordingsDir)}>
              Use default
            </button>
          )}
          <button
            disabled={busy || !info?.available}
            onClick={() => api.showRecordingsDir().catch((e) => setMessage(errorText(e)))}
          >
            Show in {fileManager}
          </button>
          <button disabled={busy} onClick={() => move(api.chooseRecordingsDir)}>
            {busy ? (
              <>
                <span className="spinner" /> Moving…
              </>
            ) : (
              "Change…"
            )}
          </button>
        </div>
      </div>
    </Group>
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

  const status =
    source === undefined
      ? "Checking…"
      : source === "keychain"
        ? "Stored in your keychain."
        : source === "environment"
          ? "Provided by the QUILL_OPENROUTER_API_KEY environment variable."
          : "Not set. Create one at openrouter.ai/keys.";

  return (
    <div className="row">
      <div className="row-label">
        <span className="row-gap">
          OpenRouter API key
          {source && (
            <span className="pill green">
              <Icon name="lock" size={11} /> Set
            </span>
          )}
        </span>
        <p className="row-hint">{message ?? status}</p>
      </div>
      <div className="row-control">
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder={source ? "Replace key…" : "sk-or-…"}
          value={draft}
          style={{ width: 180 }}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && draft.trim()) void run(() => api.setOpenrouterKey(draft), "Saved to the keychain.");
          }}
        />
        {draft.trim() ? (
          <button
            className="primary"
            disabled={busy}
            onClick={() => run(() => api.setOpenrouterKey(draft), "Saved to the keychain.")}
          >
            Save
          </button>
        ) : (
          source === "keychain" && (
            <button className="ghost" disabled={busy} onClick={() => run(() => api.deleteOpenrouterKey(), "Key removed.")}>
              Remove
            </button>
          )
        )}
      </div>
    </div>
  );
}
