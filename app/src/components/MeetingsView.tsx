import { type CSSProperties, useEffect, useMemo, useState } from "react";
import {
  api,
  clock,
  errorText,
  isMac,
  minutes,
  type Insights,
  type Meeting,
  type MeetingOverview,
  type Progress,
  type RecordingState,
} from "../api";
import { ConversationWeave } from "./ConversationWeave";
import { Icon, type IconName, Logo } from "./Icon";
import { duration } from "./Insights";
import { useElapsed } from "./RecordButton";
import { SEARCH_SHORTCUT } from "./SearchPalette";
import { writeUpFraction, writeUpStatus } from "./WriteUp";
import { groupByDay, timeOfDay, useCountUp } from "./ui";

/** Meetings shown before "Show more", and how many more each click shows. */
const PAGE = 30;

/** A week, for "this week" numbers. */
const WEEK_MS = 7 * 86_400_000;

/** Which meetings the list shows: all, a kind, or those with someone. */
type Filter = { kind: "all" | "actions" | "new" | "attention" } | { person: string };

/** A name the user gave, not a placeholder. */
const named = (p: string) => !/^Speaker \d+$/.test(p) && p !== "Others";

/**
 * Every meeting, newest first and grouped by day, with a way into search
 * (narrowed to meetings). Above them, a hero woven from recent
 * conversations, this week's numbers, and filters by kind and by person.
 */
