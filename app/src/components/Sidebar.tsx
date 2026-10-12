import { useEffect, useState, type ReactNode } from "react";
import {
  api,
  clock,
  errorText,
  minutes,
  type DictationPhase,
  type Meeting,
  type Progress,
  type SearchHit,
} from "../api";
import { Icon } from "./Icon";

interface Props {
  meetings: Meeting[];
  selectedId: string | null;
  progress: Record<string, Progress>;
  recordingId: string | null;
  dictation: DictationPhase;
  onSelect: (id: string, atMs?: number) => void;
  onSettings: () => void;
  onDictation: () => void;
  onInsights: () => void;
  /** Which main view is showing, to highlight its nav button. */
  current: string;
  /** Shown above the search box (the record button). */
  header: ReactNode;
}

export function Sidebar({
  meetings,
  selectedId,
  progress,
  recordingId,
  dictation,
  onSelect,
  onSettings,
  onDictation,
  onInsights,
  current,
  header,
}: Props) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [error, setError] = useState<string | null>(null);

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
    <aside className="sidebar">
      <div className="drag" data-tauri-drag-region />
      <div className="sidebar-head">
        {header}
        <label className="search">
          <Icon name="search" size={14} />
          <input
            type="search"
            placeholder="Search transcripts"
            aria-label="Search transcripts"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
      </div>

      {error && <p className="list-empty error-text">{error}</p>}

      {hits ? (
        <ul className="list">
          <li className="list-group">{hits.length === 0 ? "No matches" : `${hits.length} matches`}</li>
          {hits.map((h, i) => (
            <li key={`${h.meeting_id}-${h.start_ms}-${i}`}>
              <button className="list-item" onClick={() => onSelect(h.meeting_id, h.start_ms)}>
                <span className="list-title">{h.title}</span>
                <span className="list-meta">
                  {clock(h.start_ms)}
                  {h.speaker && <span className="dot-sep">{h.speaker}</span>}
                </span>
                <Snippet text={h.snippet} />
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <ul className="list">
          {meetings.length === 0 && <li className="list-empty">No meetings yet</li>}
          {groupByDay(meetings).map(([label, group]) => (
            <li key={label}>
              <div className="list-group">{label}</div>
              <ul className="list" style={{ padding: 0 }}>
                {group.map((m) => (
                  <li key={m.id}>
                    <button
                      className={`list-item${m.id === selectedId ? " selected" : ""}`}
                      onClick={() => onSelect(m.id)}
                    >
                      <span className="list-title">{m.title}</span>
                      <span className="list-meta">
                        {time(m.started_at)}
                        {m.duration_ms > 0 && <span className="dot-sep">{minutes(m.duration_ms)}</span>}
                        <Status meeting={m} progress={progress[m.id]} recording={m.id === recordingId} />
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      )}

      <nav className="sidebar-nav">
        <button className={`nav-item${current === "dictation" ? " selected" : ""}`} onClick={onDictation}>
          <Icon name="mic" size={16} /> Dictation
          {dictation === "listening" && (
            <span className="badge pill red">
              <span className="rec-dot pulsing" /> Listening
            </span>
          )}
          {dictation === "processing" && (
            <span className="badge pill accent">
              <span className="spinner" style={{ width: 10, height: 10 }} /> Typing
            </span>
          )}
        </button>
        <button className={`nav-item${current === "insights" ? " selected" : ""}`} onClick={onInsights}>
          <Icon name="chart" size={16} /> Insights
        </button>
        <button className={`nav-item${current === "settings" ? " selected" : ""}`} onClick={onSettings}>
          <Icon name="settings" size={16} /> Settings
        </button>
      </nav>
    </aside>
  );
}

function Status({ meeting, progress, recording }: { meeting: Meeting; progress?: Progress; recording: boolean }) {
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
    }[
      progress.stage
    ];
    return (
      <span className="status working dot-sep">
        <span className="spinner" style={{ width: 9, height: 9, borderWidth: 1.5 }} /> {label}
      </span>
    );
  }
  if (progress?.stage === "failed") return <span className="status failed dot-sep">Failed</span>;
  if (!meeting.has_transcript) return <span className="status pending dot-sep">Not transcribed</span>;
  return null;
}

/** "Today", "Yesterday", weekday names this week, then "October 2026". */
function groupByDay(meetings: Meeting[]): [string, Meeting[]][] {
  const groups = new Map<string, Meeting[]>();
  const startOfToday = new Date();
  startOfToday.setHours(0, 0, 0, 0);
  const day = 86_400_000;
  for (const m of meetings) {
    const date = new Date(m.started_at);
    const ago = startOfToday.getTime() - date.getTime();
    const label =
      ago <= 0
        ? "Today"
        : ago <= day
          ? "Yesterday"
          : ago <= 6 * day
            ? date.toLocaleDateString(undefined, { weekday: "long" })
            : date.toLocaleDateString(undefined, { month: "long", year: "numeric" });
    groups.set(label, [...(groups.get(label) ?? []), m]);
  }
  return [...groups];
}

function time(rfc3339: string): string {
  return new Date(rfc3339).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
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
