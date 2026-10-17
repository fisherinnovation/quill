import { type CSSProperties, useEffect, useState } from "react";
import {
  api,
  clock,
  errorText,
  minutes,
  type Meeting,
  type MeetingOverview,
  type Progress,
  type SearchHit,
} from "../api";
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
  const [overviews, setOverviews] = useState<Map<string, MeetingOverview>>(new Map());

  // Refetched when the list changes (a new meeting, a finished transcript).
  useEffect(() => {
    api
      .meetingOverviews()
      .then((all) => setOverviews(new Map(all.map((o) => [o.id, o]))))
      .catch(() => {});
  }, [meetings]);

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
              <ul className="meeting-tiles">
                {group.map((m) => (
                  <li key={m.id}>
                    <MeetingTile
                      meeting={m}
                      overview={overviews.get(m.id)}
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

/**
 * One meeting on the Meetings page: the gist and who was there over a faint
 * picture of the conversation's back and forth.
 */
function MeetingTile({
  meeting,
  overview,
  progress,
  recording,
  timeOnly,
  onClick,
}: {
  meeting: Meeting;
  overview?: MeetingOverview;
  progress?: Progress;
  recording: boolean;
  timeOnly: boolean;
  onClick: () => void;
}) {
  const o = overview;
  const share = o?.my_share != null && o.people.length > 0 ? Math.round(o.my_share * 100) : null;
  return (
    <button className="meeting-tile" onClick={onClick}>
      {o && o.speaker.length > 0 && <Conversation overview={o} />}
      <span className="tile-top">
        <span className="tile-title">{meeting.title}</span>
        <Status meeting={meeting} progress={progress} recording={recording} />
        <span className="tile-when">{timeOnly ? timeOfDay(meeting.started_at) : shortWhen(meeting.started_at)}</span>
      </span>
      {o?.gist && <span className="tile-gist">{o.gist}</span>}
      <span className="tile-meta">
        {o && o.people.length > 0 && (
          <span className="tile-people">
            <People people={o.people} />
            {peopleLabel(o.people)}
          </span>
        )}
        {meeting.duration_ms > 0 && <span className="dot-sep">{minutes(meeting.duration_ms)}</span>}
        {o && o.action_items > 0 && (
          <span className="dot-sep">
            {o.action_items} action {o.action_items === 1 ? "item" : "items"}
          </span>
        )}
        {share !== null && <span className="dot-sep">you spoke {share}%</span>}
      </span>
    </button>
  );
}

/** A speaker's colour: you in sky, the others in the transcript's colours. */
function speakerColor(slot: number): string {
  return slot === 1 ? "var(--me)" : `var(--spk-${((slot - 2) % 8) + 1})`;
}

/**
 * The conversation's back and forth across the tile: your turns on one lane,
 * the call's on another, each a pill in the speaker's colour and as long as
 * they held the floor. Fainter where little was said.
 */
function Conversation({ overview }: { overview: MeetingOverview }) {
  const { speaker, level } = overview;
  const runs: { slot: number; start: number; end: number; level: number }[] = [];
  speaker.forEach((slot, i) => {
    const last = runs[runs.length - 1];
    if (slot === 0) return;
    if (last && last.slot === slot && last.end === i) {
      last.end = i + 1;
      last.level = Math.max(last.level, level[i]);
    } else {
      runs.push({ slot, start: i, end: i + 1, level: level[i] });
    }
  });
  return (
    <svg className="tile-pattern" viewBox={`0 0 ${speaker.length} 40`} preserveAspectRatio="none" aria-hidden="true">
      {runs.map((r) => (
        <rect
          key={r.start}
          x={r.start + 0.15}
          y={r.slot === 1 ? 15 : 22}
          width={r.end - r.start - 0.3}
          height={3}
          rx={0.5}
          ry={1.5}
          style={{ fill: speakerColor(r.slot), opacity: 0.45 + (r.level / 100) * 0.55 }}
        />
      ))}
    </svg>
  );
}

/** You and up to three others as overlapping dots, in their pattern colours. */
function People({ people }: { people: string[] }) {
  const shown = people.slice(0, 3);
  return (
    <span className="people-dots" aria-hidden="true">
      <span style={{ "--dot": speakerColor(1) } as CSSProperties} />
      {shown.map((name, i) => (
        <span key={name} style={{ "--dot": speakerColor(i + 2) } as CSSProperties} />
      ))}
    </span>
  );
}

/** "You and Priya", "You, Priya and Dan", "You and 3 others". */
function peopleLabel(people: string[]): string {
  const named = people.filter((p) => !/^Speaker \d+$/.test(p) && p !== "Others");
  if (people.length === 1 && people[0] === "Others") return "You and others";
  if (named.length === 0) return `You and ${people.length} ${people.length === 1 ? "other" : "others"}`;
  const rest = people.length - Math.min(named.length, 2);
  const names = named.slice(0, 2);
  if (rest === 0) return names.length === 1 ? `You and ${names[0]}` : `You, ${names[0]} and ${names[1]}`;
  return `You, ${names.join(", ")} and ${rest} ${rest === 1 ? "other" : "others"}`;
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
