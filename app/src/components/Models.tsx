import { type ReactNode, useEffect, useState } from "react";
import {
  api,
  type DownloadProgress,
  errorText,
  events,
  fileManager,
  fileSize,
  type ModelEntry,
  type ModelsInfo,
  type Settings,
} from "../api";
import { Icon } from "./Icon";
import { ModelPicker } from "./ModelPicker";
import { Banner, Group, Row } from "./ui";

/**
 * The downloaded models, with live download progress. Refetches when a
 * download finishes.
 */
export function useModels() {
  const [info, setInfo] = useState<ModelsInfo | null>(null);
  const [progress, setProgress] = useState<Record<string, DownloadProgress>>({});
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    api
      .modelsInfo()
      .then(setInfo)
      .catch((e) => setError(errorText(e)));

  useEffect(() => {
    void refresh();
    const unlisten = events.onModelDownload((p) => {
      setProgress((all) => ({ ...all, [p.name]: p }));
      if (p.finished) {
        if (p.error) setError(`Couldn't download ${p.name}: ${p.error}`);
        void refresh();
      }
    });
    return () => void unlisten.then((f) => f());
  }, []);

  /** In-flight progress for a model, 0–1, or null if it isn't downloading. */
  const fraction = (m: ModelEntry): number | null => {
    const p = progress[m.name];
    if (p && !p.finished) return p.total > 0 ? p.done / p.total : 0;
    return m.downloading ? 0 : null;
  };

  return { info, setInfo, refresh, fraction, error, setError };
}

/** Whisper sizes, as offered for speech recognition. */
const SPEECH_LABELS: Record<string, string> = {
  turbo: "Turbo (recommended)",
  "turbo-q5": "Turbo, compact",
  "large-v3": "Large v3 (most accurate, slower)",
  medium: "Medium",
  small: "Small",
  base: "Base",
  tiny: "Tiny (fastest)",
};
const SPEECH_ORDER = ["turbo", "turbo-q5", "large-v3", "medium", "small", "base", "tiny"];

type Pending = { role: string; action: "delete" | "redownload" } | { unused: true } | { all: true } | null;

/**
 * Settings → Models, organised by job: each thing Quill does that needs a
 * model has a row, showing which model does it and whether it's on this
 * computer. Choosing a model that isn't downloaded downloads it; deleting
 * one leaves the row, marked Not downloaded. Downloads no job uses are
 * gathered at the end to clear out.
 */
