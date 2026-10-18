import { useEffect, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  api,
  clock,
  errorText,
  type DictationPhase,
  type Insights,
  type Meeting,
  type MeetingDetail,
  minutes,
  type Progress,
  type RecordingState,
} from "../api";
import { Icon } from "./Icon";
import { InsightDetails, UsageFacts } from "./Insights";
import { callSpeakers, peopleText } from "./MeetingView";
import { NoMeetings, Status } from "./MeetingsView";
import { ModelDownload } from "./Models";
import { useElapsed } from "./RecordButton";
import { Wordmark } from "./Wordmark";
import { Keys, timeOfDay } from "./ui";
import { UpdateBanner } from "./Updates";

/** How many meetings Home lists under the latest notes. */
const RECENT = 6;

/**
 * Where Quill opens: what today looks like, the notes from the latest
 * meeting, the meetings before it, then how Quill has been used.
 */
export function HomeView({
  meetings,
  progress,
  recording,
  dictation,
  onSelect,
  onMeetings,
  onDictationSettings,
}: {
  meetings: Meeting[];
  progress: Record<string, Progress>;
  recording: RecordingState;
  dictation: DictationPhase;
  onSelect: (id: string) => void;
  onMeetings: () => void;
  onDictationSettings: () => void;
}) {
  const [insights, setInsights] = useState<Insights | null>(null);
  // Read fresh each visit: they can change in Settings → Dictation.
  const [shortcut, setShortcut] = useState<{ hotkey: string; enabled: boolean } | null>(null);
  const elapsed = useElapsed(recording);

  const loadInsights = () => void api.getInsights().then(setInsights).catch(() => setInsights(null));
  useEffect(loadInsights, [meetings.length]);
  useEffect(() => {
    void api.getSettings().then((s) => setShortcut({ hotkey: s.dictation.hotkey, enabled: s.dictation.enabled }));
  }, []);

  const now = new Date();
  const today = meetings.filter((m) => sameDay(new Date(m.started_at), now));
  const featured = meetings.find((m) => m.has_summary && m.id !== recording.meeting_id);
  const recent = meetings.filter((m) => m.id !== featured?.id).slice(0, RECENT);
  const live = recording.recording ? meetings.find((m) => m.id === recording.meeting_id) : undefined;
  // Usage numbers once there's something to count.
  const used = insights && (insights.dictation.count > 0 || insights.meetings.count > 0) ? insights : null;

  return (
    <div className="page home">
      <header className="home-head">
        <Wordmark />
        <p className="home-date">{now.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })}</p>
        <h1 className="home-title">{dayLine(today)}</h1>
        <p className="home-sub">
          {used && used.dictation.words_last_7_days > 0 && (
            <>{used.dictation.words_last_7_days.toLocaleString()} words dictated this week. </>
          )}
          {shortcut &&
            (shortcut.enabled ? (
              dictation === "listening" ? (
                "Listening…"
              ) : dictation === "processing" ? (
                "Typing it out…"
              ) : (
                <>
                  Hold <Keys hotkey={shortcut.hotkey} /> in any app to dictate.
                </>
              )
            ) : (
              <>
                Dictation is off.{" "}
                <button className="link" onClick={onDictationSettings}>
                  Turn it on
                </button>
              </>
            ))}
        </p>
      </header>

      <ModelDownload variant="banner" />
      <UpdateBanner />

      {recording.recording && recording.meeting_id && (
        <button className="home-live" onClick={() => recording.meeting_id && onSelect(recording.meeting_id)}>
          <span className="rec-dot pulsing" />
          <span className="home-live-title">Recording{live ? ` ${live.title}` : ""}</span>
          <span className="home-live-time">{clock(elapsed)}</span>
          <span className="home-live-open">
            Open <Icon name="external" size={12} />
          </span>
        </button>
      )}

      {featured && <LatestNotes meeting={featured} onOpen={() => onSelect(featured.id)} />}

      <section className="home-section">
        <div className="home-section-head">
          <h2>{featured ? "Before that" : "Meetings"}</h2>
          {meetings.length > recent.length + (featured ? 1 : 0) && (
            <button className="link-button" onClick={onMeetings}>
              All meetings
            </button>
          )}
        </div>
        {meetings.length === 0 ? (
          <NoMeetings />
        ) : (
          <ul className="quiet-list">
            {recent.map((m) => (
              <li key={m.id}>
                <button onClick={() => onSelect(m.id)}>
                  <span className="quiet-title">
                    {m.title}
                    <Status meeting={m} progress={progress[m.id]} recording={recording.meeting_id === m.id} />
                  </span>
                  <span className="quiet-length">{m.duration_ms > 0 ? minutes(m.duration_ms) : ""}</span>
                  <span className="quiet-when">{relativeDay(m.started_at, now)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {used && (
        <section className="home-section">
          <div className="home-section-head">
            <h2>Your usage</h2>
          </div>
          <UsageFacts insights={used} />
          <InsightDetails insights={used} onReset={loadInsights} />
        </section>
      )}
    </div>
  );
}

/** The latest meeting with notes: its TL;DR and action items, read from the summary. */
function LatestNotes({ meeting, onOpen }: { meeting: Meeting; onOpen: () => void }) {
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api
      .getMeeting(meeting.id)
      .then(setDetail)
      .catch((e) => setError(errorText(e)));
  }, [meeting.id]);

  const markdown = detail?.summaries[0]?.markdown ?? "";
  const tldr = section(markdown, /^tl;?dr/i);
  const actions = section(markdown, /^action items/i);
  const people = detail ? callSpeakers(detail.segments) : [];
  const now = new Date();

  return (
    <article className="latest">
      <header className="latest-head">
        <p className="latest-label">Latest notes</p>
        <button className="latest-title" onClick={onOpen}>
          {meeting.title}
        </button>
        <p className="latest-meta">
          {relativeDay(meeting.started_at, now, true)}
          {meeting.duration_ms > 0 && <span className="dot-sep">{minutes(meeting.duration_ms)}</span>}
          {people.length > 0 && <span className="dot-sep">{peopleText(people)}</span>}
        </p>
      </header>
      {error && <p className="error-text small">{error}</p>}
      {tldr && (
        <div className="latest-body">
          <Markdown remarkPlugins={[remarkGfm]}>{tldr}</Markdown>
        </div>
      )}
      {actions && (
        <div className="latest-body latest-actions">
          <h3>Action items</h3>
          <Markdown remarkPlugins={[remarkGfm]}>{actions}</Markdown>
        </div>
      )}
      <button className="latest-open" onClick={onOpen}>
        Read the notes <Icon name="external" size={12} />
      </button>
    </article>
  );
}

/** The body of a summary's `## heading` section whose title matches. */
function section(markdown: string, title: RegExp): string | null {
  for (const part of markdown.split(/^## /m).slice(1)) {
    const newline = part.indexOf("\n");
    if (title.test(part.slice(0, newline).trim())) return part.slice(newline + 1).trim() || null;
  }
  return null;
}

const COUNTS = ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine"];

/** "Three meetings today, 1 h 12 min on calls." */
function dayLine(today: Meeting[]): string {
  if (today.length === 0) return "No meetings today.";
  const count = COUNTS[today.length] ?? String(today.length);
  const time = today.reduce((sum, m) => sum + m.duration_ms, 0);
  const noun = today.length === 1 ? "meeting" : "meetings";
  return time >= 60_000 ? `${count} ${noun} today, ${minutes(time)} on calls.` : `${count} ${noun} today.`;
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** "4:05 PM" today, then "Yesterday", "Monday", "Oct 4"; `withTime` adds the time to all but today's. */
function relativeDay(rfc3339: string, now: Date, withTime = false): string {
  const date = new Date(rfc3339);
  const time = timeOfDay(rfc3339);
  if (sameDay(date, now)) return withTime ? `Today at ${time}` : time;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const ago = (now.getTime() - date.getTime()) / 86_400_000;
  const day = sameDay(date, yesterday)
    ? "Yesterday"
    : ago < 6
      ? date.toLocaleDateString(undefined, { weekday: "long" })
      : date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return withTime ? `${day} at ${time}` : day;
}
