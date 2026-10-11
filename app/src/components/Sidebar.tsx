import { useEffect, useState, type ReactNode } from "react";
import { api, clock, errorText, minutes, shortDate, type Meeting, type Progress, type SearchHit } from "../api";

interface Props {
  meetings: Meeting[];
  selectedId: string | null;
  progress: Record<string, Progress>;
  recordingId: string | null;
  onSelect: (id: string, atMs?: number) => void;
  onSettings: () => void;
  /** Shown above the search box (the record button). */
  header: ReactNode;
}

export function Sidebar({ meetings, selectedId, progress, recordingId, onSelect, onSettings, header }: Props) {
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
      <div className="sidebar-record">{header}</div>
      <div className="sidebar-top">
        <input
          className="search"
          type="search"
          placeholder="Search transcripts"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button className="icon-button" title="Settings" aria-label="Settings" onClick={onSettings}>
          ⚙︎
        </button>
      </div>

      {error && <p className="muted small pad">{error}</p>}

      {hits ? (
        <ul className="list">
          {hits.length === 0 && <li className="muted pad">No matches</li>}
          {hits.map((h, i) => (
            <li key={`${h.meeting_id}-${h.start_ms}-${i}`}>
              <button className="list-item" onClick={() => onSelect(h.meeting_id, h.start_ms)}>
                <span className="list-title">{h.title}</span>
                <span className="muted small">
                  {clock(h.start_ms)}
                  {h.speaker ? ` · ${h.speaker}` : ""}
                </span>
                <Snippet text={h.snippet} />
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <ul className="list">
          {meetings.map((m) => (
            <li key={m.id}>
              <button
                className={`list-item${m.id === selectedId ? " selected" : ""}`}
                onClick={() => onSelect(m.id)}
              >
                <span className="list-title">{m.title}</span>
                <span className="muted small">
                  {shortDate(m.started_at)} · {minutes(m.duration_ms)}
                  {status(m, progress[m.id], m.id === recordingId)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}

function status(m: Meeting, p: Progress | undefined, recording: boolean): string {
  if (recording) return " · recording";
  if (p && p.stage !== "done" && p.stage !== "failed") return ` · ${p.stage}…`;
  if (p?.stage === "failed") return " · failed";
  return m.has_transcript ? "" : " · not transcribed";
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
