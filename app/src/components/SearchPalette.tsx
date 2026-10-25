import { type ReactNode, useEffect, useRef, useState } from "react";
import { api, clock, errorText, isMac, type SearchMatch, type SearchResults, type SearchScope } from "../api";
import { AppIcon } from "./AppIcon";
import { Icon } from "./Icon";
import { SemanticSearchNote, useSemanticStatus } from "./SemanticSearch";

/** The shortcut that opens search, as shown in tooltips and on the Meetings page. */
export const SEARCH_SHORTCUT = isMac ? "⌘K" : "Ctrl+K";

/** Where a search result leads. */
export type SearchTarget = { kind: "meeting"; id: string; at?: number } | { kind: "dictation"; id: number };

/** The chips that narrow a search, in order. */
const SCOPES: [SearchScope, string][] = [
  ["all", "All"],
  ["meetings", "Meetings"],
  ["notes", "Notes"],
  ["actions", "Action items"],
  ["dictations", "Dictations"],
];

/** One row in the results: a result to open, or "Show all" for a kind. */
interface Row {
  key: string;
  icon: ReactNode;
  title: ReactNode;
  meta: ReactNode;
  snippet?: string;
  matched?: SearchMatch;
  /** Opens the result, or narrows the search to its kind. */
  open: () => void;
}

/** A group of rows under its heading. */
interface Section {
  title: string;
  rows: Row[];
}

/**
 * Search across everything, over the current page: meeting titles, moments
 * in transcripts, notes, action items and dictations, by their words and
 * (with the search model) their meaning. ↑↓ move, Enter opens, Esc closes.
 */