export function MeetingsView({
  meetings,
  progress,
  recording,
  fresh,
  onSelect,
  onSearch,
}: {
  meetings: Meeting[];
  progress: Record<string, Progress>;
  recording: RecordingState;
  /** Meetings written up since the user last opened them. */
  fresh: Set<string>;
  onSelect: (id: string, atMs?: number) => void;
  /** Opens search, narrowed to meetings. */
  onSearch: () => void;
}) {
  const recordingId = recording.recording ? recording.meeting_id : null;
  const [insights, setInsights] = useState<Insights | null>(null);
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [shown, setShown] = useState(PAGE);
  const [overviews, setOverviews] = useState<Map<string, MeetingOverview>>(new Map());

  // Refetched when the list changes (a new meeting, a finished transcript).
  useEffect(() => {
    api
      .meetingOverviews()
      .then((all) => setOverviews(new Map(all.map((o) => [o.id, o]))))
      .catch(() => {});
    api
      .getInsights()
      .then(setInsights)
      .catch(() => setInsights(null));
  }, [meetings]);

  const attention = (m: Meeting) =>
    m.id !== recordingId && (m.empty || !m.has_transcript || progress[m.id]?.stage === "failed");
  // The people met most (by name), for filters: up to three, met at least twice.
  const people = useMemo(() => {
    const count = new Map<string, number>();
    for (const o of overviews.values()) {
      for (const p of new Set(o.people.filter(named))) count.set(p, (count.get(p) ?? 0) + 1);
    }
    return [...count.entries()]
      .filter(([, n]) => n >= 2)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 3)
      .map(([name]) => name);
  }, [overviews]);
  const matches = (m: Meeting, f: Filter) => {
    if ("person" in f) return overviews.get(m.id)?.people.includes(f.person) ?? false;
    switch (f.kind) {
      case "actions":
        return (overviews.get(m.id)?.action_items ?? 0) > 0;
      case "new":
        return fresh.has(m.id);
      case "attention":
        return attention(m);
      default:
        return true;
    }
  };
  const listed = meetings.filter((m) => matches(m, filter));

  const recent = meetings
    .map((m) => overviews.get(m.id))
    .filter((o): o is MeetingOverview => o !== undefined);
  const weekAgo = Date.now() - WEEK_MS;
  const thisWeek = meetings.filter((m) => Date.parse(m.started_at) >= weekAgo);
  const actionsThisWeek = thisWeek.reduce((n, m) => n + (overviews.get(m.id)?.action_items ?? 0), 0);
  const writing = meetings.find((m) => {
    const p = progress[m.id];
    return p && p.stage !== "done" && p.stage !== "failed";
  });

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
        <>
          <Hero
            recording={recording}
            meeting={meetings.find((m) => m.id === recordingId)}
            writing={writing}
            progress={writing ? progress[writing.id] : undefined}
            insights={insights}
            recent={recent}
            onOpen={onSelect}
          />
          {insights && <WeekNumbers insights={insights} actions={actionsThisWeek} />}
        </>
      )}

      {meetings.length > 0 && (
        // Looks like a search box; opens search, narrowed to meetings.
        <button className="search page-search search-opener" onClick={onSearch}>
          <Icon name="search" size={14} />
          <span>Search meetings</span>
          <kbd>{SEARCH_SHORTCUT}</kbd>
        </button>
      )}

      {meetings.length === 0 ? (
        <NoMeetings />
      ) : (
        <>
          <Filters
            meetings={meetings}
            people={people}
            filter={filter}
            count={(f) => meetings.filter((m) => matches(m, f)).length}
            onChange={setFilter}
          />
          {listed.length === 0 && <p className="muted filter-none">No meetings here yet.</p>}
          {groupByDay(listed.slice(0, shown), (m) => m.started_at).map(([label, group, oneDay]) => (
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
                      fresh={fresh.has(m.id)}
                      timeOnly={oneDay}
                      onClick={() => onSelect(m.id)}
                    />
                  </li>
                ))}
              </ul>
            </section>
          ))}
          {listed.length > shown && (
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
  fresh,
  timeOnly,
  onClick,
}: {
  meeting: Meeting;
  overview?: MeetingOverview;
  progress?: Progress;
  recording: boolean;
  fresh: boolean;
  timeOnly: boolean;
  onClick: () => void;
}) {
  const o = overview;
  const share = o?.my_share != null && o.people.length > 0 ? Math.round(o.my_share * 100) : null;
  const working = progress && progress.stage !== "done" && progress.stage !== "failed";
  const done = working ? writeUpFraction(progress) : null;
  return (
    <button className={`meeting-tile${fresh ? " fresh" : ""}`} onClick={onClick}>
      {done != null && (
        <span className="tile-progress" aria-hidden>
          <span style={{ width: `${done * 100}%` }} />
        </span>
      )}
      {o && o.speaker.length > 0 && <Conversation overview={o} />}
      <span className="tile-top">
        <span className="tile-title">{meeting.title}</span>
        <Status meeting={meeting} progress={progress} recording={recording} fresh={fresh} />
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

/**
 * The top of the page: what's happening (ready, recording, writing up),
 * this week in a line, over a weave of recent conversations.
 */
function Hero({
  recording,
  meeting,
  writing,
  progress,
  insights,
  recent,
  onOpen,
}: {
  recording: RecordingState;
  /** The meeting being recorded. */
  meeting?: Meeting;
  /** A meeting being written up. */
  writing?: Meeting;
  progress?: Progress;
  insights: Insights | null;
  recent: MeetingOverview[];
  onOpen: (id: string) => void;
}) {
  const elapsed = useElapsed(recording);
  const [error, setError] = useState<string | null>(null);
  const phase = recording.recording ? "recording" : writing ? "writing" : "idle";
  const m = insights?.meetings;
  const share = m?.my_talk_share != null ? Math.round(m.my_talk_share * 100) : null;

  async function record() {
    try {
      setError(null);
      await api.startRecording();
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <div className={`hero meetings-hero ${phase}${recording.paused ? " paused" : ""}`}>
      <ConversationWeave overviews={recent} phase={phase} />
      <div className="hero-body">
        <span className="hero-status">
          <span className="hero-dot" />
          {phase === "recording"
            ? `${recording.paused ? "Paused" : "Recording"} · ${clock(elapsed)}`
            : phase === "writing" && progress
              ? `Writing up · ${writeUpStatus(progress)}`
              : "Ready to record"}
        </span>
        <div className="hero-title">
          {phase === "recording"
            ? (meeting?.title ?? "Recording")
            : phase === "writing"
              ? writing?.title
              : m && m.ms_last_7_days > 0
                ? `${duration(m.ms_last_7_days)} on calls this week`
                : "No calls yet this week"}
        </div>
        <p>
          {phase === "recording"
            ? "Stop when the call ends, and the notes follow."
            : phase === "writing"
              ? "Turning the call into a transcript and notes."
              : m && m.count_last_7_days > 0
                ? `${m.count_last_7_days} ${m.count_last_7_days === 1 ? "meeting" : "meetings"} this week${share !== null ? `, and ${share}% of the talking was you` : ""}.`
                : isMac
                  ? "Record when a call starts, or let Quill offer when it notices one."
                  : "Record when a call starts, from here or the tray icon."}
        </p>
        {phase === "idle" ? (
          <button className="primary" onClick={() => void record()}>
            <span className="rec-dot" /> Record meeting
          </button>
        ) : (
          <button onClick={() => onOpen((phase === "recording" ? meeting?.id : writing?.id) ?? "")}>
            Open <Icon name="chevron-right" size={13} />
          </button>
        )}
        {error && <p className="error-text small">{error}</p>}
      </div>
    </div>
  );
}

/** This week's numbers, counting up once. */
function WeekNumbers({ insights, actions }: { insights: Insights; actions: number }) {
  const m = insights.meetings;
  const progress = useCountUp();
  const week = insights.days.slice(-7);
  const busiest = Math.max(1, ...week.map((d) => d.meeting_ms));
  const stats: { icon: IconName; value: number; format: (v: number) => string; label: string; chart?: boolean }[] = [
    {
      icon: "users",
      value: m.count_last_7_days,
      format: (v) => String(Math.round(v)),
      label: "meetings this week",
    },
    {
      icon: "clock",
      value: m.ms_last_7_days,
      format: (v) => duration(v),
      label: "on calls this week",
      chart: true,
    },
    {
      icon: "mic",
      value: (m.my_talk_share ?? 0) * 100,
      format: (v) => (m.my_talk_share != null ? `${Math.round(v)}%` : "–"),
      label: "of the talking is you",
    },
    {
      icon: "check",
      value: actions,
      format: (v) => String(Math.round(v)),
      label: "action items this week",
    },
  ];
  return (
    <div className="stat-tiles">
      {stats.map((s) => (
        <div className="stat-tile" key={s.label}>
          <Icon name={s.icon} size={15} />
          <strong>{s.format(s.value * progress)}</strong>
          <span>{s.label}</span>
          {s.chart && (
            <span className="stat-spark" aria-hidden>
              {week.map((d) => (
                <i key={d.date} style={{ height: `${Math.max(8, (d.meeting_ms / busiest) * 100) * progress}%` }} />
              ))}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

/** All, a kind, or the people met most. Each chip counts its meetings. */
function Filters({
  meetings,
  people,
  filter,
  count,
  onChange,
}: {
  meetings: Meeting[];
  people: string[];
  filter: Filter;
  count: (f: Filter) => number;
  onChange: (f: Filter) => void;
}) {
  const kinds: { kind: "actions" | "new" | "attention"; label: string }[] = [
    { kind: "actions", label: "Action items" },
    { kind: "new", label: "New" },
    { kind: "attention", label: "Needs attention" },
  ];
  const on = (f: Filter) => JSON.stringify(f) === JSON.stringify(filter);
  const chips = [
    ...kinds.map((k) => ({ f: { kind: k.kind } as Filter, label: k.label, person: false })),
    ...people.map((p) => ({ f: { person: p } as Filter, label: p, person: true })),
  ].filter((c) => count(c.f) > 0 || on(c.f));
  if (chips.length === 0) return null;
  return (
    <div className="kind-filter meeting-filter" role="group" aria-label="Show">
      <button className={`kind-chip${on({ kind: "all" }) ? " on" : ""}`} aria-pressed={on({ kind: "all" })} onClick={() => onChange({ kind: "all" })}>
        All <span className="kind-count">{meetings.length}</span>
      </button>
      {chips.map((c) => (
        <button
          key={c.label}
          className={`kind-chip${c.person ? " person" : ""}${"kind" in c.f ? ` ${c.f.kind}` : ""}${on(c.f) ? " on" : ""}`}
          aria-pressed={on(c.f)}
          onClick={() => onChange(on(c.f) ? { kind: "all" } : c.f)}
        >
          {c.person ? <span className="person-initial">{c.label.charAt(0)}</span> : <span className="kind-dot" />}
          {c.label} <span className="kind-count">{count(c.f)}</span>
        </button>
      ))}
    </div>
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
export function Status({
  meeting,
  progress,
  recording,
  fresh,
}: {
  meeting: Meeting;
  progress?: Progress;
  recording: boolean;
  /** Written up since the user last looked. */
  fresh?: boolean;
}) {
  if (recording) {
    return (
      <span className="status recording dot-sep">
        <span className="rec-dot pulsing" style={{ width: 7, height: 7 }} /> Recording
      </span>
    );
  }
  if (progress && progress.stage !== "done" && progress.stage !== "failed") {
    const label = progress.report
      ? writeUpStatus(progress)
      : {
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
  if (fresh) return <span className="status fresh">New</span>;
  if (meeting.empty) return <span className="status pending dot-sep">Nothing said</span>;
  if (!meeting.has_transcript) return <span className="status pending dot-sep">Not transcribed</span>;
  return null;
}

/** Renders FTS snippets, highlighting the `[matched]` words. */
