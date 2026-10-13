import { useEffect, useState } from "react";
import { ModelPicker } from "./ModelPicker";
import { ModelsGroup } from "./Models";
import { api, errorText, events, type Footprint, fileManager, fileSize, type Settings, type StorageInfo } from "../api";
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
  // Bumped when models are deleted, so the disk footprint updates.
  const [diskVersion, setDiskVersion] = useState(0);
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
        <Row
          label="Tell speakers apart"
          hint="Labels the people on the call Speaker 1, 2… by their voices, on this Mac. Rename them in the transcript."
        >
          <Switch
            label="Tell speakers apart"
            checked={settings.identify_speakers}
            onChange={(identify_speakers) => update({ identify_speakers })}
          />
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
              ? "Writes the notes on this computer with Ollama."
              : "Via OpenRouter. Only the transcript is sent, never audio."
          }
        >
          <ModelPicker
            purpose="summary"
            value={settings.summary_model}
            onChange={(summary_model) => update({ summary_model })}
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

      <ModelsGroup settings={settings} onChanged={() => setDiskVersion((v) => v + 1)} />

      <Storage version={diskVersion} />

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
const FOOTPRINT_LABELS: Record<Footprint["parts"][number]["key"], string> = {
  recordings: "Meeting audio",
  models: "Models",
  library: "Notes, transcripts and settings",
  logs: "Logs",
  app: "Quill itself",
};

/** Everything Quill keeps on disk, as one number and a bar by part. */
function DiskFootprint({ version }: { version: number }) {
  const [footprint, setFootprint] = useState<Footprint | null>(null);
  useEffect(() => {
    void api.footprint().then(setFootprint).catch(() => setFootprint(null));
  }, [version]);
  // Downloads change it too.
  useEffect(() => {
    const unlisten = events.onModelDownload((p) => {
      if (p.finished) void api.footprint().then(setFootprint);
    });
    return () => void unlisten.then((f) => f());
  }, []);
  if (!footprint) return null;
  const parts = footprint.parts.filter((p) => p.bytes > 0);
  return (
    <div className="row footprint">
      <div className="row-label">
        <span>Quill uses {fileSize(footprint.total)}</span>
        <div className="footprint-bar" role="img" aria-label={`Disk use: ${fileSize(footprint.total)}`}>
          {parts.map((p) => (
            <span
              key={p.key}
              className={`fp-${p.key}`}
              style={{ flexGrow: p.bytes }}
              title={`${FOOTPRINT_LABELS[p.key]}: ${fileSize(p.bytes)}`}
            />
          ))}
        </div>
        <div className="footprint-legend">
          {parts.map((p) => (
            <span key={p.key}>
              <i className={`fp-${p.key}`} />
              {FOOTPRINT_LABELS[p.key]} <b>{fileSize(p.bytes)}</b>
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}

function Storage({ version }: { version: number }) {
  const [info, setInfo] = useState<StorageInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  // Bumped after recordings move.
  const [moves, setMoves] = useState(0);

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
        setMoves((n) => n + 1);
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
      <DiskFootprint version={version + moves} />
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

/**
 * OpenRouter key: kept in the OS keychain, never shown again.
 *
 * Opening Settings never touches the keychain: on macOS even checking
 * whether the key is there can ask for your password. Only replacing or
 * removing the key, after clicking Change key…, does.
 */
function ApiKey() {
  const [source, setSource] = useState<"environment" | "keychain" | "unknown" | "none" | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .openrouterKeyStatus()
      .then(setSource)
      .catch((e) => setMessage(errorText(e)));
  }, []);

  async function save() {
    if (!draft.trim()) return;
    setBusy(true);
    setMessage(null);
    try {
      await api.setOpenrouterKey(draft);
      setDraft("");
      setSource("keychain");
      setEditing(false);
      setMessage("Saved to your keychain.");
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    setMessage(null);
    try {
      const removed = await api.deleteOpenrouterKey();
      setSource("none");
      setEditing(false);
      setMessage(removed ? "Key removed from your keychain." : "There was no key in your keychain.");
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  const status =
    source === "environment"
      ? "Provided by the QUILL_OPENROUTER_API_KEY environment variable."
      : source === "keychain"
        ? "Stored in your keychain."
        : source === "none"
          ? "Not set. Create one at openrouter.ai/keys."
          : "Kept in your keychain once added. Quill reads it only to write notes or tidy dictation.";

  if (editing) {
    return (
      <div className="row">
        <div className="row-label">
          <span>OpenRouter API key</span>
          <p className="row-hint">
            {message ?? "Paste a new key to replace the current one. macOS may ask for your password."}
          </p>
        </div>
        <div className="row-control">
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            autoFocus
            placeholder="sk-or-…"
            value={draft}
            style={{ width: 180 }}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void save();
              if (e.key === "Escape") setEditing(false);
            }}
          />
          <button className="primary" disabled={busy || !draft.trim()} onClick={() => void save()}>
            Save
          </button>
          <button className="ghost" disabled={busy} onClick={() => void remove()} title="Remove the key from your keychain">
            Remove
          </button>
          <button className="ghost" disabled={busy} onClick={() => setEditing(false)}>
            Cancel
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="row">
      <div className="row-label">
        <span className="row-gap">
          OpenRouter API key
          {(source === "keychain" || source === "environment") && (
            <span className="pill green">
              <Icon name="lock" size={11} /> Set
            </span>
          )}
        </span>
        <p className="row-hint">{message ?? status}</p>
      </div>
      {source !== "environment" && (
        <div className="row-control">
          <button
            onClick={() => {
              setMessage(null);
              setEditing(true);
            }}
          >
            {source === "none" ? "Add key…" : "Change key…"}
          </button>
        </div>
      )}
    </div>
  );
}
