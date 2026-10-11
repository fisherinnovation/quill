import { useCallback, useEffect, useState } from "react";
import { api, errorText, events, type Meeting, type Progress, type RecordingState } from "./api";
import { Sidebar } from "./components/Sidebar";
import { MeetingView } from "./components/MeetingView";
import { SettingsView } from "./components/SettingsView";
import { RecordButton } from "./components/RecordButton";
import { Onboarding } from "./components/Onboarding";

type View = { kind: "meeting"; id: string; at?: number } | { kind: "settings" } | { kind: "empty" };

export function App() {
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [view, setView] = useState<View>({ kind: "empty" });
  const [error, setError] = useState<string | null>(null);
  const [recording, setRecording] = useState<RecordingState>({
    recording: false,
    meeting_id: null,
    elapsed_ms: 0,
  });
  // Latest progress per meeting (absent = idle).
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  // Bumped when a meeting's data changes on the Rust side, so views refetch.
  const [version, setVersion] = useState(0);
  // null until settings load; false shows the first-run screen.
  const [onboarded, setOnboarded] = useState<boolean | null>(null);

  const refresh = useCallback(async () => {
    try {
      setMeetings(await api.listMeetings());
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    void api.recordingState().then(setRecording);
    api
      .getSettings()
      .then((s) => setOnboarded(s.onboarded))
      .catch(() => setOnboarded(true));
  }, [refresh]);

  // Subscribe to Rust events for the app's lifetime.
  useEffect(() => {
    const unlisten = [
      events.onRecording((state) => {
        setRecording(state);
        if (state.recording && state.meeting_id) {
          // Started (possibly from the menu bar): show the new meeting.
          setView({ kind: "meeting", id: state.meeting_id });
        }
        void refresh();
      }),
      events.onProgress((p) => {
        setProgress((all) => ({ ...all, [p.meeting_id]: p }));
        if (p.stage === "done" || p.stage === "failed") {
          setVersion((v) => v + 1);
          void refresh();
        }
      }),
    ];
    return () => {
      unlisten.forEach((u) => void u.then((f) => f()));
    };
  }, [refresh]);

  // Open the newest meeting on first load.
  useEffect(() => {
    if (view.kind === "empty" && meetings.length > 0) {
      setView({ kind: "meeting", id: meetings[0].id });
    }
  }, [meetings, view.kind]);

  if (onboarded === null) return null;
  if (!onboarded) return <Onboarding onDone={() => setOnboarded(true)} />;

  return (
    <div className="app">
      <Sidebar
        meetings={meetings}
        selectedId={view.kind === "meeting" ? view.id : null}
        progress={progress}
        recordingId={recording.meeting_id}
        onSelect={(id, at) => setView({ kind: "meeting", id, at })}
        onSettings={() => setView({ kind: "settings" })}
        header={<RecordButton state={recording} onError={setError} />}
      />
      <main className="main">
        {error && (
          <div className="banner error" role="alert">
            <span>{error}</span>
            <button onClick={() => setError(null)}>Dismiss</button>
          </div>
        )}
        {view.kind === "meeting" && (
          <MeetingView
            key={view.id}
            id={view.id}
            jumpTo={view.at}
            version={version}
            recording={recording.meeting_id === view.id ? recording : null}
            progress={progress[view.id]}
            onChanged={refresh}
            onDeleted={async () => {
              setView({ kind: "empty" });
              await refresh();
            }}
          />
        )}
        {view.kind === "settings" && <SettingsView />}
        {view.kind === "empty" && (
          <div className="empty">
            <h2>No meetings yet</h2>
            <p>Press Record (or use the menu-bar icon) when your meeting starts.</p>
          </div>
        )}
      </main>
    </div>
  );
}
