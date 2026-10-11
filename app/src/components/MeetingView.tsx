import { useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  api,
  clock,
  errorText,
  minutes,
  shortDate,
  type MeetingDetail,
  type Progress,
  type RecordingState,
  turns,
} from "../api";
import { useElapsed } from "./RecordButton";

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

  useEffect(() => {
    api
      .getMeeting(id)
      .then(setDetail)
      .catch((e) => setError(errorText(e)));
  }, [id, version]);

  if (error) return <div className="banner error">{error}</div>;
  if (!detail) return <div className="muted pad">Loading…</div>;

  const { meeting, segments, summaries } = detail;
  const summary = summaries[0];
  const busy = progress !== undefined && progress.stage !== "done" && progress.stage !== "failed";
  const process = (transcribe: boolean) => void api.processMeeting(id, transcribe).catch((e) => setError(errorText(e)));

  async function copySummary() {
    if (!summary) return;
    await navigator.clipboard.writeText(summary.markdown);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <article className="meeting">
      <header className="meeting-header">
        <Title
          value={meeting.title}
          onSave={async (title) => {
            await api.renameMeeting(id, title);
            setDetail({ ...detail, meeting: { ...meeting, title } });
            onChanged();
          }}
        />
        <p className="muted">
          {shortDate(meeting.started_at)} · {minutes(meeting.duration_ms)}
        </p>
        <div className="toolbar">
          <div className="tabs" role="tablist">
            <button role="tab" aria-selected={tab === "summary"} onClick={() => setTab("summary")}>
              Summary
            </button>
            <button role="tab" aria-selected={tab === "transcript"} onClick={() => setTab("transcript")}>
              Transcript
            </button>
          </div>
          <span className="spacer" />
          {!recording && !busy && segments.length === 0 && (
            <button onClick={() => process(true)}>Transcribe</button>
          )}
          {!recording && !busy && segments.length > 0 && (
            <button onClick={() => process(false)}>{summary ? "Re-summarize" : "Summarize"}</button>
          )}
          {summary && <button onClick={copySummary}>{copied ? "Copied ✓" : "Copy as Markdown"}</button>}
          {confirmDelete ? (
            <>
              <span className="small">Delete meeting and audio?</span>
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
            </>
          ) : (
            !recording && (
              <button className="subtle" onClick={() => setConfirmDelete(true)}>
                Delete
              </button>
            )
          )}
        </div>
      </header>

      {recording && <RecordingBanner state={recording} />}
      {progress && progress.stage !== "done" && (
        <ProgressBanner progress={progress} onRetry={() => process(segments.length === 0)} />
      )}

      {tab === "summary" &&
        (summary ? (
          <section className="summary">
            <Markdown remarkPlugins={[remarkGfm]}>{summary.markdown}</Markdown>
            <p className="muted small">
              {summary.model} · {summary.prompt_version} ·{" "}
              {summary.cost_usd != null ? `$${summary.cost_usd.toFixed(4)}` : "cost unknown"} ·{" "}
              {(summary.latency_ms / 1000).toFixed(1)}s
            </p>
          </section>
        ) : (
          <p className="muted pad">No summary yet.</p>
        ))}

      {tab === "transcript" && <Transcript segments={segments} jumpTo={jumpTo} />}
    </article>
  );
}

function RecordingBanner({ state }: { state: RecordingState }) {
  const elapsed = useElapsed(state);
  return (
    <div className="banner recording">
      <span className="rec-dot pulsing" /> Recording · {clock(elapsed)} — the transcript and summary appear here
      after you stop.
    </div>
  );
}

const STAGE_TEXT: Record<Progress["stage"], string> = {
  downloading: "Downloading model",
  transcribing: "Transcribing",
  summarizing: "Writing summary",
  done: "Done",
  failed: "Failed",
};

function ProgressBanner({ progress, onRetry }: { progress: Progress; onRetry: () => void }) {
  if (progress.stage === "failed") {
    return (
      <div className="banner error" role="alert">
        <span>Processing failed: {progress.message}</span>
        <button onClick={onRetry}>Retry</button>
      </div>
    );
  }
  const pct = progress.fraction != null ? ` ${Math.round(progress.fraction * 100)}%` : "";
  return (
    <div className="banner info">
      <span className="spinner" /> {progress.message ?? STAGE_TEXT[progress.stage]}
      {pct}…
    </div>
  );
}

function Transcript({ segments, jumpTo }: { segments: MeetingDetail["segments"]; jumpTo?: number }) {
  const target = useRef<HTMLDivElement>(null);
  const paragraphs = useMemo(() => turns(segments), [segments]);
  // The turn containing the search hit, if any.
  const targetIndex =
    jumpTo === undefined ? -1 : paragraphs.findIndex((t) => t.start_ms <= jumpTo && jumpTo <= t.end_ms);

  useEffect(() => {
    target.current?.scrollIntoView({ block: "center" });
  }, [targetIndex]);

  if (segments.length === 0) return <p className="muted pad">Not transcribed yet.</p>;
  return (
    <section className="transcript">
      {paragraphs.map((t, i) => (
        <div
          key={i}
          ref={i === targetIndex ? target : undefined}
          className={`line${i === targetIndex ? " highlight" : ""}`}
        >
          <span className="time">{clock(t.start_ms)}</span>
          {t.speaker && <span className={`speaker ${t.speaker === "Me" ? "me" : "others"}`}>{t.speaker}</span>}
          <span className="text">{t.text}</span>
        </div>
      ))}
    </section>
  );
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
