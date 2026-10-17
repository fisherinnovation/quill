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
import { Icon } from "./Icon";
import { MeetingRow, NoMeetings } from "./MeetingsView";
import { ModelDownload } from "./Models";
import { useElapsed } from "./RecordButton";
import { Banner, Keys } from "./ui";
import { UpdateBanner } from "./Updates";

/** How many recent meetings Home lists. */
const RECENT = 6;

function greeting(now: Date): string {
  const h = now.getHours();
  return h < 5 ? "Good evening" : h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : "Good evening";
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
  onMeetings,
  onDictationSettings,
  onInsights,
}: {
  meetings: Meeting[];
  progress: Record<string, Progress>;
  recording: RecordingState;
  dictation: DictationPhase;
  onSelect: (id: string) => void;
  onMeetings: () => void;
  onDictationSettings: () => void;
  onInsights: () => void;
}) {
  const [insights, setInsights] = useState<Insights | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  // Read fresh each visit: they can change in Settings → Dictation.
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
      <UpdateBanner />
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
          <button onClick={onDictationSettings}>{dictationEnabled ? "Settings" : "Turn on"}</button>
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
        <div className="row-gap home-section-head">
          <h2 className="group-title">Recent meetings</h2>
          <span className="spacer" />
          {meetings.length > recent.length && (
            <button className="link-button" onClick={onMeetings}>
              All meetings
            </button>
          )}
        </div>
        {recent.length === 0 ? (
          <NoMeetings />
        ) : (
          <ul className="card item-list">
            {recent.map((m) => (
              <li key={m.id}>
                <MeetingRow
                  meeting={m}
                  progress={progress[m.id]}
                  recording={recording.meeting_id === m.id}
                  onClick={() => onSelect(m.id)}
                />
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