export function ModelsGroup({
  settings,
  update,
  onChanged,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
  onChanged?: () => void;
}) {
  const { info, setInfo, fraction, error, setError } = useModels();
  const [pending, setPending] = useState<Pending>(null);
  if (!info) return null;

  const byName = (name: string) => info.models.find((m) => m.name === name);
  const speechModels = SPEECH_ORDER.map(byName).filter((m): m is ModelEntry => !!m);
  const speech = byName(settings.stt_model);
  const vad = info.models.filter((m) => m.kind === "vad");
  const speakers = info.models.filter((m) => m.for_speakers);
  // Downloaded, but no job uses them (other Whisper sizes, voice models
  // from comparisons).
  const unused = info.models.filter(
    (m) =>
      m.downloaded &&
      fraction(m) === null &&
      ((m.kind === "speech" && m.name !== settings.stt_model) || (m.kind === "speakers" && !m.for_speakers)),
  );
  const unusedBytes = unused.reduce((n, m) => n + m.size_bytes, 0);

  async function run(action: () => Promise<ModelsInfo>) {
    setPending(null);
    try {
      setInfo(await action());
      setError(null);
      onChanged?.();
    } catch (e) {
      setError(errorText(e));
    }
  }

  /** Runs `action` on each model in turn, keeping the last result. */
  const each = (models: ModelEntry[], action: (name: string) => Promise<ModelsInfo>) => async () => {
    let last = info;
    for (const m of models) last = await action(m.name);
    return last;
  };

  const confirm = (text: string, label: string, onYes: () => void) => (
    <span className="model-confirm">
      <span className="row-hint">{text}</span>
      <span className="row-gap">
        <button className="danger" onClick={onYes}>
          {label}
        </button>
        <button onClick={() => setPending(null)}>Cancel</button>
      </span>
    </span>
  );

  /**
   * A job done on this computer: its model files, status and actions.
   * Called as a function, not rendered as a component, so its select
   * isn't remounted (and closed) on every progress update.
   */
  function localJob({
    id,
    label,
    hint,
    files,
    enabled,
    offText,
    picker,
  }: {
    id: string;
    label: string;
    hint: string;
    files: ModelEntry[];
    enabled: boolean;
    offText: string;
    picker?: ReactNode;
  }) {
    const size = fileSize(files.reduce((n, m) => n + m.size_bytes, 0));
    const fractions = files.map(fraction);
    const downloading = fractions.some((f) => f !== null);
    const done = files.reduce((n, m, i) => n + (m.downloaded ? 1 : (fractions[i] ?? 0)) * m.size_bytes, 0);
    const total = files.reduce((n, m) => n + m.size_bytes, 0);
    const ready = files.length > 0 && files.every((m) => m.downloaded);
    const asking = pending && "role" in pending && pending.role === id ? pending.action : null;
    const state = !enabled ? "off" : downloading ? "downloading" : ready ? "ready" : "missing";

    let status: ReactNode;
    if (asking === "delete") {
      status = confirm(
        `Deletes ${size}. Quill downloads it again the next time it's needed.`,
        "Delete",
        () => void run(each(files, api.deleteModel)),
      );
    } else if (asking === "redownload") {
      status = confirm(
        `Replaces it with a fresh copy (${size}), checked against its published hash.`,
        "Re-download",
        () => void run(each(files, api.redownloadModel)),
      );
    } else if (state === "downloading") {
      const pct = Math.round((done / Math.max(total, 1)) * 100);
      status = (
        <span className="model-progress">
          Downloading… {pct}% of {size}
          <span className="progress">
            <span style={{ width: `${pct}%` }} />
          </span>
        </span>
      );
    } else if (state === "ready") {
      status = (
        <span className="job-status">
          <span className="model-status in-use">Downloaded</span> {size}
          <button className="link-button" onClick={() => setPending({ role: id, action: "redownload" })}>
            Re-download
          </button>
          <button className="link-button" onClick={() => setPending({ role: id, action: "delete" })}>
            Delete
          </button>
        </span>
      );
    } else if (state === "missing") {
      status = (
        <span className="job-status">
          <span className="model-status missing">Not downloaded</span> {size}
          <button
            className="link-button"
            onClick={() => void Promise.all(files.filter((m) => !m.downloaded).map((m) => api.downloadModel(m.name)))}
          >
            Download now
          </button>
        </span>
      );
    } else {
      status = (
        <span className="job-status">
          <span className="model-status unused">Off</span> {offText}
        </span>
      );
    }

    return (
      <div className={`row stack model-job ${state}`}>
        <div className="row-label">
          <span>{label}</span>
          <p className="row-hint">{hint}</p>
        </div>
        <div className="model-job-body">
          {picker}
          <div className="model-job-status">{status}</div>
        </div>
      </div>
    );
  }

  return (
    <Group title="Models">
      {error && (
        <div className="row">
          <span className="error-text small">{error}</span>
        </div>
      )}

      {localJob({
        id: "speech",
        label: "Speech recognition",
        hint: "Turns meetings and dictation into text, on this computer. Bigger models are slower and more accurate.",
        files: speech ? [speech] : [],
        enabled: true,
        offText: "",
        picker: (
          <select
            value={settings.stt_model}
            onChange={(e) => {
              const next = byName(e.target.value);
              update({ stt_model: e.target.value });
              // Fetch it now rather than at the next meeting or dictation.
              if (next && !next.downloaded && fraction(next) === null) void api.downloadModel(next.name);
            }}
          >
            {speechModels.map((m) => (
              <option key={m.name} value={m.name}>
                {SPEECH_LABELS[m.name] ?? m.name} · {fileSize(m.size_bytes)}
                {m.downloaded ? " · downloaded" : ""}
              </option>
            ))}
          </select>
        ),
      })}
      {localJob({
        id: "vad",
        label: "Silence detection",
        hint: "Silero VAD: skips quiet stretches so the speech model doesn't invent text.",
        files: vad,
        enabled: settings.vad,
        offText: "Turn on Skip silence above to use it.",
      })}
      {localJob({
        id: "speakers",
        label: "Telling speakers apart",
        hint: "pyannote segmentation and NVIDIA TitaNet voice fingerprints, on this computer.",
        files: speakers,
        enabled: settings.identify_speakers,
        offText: "Turn on Tell speakers apart above to use it.",
      })}

      <div className="row stack model-job">
        <div className="row-label">
          <span>Meeting notes</span>
          <p className="row-hint">
            {settings.summary_model.startsWith("ollama:")
              ? "Writes the notes on this computer with Ollama."
              : "Via OpenRouter. Only the transcript is sent, never audio."}
          </p>
        </div>
        <div className="model-job-body">
          <ModelPicker
            purpose="summary"
            value={settings.summary_model}
            onChange={(summary_model) => update({ summary_model })}
          />
        </div>
      </div>
      <div className="row stack model-job">
        <div className="row-label">
          <span>Dictation clean-up</span>
          <p className="row-hint">
            {settings.dictation.polish
              ? "Tidies dictated text when it needs it. Only the text is sent, never audio."
              : "Clean-up is off. Turn it on on the Dictation page."}
          </p>
        </div>
        <div className="model-job-body">
          <ModelPicker
            purpose="cleanup"
            value={settings.dictation.polish_model}
            onChange={(polish_model) => {
              const dictation = { ...settings.dictation, polish_model };
              update({ dictation });
              void api.saveDictationSettings(dictation).catch((e) => setError(errorText(e)));
            }}
          />
        </div>
      </div>

      {unused.length > 0 && (
        <Row
          label="Other downloaded models"
          hint={
            pending && "unused" in pending
              ? confirm(
                  `Deletes ${unused.map((m) => m.name).join(", ")} (${fileSize(unusedBytes)}).`,
                  "Delete",
                  () => void run(each(unused, api.deleteModel)),
                )
              : `Nothing uses ${unused.length === 1 ? "this" : "these"}: ${unused
                  .map((m) => `${m.name} (${fileSize(m.size_bytes)})`)
                  .join(", ")}.`
          }
        >
          {!(pending && "unused" in pending) && (
            <button onClick={() => setPending({ unused: true })}>Delete {fileSize(unusedBytes)}</button>
          )}
        </Row>
      )}

      <Row
        label={`${fileSize(info.bytes_on_disk)} on disk`}
        hint={
          pending && "all" in pending
            ? confirm(
                "Deletes every downloaded model. Those Quill uses download again the next time they're needed.",
                "Delete all",
                () => void run(api.deleteAllModels),
              )
            : "Models are shared with quill-cli and kept when Quill updates."
        }
      >
        {!(pending && "all" in pending) && (
          <span className="row-gap">
            <button onClick={() => void api.showModelsDir()}>Show in {fileManager}</button>
            {info.bytes_on_disk > 0 && (
              <button className="ghost" onClick={() => setPending({ all: true })}>
                Delete all…
              </button>
            )}
          </span>
        )}
      </Row>
    </Group>
  );
}

