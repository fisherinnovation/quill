// The dictation history: listed in the sidebar under the meetings, a page
// at a time, and opened one entry at a time in the main pane.
import { useCallback, useEffect, useState } from "react";
import { api, errorText, events, shortDate, type DictationEntry } from "../api";
import { Icon } from "./Icon";

/** Dictations loaded per page ("Show more" loads another). */
export const DICTATION_PAGE = 10;

export interface DictationHistory {
  entries: DictationEntry[];
  /** There are older entries to load. */
  more: boolean;
  loadMore: () => void;
  /** After a correction. */
  replace: (entry: DictationEntry) => void;
  remove: (id: number) => void;
  /** After "Delete all history". */
  clear: () => void;
}

/** The history as the sidebar shows it, kept current by `dictation-entry` events. */
export function useDictationHistory(): DictationHistory {
  const [entries, setEntries] = useState<DictationEntry[]>([]);
  const [more, setMore] = useState(false);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    void api.listDictations(null, DICTATION_PAGE).then((page) => {
      setEntries(page);
      setMore(page.length === DICTATION_PAGE);
    });
    const unlisten = events.onDictationEntry((entry) => setEntries((all) => [entry, ...all]));
    return () => void unlisten.then((f) => f());
  }, []);

  const loadMore = useCallback(() => {
    if (loading) return;
    setLoading(true);
    const oldest = entries[entries.length - 1]?.id ?? null;
    api
      .listDictations(oldest, DICTATION_PAGE)
      .then((page) => {
        setEntries((all) => [...all, ...page.filter((e) => !all.some((a) => a.id === e.id))]);
        setMore(page.length === DICTATION_PAGE);
      })
      .finally(() => setLoading(false));
  }, [entries, loading]);

  return {
    entries,
    more,
    loadMore,
    replace: (entry) => setEntries((all) => all.map((e) => (e.id === entry.id ? entry : e))),
    remove: (id) => setEntries((all) => all.filter((e) => e.id !== id)),
    clear: () => {
      setEntries([]);
      setMore(false);
    },
  };
}

/** A dictation's text for a one-line list title. */
export function dictationTitle(entry: DictationEntry): string {
  return (entry.corrected ?? entry.text).replace(/\s+/g, " ").trim() || "(empty)";
}

/** One dictation, opened from the sidebar: the text, Correct, Copy and Delete. */
export function DictationEntryView({
  id,
  onChanged,
  onDeleted,
  onOpenDictation,
}: {
  id: number;
  onChanged: (entry: DictationEntry) => void;
  onDeleted: (id: number) => void;
  /** Shows the Dictation page, where suggestions from corrections are. */
  onOpenDictation: () => void;
}) {
  const [entry, setEntry] = useState<DictationEntry | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [learned, setLearned] = useState<number | null>(null);
  const [copied, setCopied] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    api
      .getDictation(id)
      .then(setEntry)
      .catch((e) => setError(errorText(e)));
  }, [id]);

  if (!entry) return error ? <div className="page error-text">{error}</div> : null;
  const text = entry.corrected ?? entry.text;

  async function saveCorrection() {
    if (editing === null) return;
    try {
      const before = new Set((await api.listSuggestions()).map((s) => s.id));
      const { entry: saved, suggestions } = await api.correctDictation(id, editing);
      setEntry(saved);
      onChanged(saved);
      setLearned(suggestions.filter((s) => s.status === "pending" && !before.has(s.id)).length);
      setEditing(null);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }

  async function copy() {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <article className="page dictation-entry">
      <header className="page-head">
        <div>
          <h1 className="page-title">Dictation</h1>
          <p className="page-subtitle">
            {shortDate(entry.created_at)}
            {entry.app && ` · ${entry.app}`}
          </p>
        </div>
        <span className="spacer" />
        {confirmDelete ? (
          <div className="confirm">
            Delete this dictation?
            <button
              className="danger"
              onClick={async () => {
                await api.deleteDictation(id);
                onDeleted(id);
              }}
            >
              Delete
            </button>
            <button onClick={() => setConfirmDelete(false)}>Cancel</button>
          </div>
        ) : (
          <span className="row-gap">
            {editing === null && (
              <button
                onClick={() => {
                  setLearned(null);
                  setEditing(text);
                }}
              >
                <Icon name="text" size={14} /> Correct
              </button>
            )}
            <button onClick={() => void copy()}>
              <Icon name={copied ? "check" : "copy"} size={14} /> {copied ? "Copied" : "Copy"}
            </button>
            <button className="icon-btn" aria-label="Delete" title="Delete" onClick={() => setConfirmDelete(true)}>
              <Icon name="trash" size={14} />
            </button>
          </span>
        )}
      </header>

      <div className="card history-item">
        {editing !== null ? (
          <div className="correct-editor">
            <textarea
              autoFocus
              rows={Math.min(12, Math.max(3, Math.ceil(editing.length / 70)))}
              value={editing}
              onChange={(e) => setEditing(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void saveCorrection();
                if (e.key === "Escape") setEditing(null);
              }}
            />
            <div className="row-gap">
              <span className="faint small">Fix what Quill got wrong. It suggests words to learn from your changes.</span>
              <span className="spacer" />
              <button onClick={() => setEditing(null)}>Cancel</button>
              <button
                className="primary"
                disabled={!editing.trim() || editing.trim() === text}
                onClick={() => void saveCorrection()}
              >
                Save
              </button>
            </div>
          </div>
        ) : (
          <p className="history-text">{text}</p>
        )}
        {error && <p className="error-text small">{error}</p>}
        {learned !== null && (
          <p className="correct-result small">
            <Icon name="check" size={12} />{" "}
            {learned > 0 ? (
              <>
                Saved. {learned === 1 ? "1 suggestion" : `${learned} suggestions`} to review under{" "}
                <button className="link" onClick={onOpenDictation}>
                  Dictation → Learn from your fixes
                </button>
                .
              </>
            ) : (
              "Saved. Nothing to learn from this one (only misspelled names and terms, or words typed apart, are learned)."
            )}
          </p>
        )}
        <div className="history-meta">
          <span>{(entry.audio_ms / 1000).toFixed(1)} s spoken</span>
          <span className="dot-sep">typed in {(entry.latency_ms / 1000).toFixed(1)} s</span>
          {entry.polish_model && (
            <span className="dot-sep row-gap" style={{ gap: 4 }}>
              <Icon name="sparkle" size={12} /> tidied by {entry.polish_model}
            </span>
          )}
          {!entry.pasted && <span className="pill amber">Left on clipboard</span>}
          {entry.corrected && <span className="dot-sep">corrected</span>}
        </div>
        {entry.corrected && (
          <details>
            <summary>As typed</summary>
            {entry.text}
          </details>
        )}
        {entry.raw && (
          <details>
            <summary>As spoken</summary>
            {entry.raw}
          </details>
        )}
      </div>
    </article>
  );
}
