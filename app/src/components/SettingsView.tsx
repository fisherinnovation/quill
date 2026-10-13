import { useEffect, useState } from "react";
import { ModelsGroup } from "./Models";
import {
  api,
  errorText,
  events,
  type Footprint,
  fileManager,
  fileSize,
  isMac,
  type KeySource,
  keyStore,
  type Settings,
  type StorageInfo,
} from "../api";
import { AccessCheck } from "./AccessCheck";
import { Icon } from "./Icon";
import { Group, LANGUAGES, Row, SaveIndicator, Switch, useAutosave } from "./ui";


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
  // What sends text to OpenRouter right now (the rest runs on Ollama).
  const viaOpenRouter = [
    !settings.summary_model.startsWith("ollama:") && "meeting notes",
    settings.dictation.polish && !settings.dictation.polish_model.startsWith("ollama:") && "dictation clean-up",
  ].filter((x): x is string => typeof x === "string");

  return (
    <div className="page settings">
      <div className="page-head">
        <h1 className="page-title">Settings</h1>
        <span className="spacer" />
        <SaveIndicator state={saveState} />
      </div>

      <Group title="Transcription">
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
      </Group>

      <Group title="OpenRouter">
        <Row
          label="Used for"
          hint={
            viaOpenRouter.length > 0
              ? `${viaOpenRouter.join(" and ").replace(/^./, (c) => c.toUpperCase())}, as set under Models below. Only text is sent, never audio, and only to providers that don't store or train on it.`
              : "Nothing right now: meeting notes and dictation clean-up both run on this computer with Ollama."
          }
        />
        <ApiKey />
        <Row
          label="Zero data retention only"
          hint="Stricter than the default (providers that don't store or train on data); fewer models qualify."
        >
          <Switch label="Zero data retention only" checked={settings.zdr} onChange={(zdr) => update({ zdr })} />
        </Row>
      </Group>

      <ModelsGroup settings={settings} update={update} onChanged={() => setDiskVersion((v) => v + 1)} />

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

/** Recordings with nothing said in them, to delete in one go. */
function EmptyRecordings({ onDeleted }: { onDeleted: () => void }) {
  const [empty, setEmpty] = useState<{ count: number; bytes: number } | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    void api.emptyRecordings().then(setEmpty);
  }, []);

  if (message) {
    return (
      <div className="row">
        <div className="row-label">
          <p className="row-hint" style={{ marginTop: 0 }}>
            {message}
          </p>
        </div>
      </div>
    );
  }
  if (!empty || empty.count === 0) return null;
  const what = empty.count === 1 ? "1 recording" : `${empty.count} recordings`;
  return (
    <Row
      label={`${what} with nothing said`}
      hint={
        confirming
          ? `Deletes ${empty.count === 1 ? "it" : "them"} and the audio (${fileSize(empty.bytes)}). This can't be undone.`
          : `Quill heard no speech in ${empty.count === 1 ? "it" : "them"}. They take ${fileSize(empty.bytes)}.`
      }
    >
      {confirming ? (
        <span className="row-gap">
          <button onClick={() => setConfirming(false)}>Cancel</button>
          <button
            className="danger"
            onClick={async () => {
              try {
                const n = await api.deleteEmptyRecordings();
                setMessage(`Deleted ${n === 1 ? "1 empty recording" : `${n} empty recordings`}.`);
                onDeleted();
              } catch (e) {
                setMessage(errorText(e));
              }
            }}
          >
            Delete {what}
          </button>
        </span>
      ) : (
        <button onClick={() => setConfirming(true)}>Delete…</button>
      )}
    </Row>
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
      <EmptyRecordings onDeleted={() => setMoves((n) => n + 1)} />
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
 * On macOS, opening Settings never touches the keychain: even checking
 * whether the key is there can ask for your password, so its status may be
 * unknown. Only replacing or removing the key does. Windows reports it.
 */
function ApiKey() {
  const [source, setSource] = useState<KeySource | null>(null);
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
      setMessage(`Saved to your ${keyStore}.`);
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
      setMessage(removed ? `Key removed from your ${keyStore}.` : `There was no key in your ${keyStore}.`);
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
        ? `Stored in your ${keyStore}.`
        : source === "none"
          ? "Meeting notes and dictation clean-up with OpenRouter models need one."
          : `Kept in your ${keyStore} once added. Quill reads it only to write notes or tidy dictation.`;

  if (editing) {
    return (
      <div className="row">
        <div className="row-label">
          <span>API key</span>
          <p className="row-hint">
            {message ??
              (source === "none"
                ? `Paste your OpenRouter key. It's kept in your ${keyStore}.`
                : `Paste a new key to replace the current one.${isMac ? " macOS may ask for your password." : ""}`)}
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
          {source !== "none" && (
            <button className="ghost" disabled={busy} onClick={() => void remove()} title={`Remove the key from your ${keyStore}`}>
              Remove
            </button>
          )}
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
          API key
          {(source === "keychain" || source === "environment") && (
            <span className="pill green">
              <Icon name="lock" size={11} /> Set
            </span>
          )}
          {source === "none" && <span className="pill amber">Not set</span>}
        </span>
        <p className="row-hint">{message ?? status}</p>
      </div>
      {source !== "environment" && (
        <div className="row-control">
          {source === "none" && (
            <button className="link" onClick={() => void api.openLink("openrouter_keys")}>
              Get a key <Icon name="external" size={12} />
            </button>
          )}
          <button
            className={source === "none" ? "primary" : undefined}
            onClick={() => {
              setMessage(null);
              setEditing(true);
            }}
          >
            {source === "none" ? "Add key…" : source === "unknown" ? "Add or change key…" : "Change key…"}
          </button>
        </div>
      )}
    </div>
  );
}
