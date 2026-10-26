import { useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, errorText, type DailySummary } from "../api";
import { Icon } from "./Icon";

/** A day's meetings need at least this many with notes to summarize: one meeting's notes are its own summary. */
export const DAY_SUMMARY_MIN_MEETINGS = 2;

/** The local day of an RFC 3339 time, as YYYY-MM-DD (how daily summaries are keyed). */
export function localDay(rfc3339: string): string {
  return new Date(rfc3339).toLocaleDateString("en-CA");
}

/**
 * Under a day's heading on the Meetings page: that day's summary, written
 * from its meetings' notes when asked, or the button to write it. Nothing
 * on days with fewer than two meetings with notes.
 */
export function DaySummary({
  day,
  withNotes,
  summary,
  onWritten,
}: {
  /** YYYY-MM-DD, local. */
  day: string;
  /** How many of the day's meetings have notes. */
  withNotes: number;
  summary: DailySummary | undefined;
  onWritten: (summary: DailySummary) => void;
}) {
  const [writing, setWriting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);

  async function write() {
    setWriting(true);
    setError(null);
    try {
      onWritten(await api.summarizeDay(day));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setWriting(false);
    }
  }

  if (!summary) {
    if (withNotes < DAY_SUMMARY_MIN_MEETINGS) return null;
    return (
      <div className="day-summary-start">
        <button className="link-button" onClick={() => void write()} disabled={writing}>
          {writing ? <span className="spinner" style={{ width: 12, height: 12 }} /> : <Icon name="sparkle" size={13} />}
          {writing ? "Writing the day's summary…" : "Summarize this day"}
        </button>
        {error && <span className="error-text small">{error}</span>}
      </div>
    );
  }

  const { tldr, rest } = split(summary.markdown);
  return (
    <section className="day-summary" aria-label="Daily summary">
      <header className="day-summary-head">
        <Icon name="sparkle" size={14} />
        <span className="day-summary-title">Daily summary</span>
        <span className="day-summary-meta">
          from {summary.meetings} {summary.meetings === 1 ? "meeting" : "meetings"}
        </span>
        {summary.outdated && (
          <span className="pill amber" title="A meeting's notes changed or another meeting that day got notes since">
            Notes changed
          </span>
        )}
        <span className="spacer" />
        <button className="ghost small" onClick={() => void write()} disabled={writing} title="Write it again from the day's notes">
          {writing ? <span className="spinner" style={{ width: 12, height: 12 }} /> : <Icon name="refresh" size={13} />}
          {writing ? "Rewriting…" : "Rewrite"}
        </button>
      </header>
      <div className="day-summary-body">
        <Markdown remarkPlugins={[remarkGfm]}>{tldr}</Markdown>
        {rest && expanded && <Markdown remarkPlugins={[remarkGfm]}>{rest}</Markdown>}
      </div>
      {rest && (
        <button className="link-button day-summary-more" onClick={() => setExpanded((e) => !e)} aria-expanded={expanded}>
          {expanded ? "Show less" : "Decisions, action items and more"}
        </button>
      )}
      {error && <p className="error-text small">{error}</p>}
    </section>
  );
}

/** The TL;DR's text (without its heading), and every other section. */
function split(markdown: string): { tldr: string; rest: string } {
  const sections = markdown.split(/^(?=## )/m).map((s) => s.trim());
  const isTldr = (s: string) => /^## TL;?DR/i.test(s);
  const tldr = sections.find(isTldr);
  if (!tldr) return { tldr: markdown, rest: "" };
  return {
    tldr: tldr.replace(/^## [^\n]*\n?/, "").trim(),
    rest: sections.filter((s) => s && !isTldr(s)).join("\n\n"),
  };
}
