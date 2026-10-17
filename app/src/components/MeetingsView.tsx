import { useEffect, useState } from "react";
import { api, clock, errorText, minutes, type Meeting, type Progress, type SearchHit } from "../api";
import { Icon, Logo } from "./Icon";
import { groupByDay, timeOfDay } from "./ui";

/** Meetings shown before "Show more", and how many more each click shows. */
const PAGE = 30;

/**
 * Every meeting, newest first and grouped by day, with search across the
 * transcripts.
 */
export function MeetingsView({
  meetings,
  progress,
  recordingId,
  onSelect,
}: {
  meetings: Meeting[];
  progress: Record<string, Progress>;
  recordingId: string | null;
  onSelect: (id: string, atMs?: number) => void;
}) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [shown, setShown] = useState(PAGE);

  // Search as you type, debounced so we don't query on every keystroke.
  useEffect(() => {
    if (!query.trim()) {
      setHits(null);
      return;
    }
    const timer = setTimeout(() => {
      api
        .search(query)
        .then((h) => {
          setHits(h);
          setError(null);
        })
        .catch((e) => setError(errorText(e)));
    }, 150);
    return () => clearTimeout(timer);
  }, [query]);

  return (
    <div className="page meetings">
      <header className="page-head">
        <div>
          <h1 className="page-title">Meetings</h1>
          <p className="page-subtitle">
            {meetings.length === 0
              ? "Recorded calls, with their notes and transcripts."
              : `${meetings.length} ${meetings.length === 1 ? "meeting" : "meetings"}`}
          </p>
        </div>
      </header>

      {meetings.length > 0 && (
        <label className="search page-search">
          <Icon name="search" size={14} />
          <input
            type="search"
            placeholder="Search transcripts"
            aria-label="Search transcripts"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
      )}
      {error && <p className="error-text small">{error}</p>}

      {hits ? (
        <section className="group">
          <h2 className="group-title">{hits.length === 0 ? "No matches" : `${hits.length} matches`}</h2>
          {hits.length > 0 && (
            <ul className="card item-list">
              {hits.map((h, i) => (
                <li key={`${h.meeting_id}-${h.start_ms}-${i}`}>
                  <button className="item-row" onClick={() => onSelect(h.meeting_id, h.start_ms)}>
                    <span className="item-title">{h.title}</span>
                    <span className="item-meta">
                      {clock(h.start_ms)}
                      {h.speaker && <span className="dot-sep">{h.speaker}</span>}
                    </span>
                    <Snippet text={h.snippet} />
                    <Icon name="external" size={13} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : meetings.length === 0 ? (
        <NoMeetings />
      ) : (
        <>
          {groupByDay(meetings.slice(0, shown), (m) => m.started_at).map(([label, group, oneDay]) => (
            <section className="group" key={label}>
              <h2 className="group-title">{label}</h2>
              <ul className="card item-list">
                {group.map((m) => (
                  <li key={m.id}>
                    <MeetingRow
                      meeting={m}
                      progress={progress[m.id]}
                      recording={m.id === recordingId}
                      timeOnly={oneDay}
                      onClick={() => onSelect(m.id)}
                    />
                  </li>
                ))}
              </ul>
            </section>
          ))}
          {meetings.length > shown && (
            <button className="show-more" onClick={() => setShown((n) => n + PAGE)}>
              Show more meetings
            </button>
          )}
        </>
      )}
    </div>
  );
}

/** One meeting in a list: title, when, length and state. */
export function MeetingRow({
  meeting,
  progress,
  recording,
  timeOnly = false,
  onClick,
}: {
  meeting: Meeting;
  progress?: Progress;
  recording: boolean;
  /** Under a heading for its day: just the time. */
  timeOnly?: boolean;
  onClick: () => void;
}) {
  return (
    <button className="item-row" onClick={onClick}>
      <span className="item-title">{meeting.title}</span>
      <span className="item-meta">
        {timeOnly ? timeOfDay(meeting.started_at) : shortWhen(meeting.started_at)}
        {meeting.duration_ms > 0 && <span className="dot-sep">{minutes(meeting.duration_ms)}</span>}
        <Status meeting={meeting} progress={progress} recording={recording} />
      </span>
      <Icon name="external" size={13} />
    </button>
  );
}

/** Shown before the first recording. */
export function NoMeetings() {
  return (
    <div className="card home-empty">
      <Logo size={40} />
      <div>
        <strong>Your meetings will appear here</strong>
        <p className="muted small">
          Press Record when a call starts, here or from the menu-bar icon. Works with Zoom, Meet, Teams or any other
          call.
        </p>
      </div>
    </div>
  );
}

/** "Mon, Oct 12 · 2:15 PM" in the user's locale. */
function shortWhen(rfc3339: string): string {
  const d = new Date(rfc3339);
  const day = d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return `${day} · ${time}`;
}

/** A meeting's state in a list: recording, working, failed, nothing said, not transcribed. */
export function Status({ meeting, progress, recording }: { meeting: Meeting; progress?: Progress; recording: boolean }) {
  if (recording) {
    return (
      <span className="status recording dot-sep">
        <span className="rec-dot pulsing" style={{ width: 7, height: 7 }} /> Recording
      </span>
    );
  }
  if (progress && progress.stage !== "done" && progress.stage !== "failed") {
    const label = {
      downloading: "Downloading",
      transcribing: "Transcribing",
      identifying: "Finding speakers",
      summarizing: "Summarizing",
    }[progress.stage];
    return (
      <span className="status working dot-sep">
        <span className="spinner" style={{ width: 9, height: 9, borderWidth: 1.5 }} /> {label}
      </span>
    );
  }
  if (progress?.stage === "failed") return <span className="status failed dot-sep">Failed</span>;
  if (meeting.empty) return <span className="status pending dot-sep">Nothing said</span>;
  if (!meeting.has_transcript) return <span className="status pending dot-sep">Not transcribed</span>;
  return null;
}

/** Renders FTS snippets, highlighting the `[matched]` words. */
function Snippet({ text }: { text: string }) {
  const parts = text.split(/(\[[^\]]*\])/g);
  return (
    <span className="snippet">
      {parts.map((p, i) =>
        p.startsWith("[") && p.endsWith("]") ? <mark key={i}>{p.slice(1, -1)}</mark> : <span key={i}>{p}</span>,
      )}
    </span>
  );
}
