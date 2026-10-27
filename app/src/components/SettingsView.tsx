import { useEffect, useState } from "react";
import { ModelsGroup } from "./Models";
import {
  api,
  applyTheme,
  type DictationSettings,
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
  type Theme,
} from "../api";
import { AccessCheck } from "./AccessCheck";
import { DictationSettingsTab } from "./DictationSettings";
import { Icon } from "./Icon";
import { KnownVoicesRows } from "./KnownVoices";
import { Group, LANGUAGES, Row, SaveIndicator, type SaveState, Switch, useAutosave } from "./ui";
import { UpdateRows } from "./Updates";

export type SettingsTab = "general" | "meetings" | "dictation" | "models" | "storage";

const THEMES: [Theme, string][] = [
  ["system", "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];

const TABS: [SettingsTab, string][] = [
  ["general", "General"],
  ["meetings", "Meetings"],
  ["dictation", "Dictation"],
  ["models", "Models"],
  ["storage", "Storage"],
];

export function SettingsView({
  tab: initialTab = "general",
  onReviewSamples,
}: {
  tab?: SettingsTab;
  /** Opens the review of saved dictation samples. */
  onReviewSamples?: () => void;
}) {
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  const [settings, setSettings] = useState<Settings | null>(null);
  // Saved on their own (save_dictation_settings rebinds the shortcut;
  // save_settings leaves dictation alone).
  const [dictation, setDictation] = useState<DictationSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Bumped when models are deleted, so the disk footprint updates.
  const [diskVersion, setDiskVersion] = useState(0);
  const saveState = combined(
    useAutosave(settings, api.saveSettings),
    useAutosave(dictation, api.saveDictationSettings),
  );

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setDictation(s.dictation);
      })
      .catch((e) => setError(errorText(e)));
  }, []);

  if (!settings || !dictation) return error ? <div className="page error-text">{error}</div> : null;

  const update = (patch: Partial<Settings>) => {
    const { dictation: d, ...rest } = patch;
    if (d) setDictation(d);
    if (Object.keys(rest).length > 0) setSettings({ ...settings, ...rest });
  };
  const current = { ...settings, dictation };
  // What sends text to OpenRouter right now (the rest runs on Ollama).
  const viaOpenRouter = [
    !settings.summary_model.startsWith("ollama:") && "meeting notes",
    dictation.polish && !dictation.polish_model.startsWith("ollama:") && "dictation clean-up",
  ].filter((x): x is string => typeof x === "string");

  return (
    <div className="page settings">
      <div className="page-head">
        <h1 className="page-title">Settings</h1>
        <span className="spacer" />
        <SaveIndicator state={saveState} />
      </div>
      <div className="tabs settings-tabs" role="tablist">
        {TABS.map(([key, label]) => (
          <button key={key} role="tab" aria-selected={tab === key} onClick={() => setTab(key)}>
            {label}
          </button>
        ))}
      </div>

      {tab === "general" && (
        <>
          <Group title="Appearance">
            <Row
              label="Theme"
              hint={
                settings.theme === "system"
                  ? "Matches your computer, and switches when it does."
                  : `Always ${settings.theme}, whatever your computer uses.`
              }
            >
              <div className="segmented" role="radiogroup" aria-label="Theme">
                {THEMES.map(([theme, label]) => (
                  <button
                    key={theme}
                    role="radio"
                    aria-checked={settings.theme === theme}
                    onClick={() => {
                      // Now, not after the autosave: the other windows follow
                      // once it's saved.
                      applyTheme(theme);
                      update({ theme });
                    }}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </Row>
          </Group>

          <Group title="OpenRouter">
            <Row
              label="Used for"
              hint={
                viaOpenRouter.length > 0
                  ? `${viaOpenRouter.join(" and ").replace(/^./, (c) => c.toUpperCase())}, as set under Models. Only text is sent, never audio, and only to providers that don't store or train on it.`
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

          <section className="group">
            <h2 className="group-title">Audio access</h2>
            <AccessCheck framed />
          </section>

          <Group title="Updates">
            <UpdateRows settings={settings} update={update} />
          </Group>

          <Group title="Troubleshooting">
            <Row
              label="Log files"
              hint="What Quill did and any errors, kept for two weeks. Never includes transcripts, notes or dictated text."
            >
              <button onClick={() => void api.showLogs()}>Show in {fileManager}</button>
            </Row>
          </Group>
        </>
      )}

      {tab === "meetings" && (
        <>
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
        <KnownVoicesRows
          enabled={settings.recognize_voices}
          onChange={(recognize_voices) => update({ recognize_voices })}
        />
      </Group>

      <Group title="Calls">
        <Row
          label="Offer to record calls"
          hint={
            isMac
              ? "When Zoom, Teams, FaceTime, Slack, Discord or a browser starts using your microphone, Quill asks whether to record, and offers to stop when the call ends. It sees only which app is using the mic."
              : "Coming to Windows."
          }
        >
          <Switch
            label="Offer to record calls"
            checked={settings.offer_to_record_calls}
            onChange={(offer_to_record_calls) => update({ offer_to_record_calls })}
          />
        </Row>
      </Group>

      <Group title="Live notes">
        <Row
          label="Live notes during meetings"
          hint="While recording, a panel at the right edge of your screen shows what to ask before the end, follow-ups and decisions, updated every minute. The last 15 minutes of the conversation are sent to your notes model (Settings → Models) each time. The panel is hidden from screen sharing."
        >
          <Switch
            label="Live notes during meetings"
            checked={settings.live_notes}
            onChange={(live_notes) => update({ live_notes })}
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
          label="Tell me when notes are ready"
          hint="A notification when a meeting's notes are ready, or couldn't be written, if Quill isn't in front. It shows the meeting's title."
        >
          <Switch
            label="Tell me when notes are ready"
            checked={settings.notify_notes_ready}
            onChange={(notify_notes_ready) => update({ notify_notes_ready })}
          />
        </Row>
      </Group>

      <Group title="Search">
        <Row
          label="Search by meaning"
          hint="Search also finds moments, notes, action items and dictations that say the same thing in other words, in any of about 100 languages. It uses a 488 MB model that runs on this computer, downloaded when you ask (Settings → Models, or the search window). Nothing you search leaves it. Off, search matches words only."
        >
          <Switch
            label="Search by meaning"
            checked={settings.semantic_search}
            onChange={(semantic_search) => update({ semantic_search })}
          />
        </Row>
      </Group>

      <Group title="AI assistants">
        <Row
          label="Let AI assistants read your meetings"
          hint="An assistant such as Claude Desktop or Claude Code can search your meetings, read their notes and transcripts and see your action items, to answer what you ask it. What it reads is sent to that assistant's provider. Dictations are never shared. Assistants connect through quill-cli mcp (see Help). Turning this off stops them at once."
        >
          <Switch
            label="Let AI assistants read your meetings"
            checked={settings.assistant_access}
            onChange={(assistant_access) => update({ assistant_access })}
          />
        </Row>
      </Group>

        </>
      )}

      {tab === "dictation" && (
        <DictationSettingsTab
          settings={dictation}
          speechModel={dictation.stt_model ?? current.stt_model}
          update={(patch) => setDictation({ ...dictation, ...patch })}
          onReviewSamples={onReviewSamples}
        />
      )}

      {tab === "models" && (
        <ModelsGroup settings={current} update={update} onChanged={() => setDiskVersion((v) => v + 1)} />
      )}

      {tab === "storage" && <Storage version={diskVersion} />}
    </div>
  );
}

/** One indicator for two autosaves: an error first, then saving, then saved. */
function combined(a: SaveState, b: SaveState): SaveState {
  for (const kind of ["error", "saving", "saved"] as const) {
    if (a.kind === kind) return a;
    if (b.kind === kind) return b;
  }
  return a;
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
