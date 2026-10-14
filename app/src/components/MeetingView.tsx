import { type CSSProperties, useEffect, useMemo, useRef, useState } from "react";
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
import { MeetingStatsView } from "./MeetingStats";

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
  const [tab, setTab] = useState<"summary" | "transcript" | "stats">(jumpTo !== undefined ? "transcript" : "summary");
  // A moment picked on the Stats tab, to show in the transcript.
  const [statsJump, setStatsJump] = useState<number | undefined>(undefined);
  const [copied, setCopied] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [exported, setExported] = useState<{ path: string } | { error: string } | null>(null);
  // When speakers were last renamed or re-split here, to flag older notes.
  const [speakersEditedAt, setSpeakersEditedAt] = useState<number | null>(null);

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
  const people = callSpeakers(segments);
  const notesOutdated =
    summary !== undefined && speakersEditedAt !== null && Date.parse(summary.created_at) < speakersEditedAt;

  async function renameSpeaker(from: string, to: string) {
    await api.renameSpeaker(id, from, to);
    setDetail((d) => d && { ...d, segments: d.segments.map((s) => (s.speaker === from ? { ...s, speaker: to } : s)) });
    setSpeakersEditedAt(Date.now());
  }

  function splitSpeakers(count: number | null) {
    setSpeakersEditedAt(Date.now());
    void api.splitSpeakers(id, count).catch((e) => setError(errorText(e)));
  }

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
                  <button className={meeting.empty ? undefined : "primary"} onClick={() => process(true)}>
                    <Icon name="text" size={14} /> {meeting.empty ? "Transcribe again" : "Transcribe"}
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
          {people.length > 0 && <span className="dot-sep">{peopleText(people)}</span>}
        </div>
        <div className="tabs" role="tablist">
          <button role="tab" aria-selected={tab === "summary"} onClick={() => setTab("summary")}>
            <Icon name="sparkle" size={14} /> Summary
          </button>
          <button role="tab" aria-selected={tab === "transcript"} onClick={() => setTab("transcript")}>
            <Icon name="text" size={14} /> Transcript
            {segments.length > 0 && <span className="tab-count">{turns(segments).length}</span>}
          </button>
          <button role="tab" aria-selected={tab === "stats"} onClick={() => setTab("stats")}>
            <Icon name="chart" size={14} /> Stats
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
      {meeting.empty && !recording && !busy && (
        <Banner
          kind="info"
          icon="info"
          action={
            <>
              <button
                onClick={async () => {
                  await api.keepEmptyRecording(id);
                  setDetail({ ...detail, meeting: { ...meeting, empty: false } });
                  onChanged();
                }}
              >
                Keep
              </button>
              <button
                className="danger"
                onClick={async () => {
                  await api.deleteMeeting(id);
                  onDeleted();
                }}
              >
                Delete recording
              </button>
            </>
          }
        >
          Nothing was said in this recording, so there's nothing to transcribe or summarize.
        </Banner>
      )}
      {progress && progress.stage !== "done" && (
        <ProgressBanner progress={progress} onRetry={() => process(segments.length === 0)} />
      )}

      {tab === "summary" && notesOutdated && !busy && (
        <Banner kind="info" icon="sparkle" action={<button onClick={() => process(false)}>Re-summarize</button>}>
          Speakers have changed since these notes were written.
        </Banner>
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
            title={
              recording
                ? "Notes come after the call"
                : busy
                  ? "Working on it…"
                  : meeting.empty
                    ? "Nothing was said"
                    : "No summary yet"
            }
            text={
              recording
                ? "When you stop recording, Quill transcribes the call on this Mac and writes the summary."
                : meeting.empty
                  ? "Quill didn't hear any speech in this recording."
                  : segments.length > 0
                  ? "Summarize the transcript to get decisions, action items and key points."
                  : "Transcribe this recording first."
            }
          />
        ))}

      {tab === "transcript" && segments.length > 0 && !recording && (
        <SpeakerBar segments={segments} busy={busy} onRename={renameSpeaker} onSplit={splitSpeakers} />
      )}
      {tab === "transcript" && (
        <Transcript
          segments={segments}
          jumpTo={statsJump ?? jumpTo}
          recording={!!recording}
          empty={meeting.empty}
        />
      )}
      {tab === "stats" && (
        <MeetingStatsView
          id={id}
          version={version}
          onJump={(ms) => {
            setStatsJump(ms);
            setTab("transcript");
          }}
        />
      )}
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
  identifying: "Telling the speakers apart",
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
  empty,
}: {
  segments: MeetingDetail["segments"];
  jumpTo?: number;
  recording: boolean;
  /** Transcribed, and nothing was said. */
  empty: boolean;
}) {
  const target = useRef<HTMLDivElement>(null);
  const paragraphs = useMemo(() => turns(segments), [segments]);
  const colors = useMemo(() => speakerColors(segments), [segments]);
  // The turn containing the search hit, if any.
  const targetIndex =
    jumpTo === undefined
      ? -1
      : // The turn containing that moment, or (in a pause) the next one.
        ((i) => (i >= 0 ? i : paragraphs.findIndex((t) => t.start_ms >= jumpTo)))(
          paragraphs.findIndex((t) => t.start_ms <= jumpTo && jumpTo <= t.end_ms),
        );

  useEffect(() => {
    target.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [targetIndex]);

  if (segments.length === 0) {
    return (
      <Placeholder
        icon="text"
        title={recording ? "Listening…" : empty ? "Nothing was said" : "Not transcribed yet"}
        text={
          recording
            ? "The transcript appears here after you stop recording."
            : empty
              ? "Quill didn't hear any speech in this recording."
              : "Transcription runs on this Mac; the audio never leaves it."
        }
      />
    );
  }
  return (
    <section className="transcript">
      {paragraphs.map((t, i) => {
        const me = t.speaker === ME;
        return (
          <div key={i} ref={i === targetIndex ? target : undefined} className={`turn${i === targetIndex ? " highlight" : ""}`}>
            <div className={`avatar ${me ? "me" : "others"}`} style={colorStyle(colors, t.speaker)}>
              {me ? "Me" : initial(t.speaker)}
            </div>
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

const ME = "Me";
const OTHERS = "Others";

interface CallSpeaker {
  name: string;
  talkMs: number;
}

/** Everyone but "Me", in order of first appearance, with how long they talk. */
function callSpeakers(segments: MeetingDetail["segments"]): CallSpeaker[] {
  const byName = new Map<string, CallSpeaker>();
  for (const s of segments) {
    if (!s.speaker || s.speaker === ME) continue;
    const entry = byName.get(s.speaker) ?? { name: s.speaker, talkMs: 0 };
    entry.talkMs += s.end_ms - s.start_ms;
    byName.set(s.speaker, entry);
  }
  return [...byName.values()];
}

/** "You and 3 others", or "You and others" when they weren't told apart. */
function peopleText(people: CallSpeaker[]): string {
  if (people.length === 1 && people[0].name === OTHERS) return "You and others";
  if (people.length === 1) return `You and ${people[0].name.startsWith("Speaker ") ? "1 other" : people[0].name}`;
  return `You and ${people.length} others`;
}

/** Each call speaker's colour slot, by first appearance. */
function speakerColors(segments: MeetingDetail["segments"]): Map<string, number> {
  return new Map(callSpeakers(segments).map((p, i) => [p.name, i]));
}

function colorStyle(colors: Map<string, number>, speaker: string | null | undefined) {
  const slot = speaker ? colors.get(speaker) : undefined;
  return slot === undefined ? undefined : ({ "--spk": `var(--spk-${(slot % 8) + 1})` } as CSSProperties);
}

/** "3" for "Speaker 3", else the first letter of the name. */
function initial(name: string | null | undefined): string {
  if (!name) return "?";
  const numbered = /^Speaker (\d+)$/.exec(name);
  return numbered ? numbered[1] : name.slice(0, 1).toUpperCase();
}

function talkTime(ms: number): string {
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s}s` : `${Math.round(s / 60)}m`;
}

/**
 * The people on the call above the transcript: click one to rename them
 * (typing another speaker's name merges the two), or say how many people
 * there were to split the voices again.
 */
function SpeakerBar({
  segments,
  busy,
  onRename,
  onSplit,
}: {
  segments: MeetingDetail["segments"];
  busy: boolean;
  onRename: (from: string, to: string) => Promise<void>;
  onSplit: (count: number | null) => void;
}) {
  const people = callSpeakers(segments);
  const colors = speakerColors(segments);
  const [editing, setEditing] = useState<string | null>(null);
  const [pending, setPending] = useState<number | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  if (people.length === 0) return null;

  const together = people.length === 1 && people[0].name === OTHERS;
  // Names the user gave would be replaced by a re-split.
  const named = people.some((p) => p.name !== OTHERS && !/^Speaker \d+$/.test(p.name));
  const split = (count: number | null) => {
    if (named) setPending(count);
    else onSplit(count);
  };
  const max = Math.max(12, people.length);

  return (
    <div className="speaker-bar">
      {people.map((p) =>
        editing === p.name ? (
          <RenameSpeaker
            key={p.name}
            name={p.name}
            others={people.map((o) => o.name).filter((n) => n !== p.name)}
            onDone={async (to) => {
              setEditing(null);
              if (to && to !== p.name) {
                try {
                  await onRename(p.name, to);
                  setError(null);
                } catch (e) {
                  setError(errorText(e));
                }
              }
            }}
          />
        ) : (
          <button
            key={p.name}
            className={`speaker-chip${together ? " static" : ""}`}
            title={together ? undefined : "Rename"}
            disabled={busy}
            onClick={() => !together && setEditing(p.name)}
          >
            <span className="dot" style={colorStyle(colors, p.name)}>
              {initial(p.name)}
            </span>
            {p.name}
            <span className="talk">{talkTime(p.talkMs)}</span>
          </button>
        ),
      )}
      {error && <span className="muted">{error}</span>}
      <span className="speaker-count">
        {pending !== undefined ? (
          <>
            This replaces the names you've given.
            <button
              className="primary"
              onClick={() => {
                onSplit(pending);
                setPending(undefined);
              }}
            >
              Split again
            </button>
            <button onClick={() => setPending(undefined)}>Cancel</button>
          </>
        ) : together ? (
          <button disabled={busy} onClick={() => split(null)}>
            Tell speakers apart
          </button>
        ) : (
          <>
            People on the call
            <select
              disabled={busy}
              value=""
              onChange={(e) => split(e.target.value === "auto" ? null : Number(e.target.value))}
              aria-label="People on the call"
            >
              <option value="" disabled hidden>
                {people.length}
              </option>
              <option value="auto">Let Quill decide</option>
              {Array.from({ length: max }, (_, i) => i + 1).map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </>
        )}
      </span>
    </div>
  );
}

function RenameSpeaker({
  name,
  others,
  onDone,
}: {
  name: string;
  others: string[];
  onDone: (to: string | null) => void;
}) {
  const [draft, setDraft] = useState(/^Speaker \d+$/.test(name) || name === OTHERS ? "" : name);
  const merging = others.find((o) => o.toLowerCase() === draft.trim().toLowerCase());
  const listId = `speakers-${name.replace(/\W/g, "")}`;
  // Enter or Escape unmounts the input, which can also fire a blur.
  const finished = useRef(false);
  const finish = (save: boolean) => {
    if (finished.current) return;
    finished.current = true;
    onDone(save && draft.trim() ? (merging ?? draft.trim()) : null);
  };
  return (
    <span className="speaker-rename">
      <input
        autoFocus
        list={listId}
        placeholder={`Name for ${name}`}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => finish(true)}
        onKeyDown={(e) => {
          if (e.key === "Enter") finish(true);
          if (e.key === "Escape") finish(false);
        }}
      />
      <datalist id={listId}>
        {others.map((o) => (
          <option key={o} value={o} />
        ))}
      </datalist>
      <span className="hint">{merging ? `Merges with ${merging}` : "Enter to save, Esc to cancel"}</span>
    </span>
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
