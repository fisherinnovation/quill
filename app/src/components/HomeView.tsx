import { useEffect, useState } from "react";
import {
  api,
  clock,
  errorText,
  type DictationPhase,
  type Insights,
  type Meeting,
  minutes,
  type Progress,
  type RecordingState,
} from "../api";
import { Icon, Logo } from "./Icon";
import { ModelDownload } from "./Models";
import { useElapsed } from "./RecordButton";
import { Status } from "./Sidebar";
import { Banner, Keys } from "./ui";

/** How many recent meetings Home lists. */
const RECENT = 6;

function greeting(now: Date): string {
  const h = now.getHours();
  return h < 5 ? "Good evening" : h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : "Good evening";
}

/** "Mon, Oct 12 · 2:15 PM" in the user's locale. */
function shortWhen(rfc3339: string): string {
  const d = new Date(rfc3339);
  const day = d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return `${day} · ${time}`;
}

/**
 * Where Quill opens, and where it goes when there's nothing else to show
 * (after deleting a meeting): quick actions, the week at a glance and the
 * latest meetings.
 */
export function HomeView({
  meetings,
  progress,
  recording,
  dictation,
  onSelect,
  onDictation,
  onInsights,
}: {
  meetings: Meeting[];
  progress: Record<string, Progress>;
  recording: RecordingState;
  dictation: DictationPhase;
  onSelect: (id: string) => void;
  onDictation: () => void;
  onInsights: () => void;
}) {
  const [insights, setInsights] = useState<Insights | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  // Read fresh each visit: they can change on the Dictation page.
  const [shortcut, setShortcut] = useState<{ hotkey: string; enabled: boolean } | null>(null);
  const elapsed = useElapsed(recording);

  useEffect(() => {
    void api.getInsights().then(setInsights).catch(() => setInsights(null));
  }, [meetings.length]);
  useEffect(() => {
    void api.getSettings().then((s) => setShortcut({ hotkey: s.dictation.hotkey, enabled: s.dictation.enabled }));
  }, []);
  const dictationEnabled = shortcut?.enabled ?? true;

  async function record() {
    setStarting(true);
    setError(null);
    try {
      // The recording-state event then opens the new meeting.
      await api.startRecording();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setStarting(false);
    }
  }

  const now = new Date();
  const recent = meetings.slice(0, RECENT);
  const week = insights && {
    meetings: insights.meetings.count_last_7_days,
    meetingMs: insights.meetings.ms_last_7_days,
    words: insights.dictation.words_last_7_days,
  };

  return (
    <div className="page home">
      <header className="home-head">
        <h1 className="page-title">{greeting(now)}</h1>
        <p className="page-subtitle">
          {now.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })}
        </p>
      </header>

      <ModelDownload variant="banner" />
      {error && (
        <Banner kind="error" icon="alert">
          {error}
        </Banner>
      )}

      <div className="home-actions">
        <div className={`home-card${recording.recording ? " live" : ""}`}>
          <div className="home-card-icon record">
            <span className={`rec-dot${recording.recording ? " pulsing" : ""}`} />
          </div>
          <div className="home-card-body">
            <strong>{recording.recording ? `Recording · ${clock(elapsed)}` : "Record a meeting"}</strong>
            <p>
              {recording.recording
                ? "Quill is capturing your microphone and the call."
                : "Captures your microphone and the call, then writes the notes. No bot joins."}
            </p>
          </div>
          {recording.recording && recording.meeting_id ? (
            <button onClick={() => recording.meeting_id && onSelect(recording.meeting_id)}>Open</button>
          ) : (
            <button className="primary" disabled={starting} onClick={() => void record()}>
              {starting ? "Starting…" : "Record"}
            </button>
          )}
        </div>

        <div className="home-card">
          <div className="home-card-icon">
            <Icon name="mic" size={17} />
          </div>
          <div className="home-card-body">
            <strong>
              {dictation === "listening" ? "Listening…" : dictation === "processing" ? "Typing it out…" : "Dictate anywhere"}
            </strong>
            <p>
              {dictationEnabled ? (
                <>
                  Hold <Keys hotkey={shortcut?.hotkey ?? ""} /> in any app and speak.
                </>
              ) : (
                "Dictation is off."
              )}
            </p>
          </div>
          <button onClick={onDictation}>{dictationEnabled ? "Settings" : "Turn on"}</button>
        </div>
      </div>

      {week && (week.meetings > 0 || week.words > 0) && (
        <section className="group">
          <div className="row-gap home-section-head">
            <h2 className="group-title">This week</h2>
            <span className="spacer" />
            <button className="link-button" onClick={onInsights}>
              More insights
            </button>
          </div>
          <div className="home-stats">
            <div className="home-stat">
              <span className="home-stat-value">{week.meetings}</span>
              <span className="home-stat-label">{week.meetings === 1 ? "meeting" : "meetings"}</span>
            </div>
            <div className="home-stat">
              <span className="home-stat-value">{minutes(week.meetingMs)}</span>
              <span className="home-stat-label">in meetings</span>
            </div>
            <div className="home-stat">
              <span className="home-stat-value">{week.words.toLocaleString()}</span>
              <span className="home-stat-label">words dictated</span>
            </div>
          </div>
        </section>
      )}

      <section className="group">
        <h2 className="group-title">Recent meetings</h2>
        {recent.length === 0 ? (
          <div className="card home-empty">
            <Logo size={40} />
            <div>
              <strong>Your meetings will appear here</strong>
              <p className="muted small">
                Press Record when a call starts, here, in the sidebar or from the menu-bar icon. Works with Zoom,
                Meet, Teams or any other call.
              </p>
            </div>
          </div>
        ) : (
          <ul className="card home-meetings">
            {recent.map((m) => (
              <li key={m.id}>
                <button className="home-meeting" onClick={() => onSelect(m.id)}>
                  <span className="home-meeting-title">{m.title}</span>
                  <span className="home-meeting-meta">
                    {shortWhen(m.started_at)}
                    {m.duration_ms > 0 && <span className="dot-sep">{minutes(m.duration_ms)}</span>}
                    <Status meeting={m} progress={progress[m.id]} recording={recording.meeting_id === m.id} />
                  </span>
                  <Icon name="external" size={13} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
