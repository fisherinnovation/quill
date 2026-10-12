import { useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  api,
  clock,
  errorText,
  fileManager,
  minutes,
  type MeetingDetail,
  type Progress,
  type RecordingState,
  turns,
} from "../api";
import { useElapsed } from "./RecordButton";
import { Icon } from "./Icon";
import { Banner } from "./ui";

interface Props {
  id: string;
  /** Transcript position to scroll to (from a search hit). */
  jumpTo?: number;
  /** Changes when the meeting's data may have changed; triggers a refetch. */
  version: number;
  /** Set while this meeting is being recorded. */
  recording: RecordingState | null;
  progress?: Progress;
  onChanged: () => void;
  onDeleted: () => void;
}

export function MeetingView({ id, jumpTo, version, recording, progress, onChanged, onDeleted }: Props) {
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"summary" | "transcript">(jumpTo !== undefined ? "transcript" : "summary");
  const [copied, setCopied] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [exported, setExported] = useState<{ path: string } | { error: string } | null>(null);

  useEffect(() => {
    api
      .getMeeting(id)
      .then(setDetail)
      .catch((e) => setError(errorText(e)));
  }, [id, version]);

  if (error) {
    return (
      <div className="page">
        <Banner kind="error" icon="alert">
          {error}
        </Banner>
      </div>
    );
  }
  if (!detail) return null;

  const { meeting, segments, summaries } = detail;
  const summary = summaries[0];
  const busy = progress !== undefined && progress.stage !== "done" && progress.stage !== "failed";
  const process = (transcribe: boolean) => void api.processMeeting(id, transcribe).catch((e) => setError(errorText(e)));
  const speakers = new Set(segments.map((s) => s.speaker).filter(Boolean)).size;

  async function copySummary() {
    if (!summary) return;
    await navigator.clipboard.writeText(summary.markdown);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  async function exportSummary() {
    try {
      const path = await api.exportSummary(id);
      if (path) setExported({ path });
    } catch (e) {
      setExported({ error: errorText(e) });
    }
  }

  return (
    <article className="page meeting">
      <header className="meeting-header">
        <div className="meeting-title-row">
          <Title
            value={meeting.title}
            onSave={async (title) => {
              await api.renameMeeting(id, title);
              setDetail({ ...detail, meeting: { ...meeting, title } });
              onChanged();
            }}
          />
          <div className="meeting-actions">
            {confirmDelete ? (
              <div className="confirm">
                Delete meeting and audio?
                <button
                  className="danger"
                  onClick={async () => {
                    await api.deleteMeeting(id);
                    onDeleted();
                  }}
                >
                  Delete
                </button>
                <button onClick={() => setConfirmDelete(false)}>Cancel</button>
              </div>
            ) : (
              <>
                {!recording && !busy && segments.length === 0 && (
                  <button className="primary" onClick={() => process(true)}>
                    <Icon name="text" size={14} /> Transcribe
                  </button>
                )}
                {!recording && !busy && segments.length > 0 && (
                  <button onClick={() => process(false)} title="Write the summary again">
                    <Icon name={summary ? "refresh" : "sparkle"} size={14} />
                    {summary ? "Re-summarize" : "Summarize"}
                  </button>
                )}
                {summary && (
                  <>
                    <button className="icon-btn" title="Copy summary as Markdown" onClick={copySummary}>
                      <Icon name={copied ? "check" : "copy"} size={15} />
                    </button>
                    <button className="icon-btn" title="Export summary…" onClick={exportSummary}>
                      <Icon name="download" size={15} />
                    </button>
                  </>
                )}
                {!recording && (
                  <button className="icon-btn ghost" title="Delete meeting" onClick={() => setConfirmDelete(true)}>
                    <Icon name="trash" size={15} />
                  </button>
                )}
              </>
            )}
          </div>
        </div>
        <div className="meeting-meta">
          <span>{longDate(meeting.started_at)}</span>
          {meeting.duration_ms > 0 && <span className="dot-sep">{minutes(meeting.duration_ms)}</span>}
          {speakers > 1 && <span className="dot-sep">You and others</span>}
        </div>
        <div className="tabs" role="tablist">
          <button role="tab" aria-selected={tab === "summary"} onClick={() => setTab("summary")}>
            <Icon name="sparkle" size={14} /> Summary
          </button>
          <button role="tab" aria-selected={tab === "transcript"} onClick={() => setTab("transcript")}>
            <Icon name="text" size={14} /> Transcript
            {segments.length > 0 && <span className="tab-count">{turns(segments).length}</span>}
          </button>
        </div>
      </header>

      {exported &&
        ("path" in exported ? (
          <Banner
            kind="info"
            icon="check"
            action={
              <>
                <button onClick={() => void api.revealFile(exported.path)}>Show in {fileManager}</button>
                <button className="ghost icon-btn" aria-label="Dismiss" onClick={() => setExported(null)}>
                  <Icon name="x" size={14} />
                </button>
              </>
            }
          >
            Saved <strong>{exported.path.split(/[\\/]/).pop()}</strong>
          </Banner>
        ) : (
          <Banner
            kind="error"
            icon="alert"
            action={
              <button className="ghost" onClick={() => setExported(null)}>
                Dismiss
              </button>
            }
          >
            Export failed: {exported.error}
          </Banner>
        ))}
      {recording && <RecordingBanner state={recording} />}
      {progress && progress.stage !== "done" && (
        <ProgressBanner progress={progress} onRetry={() => process(segments.length === 0)} />
      )}

      {tab === "summary" &&
        (summary ? (
          <section className="summary">
            <Markdown remarkPlugins={[remarkGfm]}>{summary.markdown}</Markdown>
            <footer className="summary-footer">
              <span>Written by {summary.model.split("/").pop()}</span>
              <span className="dot-sep">{summary.prompt_version}</span>
              {summary.cost_usd != null && <span className="dot-sep">${summary.cost_usd.toFixed(4)}</span>}
              <span className="dot-sep">{(summary.latency_ms / 1000).toFixed(1)} s</span>
            </footer>
          </section>
        ) : (
          <Placeholder
            icon="sparkle"
            title={recording ? "Notes come after the call" : busy ? "Working on it…" : "No summary yet"}
            text={
              recording
                ? "When you stop recording, Quill transcribes the call on this Mac and writes the summary."
                : segments.length > 0
                  ? "Summarize the transcript to get decisions, action items and key points."
                  : "Transcribe this recording first."
            }
          />
        ))}

      {tab === "transcript" && <Transcript segments={segments} jumpTo={jumpTo} recording={!!recording} />}
    </article>
  );
}

function Placeholder({ icon, title, text }: { icon: "sparkle" | "text"; title: string; text: string }) {
  return (
    <div className="placeholder">
      <div className="placeholder-icon">
        <Icon name={icon} size={20} />
      </div>
      <strong>{title}</strong>
      <p className="muted" style={{ maxWidth: 380 }}>
        {text}
      </p>
    </div>
  );
}

function RecordingBanner({ state }: { state: RecordingState }) {
  const elapsed = useElapsed(state);
  return (
    <Banner kind="recording">
      <span className="row-gap">
        <span className="rec-dot pulsing" /> Recording · {clock(elapsed)}
      </span>
    </Banner>
  );
}

const STAGE_TEXT: Record<Progress["stage"], string> = {
  downloading: "Downloading the speech model",
  transcribing: "Transcribing on this Mac",
  summarizing: "Writing the summary",
  done: "Done",
  failed: "Failed",
};

function ProgressBanner({ progress, onRetry }: { progress: Progress; onRetry: () => void }) {
  if (progress.stage === "failed") {
    return (
      <Banner kind="error" icon="alert" action={<button onClick={onRetry}>Retry</button>}>
        Processing failed: {progress.message}
      </Banner>
    );
  }
  return (
    <div className="banner info">
      <span className="spinner" style={{ color: "var(--grape-text)" }} />
      <div className="banner-text">
        {progress.message ?? STAGE_TEXT[progress.stage]}
        {progress.fraction != null && ` · ${Math.round(progress.fraction * 100)}%`}…
        {progress.fraction != null && (
          <div className="progress">
            <div style={{ width: `${progress.fraction * 100}%` }} />
          </div>
        )}
      </div>
    </div>
  );
}

function Transcript({
  segments,
  jumpTo,
  recording,
}: {
  segments: MeetingDetail["segments"];
  jumpTo?: number;
  recording: boolean;
}) {
  const target = useRef<HTMLDivElement>(null);
  const paragraphs = useMemo(() => turns(segments), [segments]);
  // The turn containing the search hit, if any.
  const targetIndex =
    jumpTo === undefined ? -1 : paragraphs.findIndex((t) => t.start_ms <= jumpTo && jumpTo <= t.end_ms);

  useEffect(() => {
    target.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [targetIndex]);

  if (segments.length === 0) {
    return (
      <Placeholder
        icon="text"
        title={recording ? "Listening…" : "Not transcribed yet"}
        text={
          recording
            ? "The transcript appears here after you stop recording."
            : "Transcription runs on this Mac; the audio never leaves it."
        }
      />
    );
  }
  return (
    <section className="transcript">
      {paragraphs.map((t, i) => {
        const me = t.speaker === "Me";
        return (
          <div key={i} ref={i === targetIndex ? target : undefined} className={`turn${i === targetIndex ? " highlight" : ""}`}>
            <div className={`avatar ${me ? "me" : "others"}`}>{me ? "Me" : (t.speaker ?? "?").slice(0, 1)}</div>
            <div>
              <div className="turn-head">
                <span className="turn-speaker">{me ? "You" : (t.speaker ?? "Speaker")}</span>
                <span className="turn-time">{clock(t.start_ms)}</span>
              </div>
              <p className="turn-text">{t.text}</p>
            </div>
          </div>
        );
      })}
    </section>
  );
}

/** "Saturday, October 11 at 2:15 PM" in the user's locale. */
function longDate(rfc3339: string): string {
  const date = new Date(rfc3339);
  const day = date.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
  const time = date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return `${day} at ${time}`;
}

/** Click-to-edit title. Enter saves, Escape cancels. */
function Title({ value, onSave }: { value: string; onSave: (title: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);

  if (!editing) {
    return (
      <h1 className="title" title="Click to rename" onClick={() => setEditing(true)}>
        {value}
      </h1>
    );
  }
  const finish = async (save: boolean) => {
    if (save && draft.trim() && draft !== value) await onSave(draft.trim());
    setEditing(false);
  };
  return (
    <input
      className="title-input"
      autoFocus
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => void finish(true)}
      onKeyDown={(e) => {
        if (e.key === "Enter") void finish(true);
        if (e.key === "Escape") {
          setDraft(value);
          void finish(false);
        }
      }}
    />
  );
}