export function SearchPalette({
  initialScope,
  onOpen,
  onClose,
}: {
  /** Narrowed to one kind when opened, e.g. from the Meetings page. */
  initialScope: SearchScope;
  onOpen: (target: SearchTarget) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<SearchScope>(initialScope);
  const [results, setResults] = useState<SearchResults | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const semantic = useSemanticStatus();

  // Loads the search model now, so the first search doesn't wait.
  useEffect(() => {
    input.current?.focus();
    void api.prepareSearch().catch(() => {});
  }, []);

  // Search as you type, debounced; a slow answer never replaces a newer one.
  const latest = useRef(0);
  useEffect(() => {
    const ask = ++latest.current;
    if (!query.trim()) {
      setResults(null);
      setError(null);
      return;
    }
    const timer = setTimeout(() => {
      api
        .search(query, scope)
        .then((r) => {
          if (ask !== latest.current) return;
          setResults(r);
          setError(null);
          setActive(0);
        })
        .catch((e) => {
          if (ask === latest.current) setError(errorText(e));
        });
    }, 120);
    return () => clearTimeout(timer);
  }, [query, scope]);

  const go = (target: SearchTarget) => {
    onOpen(target);
    onClose();
  };
  const sections = results ? toSections(results, scope, go, setScope) : [];
  const rows = sections.flatMap((s) => s.rows);
  const count = rows.length;

  // Keeps the active row in view as the arrows move it.
  useEffect(() => {
    list.current?.querySelector(`[data-row="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown" && count > 0) {
      e.preventDefault();
      setActive((i) => (i + 1) % count);
    } else if (e.key === "ArrowUp" && count > 0) {
      e.preventDefault();
      setActive((i) => (i - 1 + count) % count);
    } else if (e.key === "Enter" && rows[active]) {
      e.preventDefault();
      rows[active].open();
    }
  };

  let index = 0;
  return (
    <div className="palette-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette" role="dialog" aria-modal="true" aria-label="Search" onKeyDown={onKey}>
        <label className="palette-search">
          <Icon name="search" size={16} />
          <input
            ref={input}
            type="search"
            placeholder="Search meetings, notes, action items and dictations"
            aria-label="Search"
            aria-controls="palette-results"
            aria-activedescendant={count > 0 ? `palette-row-${active}` : undefined}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <kbd className="palette-esc">Esc</kbd>
        </label>
        <div className="palette-scopes" role="group" aria-label="Search in">
          {SCOPES.map(([s, label]) => (
            <button
              key={s}
              className={`kind-chip${scope === s ? " on" : ""}`}
              aria-pressed={scope === s}
              onClick={() => {
                setScope(s);
                input.current?.focus();
              }}
            >
              {label}
            </button>
          ))}
        </div>

        <div className="palette-results" id="palette-results" role="listbox" ref={list}>
          {!query.trim() ? (
            <p className="palette-empty">
              Find a meeting, something said or decided, an action item, or a dictation.
            </p>
          ) : error ? (
            <p className="palette-empty error-text">{error}</p>
          ) : results && count === 0 ? (
            <p className="palette-empty">Nothing found for “{query.trim()}”.</p>
          ) : (
            sections.map((section) => (
              <section key={section.title} className="palette-group">
                <h2 className="palette-group-title">{section.title}</h2>
                {section.rows.map((row) => {
                  const i = index++;
                  return (
                    <button
                      key={row.key}
                      id={`palette-row-${i}`}
                      data-row={i}
                      role="option"
                      aria-selected={i === active}
                      className={`palette-row${i === active ? " active" : ""}`}
                      onMouseMove={() => i !== active && setActive(i)}
                      onClick={row.open}
                    >
                      <span className="palette-row-icon">{row.icon}</span>
                      <span className="palette-row-title">{row.title}</span>
                      <span className="palette-row-meta">
                        {row.meta}
                        {row.matched === "meaning" && (
                          <span className="match-tag" title="Found by meaning: it says this in other words">
                            Related
                          </span>
                        )}
                      </span>
                      {row.snippet && <Snippet text={row.snippet} />}
                    </button>
                  );
                })}
              </section>
            ))
          )}
        </div>
        <SemanticSearchNote status={semantic} />
        <footer className="palette-foot">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> to move
          </span>
          <span>
            <kbd>Enter</kbd> to open
          </span>
          <span>
            <kbd>Esc</kbd> to close
          </span>
        </footer>
      </div>
    </div>
  );
}

/** The results as titled groups of rows, in a fixed order. */
function toSections(
  r: SearchResults,
  scope: SearchScope,
  go: (target: SearchTarget) => void,
  narrow: (scope: SearchScope) => void,
): Section[] {
  const out: Section[] = [];
  const add = (title: string, rows: Row[], more: boolean, kind: SearchScope) => {
    if (rows.length === 0) return;
    if (more && scope === "all") {
      rows.push({
        key: `more-${kind}`,
        icon: <Icon name="chevron-right" size={14} />,
        title: <span className="palette-more">Show all {title.toLowerCase()}</span>,
        meta: null,
        open: () => narrow(kind),
      });
    }
    out.push({ title, rows });
  };

  add(
    "Meetings",
    [
      ...r.meetings.hits.map((h) => ({
        key: `meeting-${h.meeting_id}`,
        icon: <Icon name="users" size={14} />,
        title: <Snippet text={h.snippet} inline />,
        meta: day(h.started_at),
        open: () => go({ kind: "meeting", id: h.meeting_id }),
      })),
      ...r.moments.hits.map((h, i) => ({
        key: `moment-${h.meeting_id}-${h.start_ms}-${i}`,
        icon: <Icon name="wave" size={14} />,
        title: h.title,
        meta: (
          <>
            {clock(h.start_ms)}
            {h.speaker && <span className="dot-sep">{h.speaker}</span>}
          </>
        ),
        snippet: h.snippet,
        matched: h.matched,
        open: () => go({ kind: "meeting", id: h.meeting_id, at: h.start_ms }),
      })),
    ],
    r.meetings.more || r.moments.more,
    "meetings",
  );
  add(
    "Notes",
    r.notes.hits.map((h, i) => ({
      key: `notes-${h.meeting_id}-${i}`,
      icon: <Icon name="text" size={14} />,
      title: h.title,
      meta: (
        <>
          {h.heading ?? "Notes"}
          <span className="dot-sep">{day(h.started_at)}</span>
        </>
      ),
      snippet: h.snippet,
      matched: h.matched,
      open: () => go({ kind: "meeting", id: h.meeting_id }),
    })),
    r.notes.more,
    "notes",
  );
  add(
    "Action items",
    r.actions.hits.map((h) => ({
      key: `action-${h.action_id}`,
      icon: <Icon name={h.done ? "check" : "checklist"} size={14} />,
      title: <Snippet text={h.snippet} inline />,
      meta: (
        <>
          {h.done ? "Done" : "Open"}
          <span className="dot-sep">{h.title}</span>
        </>
      ),
      matched: h.matched,
      open: () => go({ kind: "meeting", id: h.meeting_id }),
    })),
    r.actions.more,
    "actions",
  );
  add(
    "Dictations",
    r.dictations.hits.map((h) => ({
      key: `dictation-${h.dictation_id}`,
      icon: h.app ? <AppIcon id={h.app_id} name={h.app} /> : <Icon name="mic" size={14} />,
      title: <Snippet text={h.snippet} inline />,
      meta: (
        <>
          {h.app ?? "Dictation"}
          <span className="dot-sep">{day(h.created_at)}</span>
        </>
      ),
      matched: h.matched,
      open: () => go({ kind: "dictation", id: h.dictation_id }),
    })),
    r.dictations.more,
    "dictations",
  );
  return out;
}

/** "Today", "Yesterday", or a short date ("Oct 12", with the year if not this one). */
function day(rfc3339: string): string {
  const d = new Date(rfc3339);
  if (Number.isNaN(d.getTime())) return "";
  const today = new Date();
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((start(today) - start(d)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: d.getFullYear() === today.getFullYear() ? undefined : "numeric",
  });
}

/** Text with search matches (wrapped in [ ]) highlighted. */
function Snippet({ text, inline }: { text: string; inline?: boolean }) {
  const parts = text.split(/(\[[^\]]*\])/g);
  return (
    <span className={inline ? "snippet-inline" : "snippet"}>
      {parts.map((p, i) =>
        p.startsWith("[") && p.endsWith("]") ? <mark key={i}>{p.slice(1, -1)}</mark> : <span key={i}>{p}</span>,
      )}
    </span>
  );
}