/**
 * Downloads what the settings need, with progress: in onboarding, and on
 * the Dictation page when the speech model is missing. Renders nothing once
 * everything is there.
 */
export function ModelDownload({ variant }: { variant: "onboarding" | "banner" }) {
  const { info, fraction } = useModels();
  // Every model that was missing while this was on screen, so progress
  // counts the whole set rather than restarting as each one finishes, and
  // onboarding can say when it's done instead of just disappearing.
  const [plan, setPlan] = useState<string[]>([]);
  const relevant = (info?.models ?? []).filter(
    // Dictation only needs Whisper and VAD; onboarding fetches everything.
    (m) => m.in_use && (variant === "onboarding" || m.kind !== "speakers"),
  );
  const missingNames = relevant.filter((m) => !m.downloaded).map((m) => m.name);
  const missingKey = missingNames.join(",");
  useEffect(() => {
    if (missingKey) setPlan((p) => [...new Set([...p, ...missingKey.split(",")])]);
  }, [missingKey]);
  if (!info) return null;

  if (missingNames.length === 0) {
    return variant === "onboarding" && plan.length > 0 ? (
      <div className="model-download">
        <Icon name="check" size={16} />
        <div>
          <strong>Speech models ready</strong>
        </div>
      </div>
    ) : null;
  }

  const planned = relevant.filter((m) => plan.includes(m.name) || !m.downloaded);
  const total = planned.reduce((n, m) => n + m.size_bytes, 0);
  const done = planned.reduce((n, m) => n + (m.downloaded ? 1 : (fraction(m) ?? 0)) * m.size_bytes, 0);
  const downloading = planned.some((m) => fraction(m) !== null);
  const percent = Math.round((done / Math.max(total, 1)) * 100);
  const start = () => void api.downloadNeededModels();

  if (variant === "banner") {
    return (
      <Banner
        kind="info"
        icon="download"
        action={downloading ? undefined : <button onClick={start}>Download ({fileSize(total)})</button>}
      >
        {downloading ? (
          <div className="banner-text">
            Downloading the speech model · {percent}%. Dictation works as soon as it's done.
            <div className="progress">
              <div style={{ width: `${percent}%` }} />
            </div>
          </div>
        ) : (
          "Dictation needs the speech model, which runs on this computer."
        )}
      </Banner>
    );
  }
  return (
    <div className="model-download">
      <div>
        <strong>Download the speech models</strong>
        <p className="muted small">
          {fileSize(total)}, from Hugging Face, checked before use. They run on this computer, so audio never
          leaves it. {downloading && "You can keep going while this finishes."}
        </p>
        {downloading && (
          <div className="progress">
            <div style={{ width: `${percent}%` }} />
          </div>
        )}
      </div>
      {downloading ? (
        <span className="muted small">{percent}%</span>
      ) : (
        <button className="primary" onClick={start}>
          <Icon name="download" size={14} /> Download
        </button>
      )}
    </div>
  );
}
