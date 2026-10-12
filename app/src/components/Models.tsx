import { useEffect, useState } from "react";
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

const KIND_LABEL: Record<ModelEntry["kind"], string> = {
  speech: "Speech",
  vad: "Silence detection",
  speakers: "Speakers",
};

/** What the (possibly unsaved) settings need, computed here so the list reacts at once. */
function inUse(m: ModelEntry, settings: Settings): boolean {
  if (m.kind === "speech") return m.name === settings.stt_model;
  if (m.kind === "vad") return settings.vad;
  return settings.identify_speakers && m.for_speakers;
}

type Pending = { name: string; action: "delete" | "redownload" } | { all: true } | null;

/**
 * Settings → Models: what's downloaded and needed, with sizes and live
 * progress. Delete or re-download any model, or delete them all.
 */
export function ModelsGroup({ settings, onChanged }: { settings: Settings; onChanged?: () => void }) {
  const { info, setInfo, fraction, error, setError } = useModels();
  // An action waiting for confirmation (deleting a model in use, a
  // re-download, deleting everything).
  const [pending, setPending] = useState<Pending>(null);
  if (!info) return null;

  const shown = info.models.filter((m) => m.downloaded || inUse(m, settings) || fraction(m) !== null);
  const missing = shown.filter((m) => inUse(m, settings) && !m.downloaded && fraction(m) === null);
  const anyDownloaded = info.models.some((m) => m.downloaded);

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

  return (
    <Group title="Models">
      {error && (
        <div className="row">
          <span className="error-text small">{error}</span>
        </div>
      )}
      {shown.map((m) => {
        const f = fraction(m);
        const needed = inUse(m, settings);
        const asking = pending && "name" in pending && pending.name === m.name ? pending.action : null;
        const size = fileSize(m.size_bytes);
        return (
          <Row
            key={m.name}
            label={
              <>
                {m.name} <span className="faint">· {KIND_LABEL[m.kind]}</span>
                {needed && m.downloaded && <span className="model-tag">In use</span>}
              </>
            }
            hint={
              asking === "delete" ? (
                confirm(
                  `Quill uses this model, so it will download it again (${size}) the next time it's needed.`,
                  "Delete",
                  () => void run(() => api.deleteModel(m.name)),
                )
              ) : asking === "redownload" ? (
                confirm(
                  `Deletes this copy and downloads ${size} again, checked against its published hash.`,
                  "Re-download",
                  () => void run(() => api.redownloadModel(m.name)),
                )
              ) : f !== null ? (
                <span className="model-progress">
                  Downloading… {Math.round(f * 100)}% of {size}
                  <span className="progress">
                    <span style={{ width: `${f * 100}%` }} />
                  </span>
                </span>
              ) : (
                `${m.description} · ${size}`
              )
            }
          >
            {asking || f !== null ? null : !m.downloaded ? (
              <button onClick={() => void api.downloadModel(m.name)}>
                <Icon name="download" size={14} /> Download
              </button>
            ) : (
              <span className="row-gap">
                <button
                  className="icon-btn ghost"
                  title="Re-download: replace this copy with a fresh one"
                  aria-label={`Re-download ${m.name}`}
                  onClick={() => setPending({ name: m.name, action: "redownload" })}
                >
                  <Icon name="refresh" size={14} />
                </button>
                <button
                  className="icon-btn ghost"
                  title="Delete"
                  aria-label={`Delete ${m.name}`}
                  onClick={() =>
                    needed ? setPending({ name: m.name, action: "delete" }) : void run(() => api.deleteModel(m.name))
                  }
                >
                  <Icon name="trash" size={14} />
                </button>
              </span>
            )}
          </Row>
        );
      })}
      <Row
        label={`${fileSize(info.bytes_on_disk)} on disk`}
        hint={
          pending && "all" in pending
            ? confirm(
                "Deletes every downloaded model. Those Quill uses download again the next time they're needed.",
                "Delete all",
                () => void run(() => api.deleteAllModels()),
              )
            : missing.length > 0
              ? "Models download automatically the first time they're needed, or download them now."
              : "Models are shared with quill-cli and kept when Quill updates."
        }
      >
        {!(pending && "all" in pending) && (
          <span className="row-gap">
            <button onClick={() => void api.showModelsDir()}>Show in {fileManager}</button>
            {anyDownloaded && (
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
