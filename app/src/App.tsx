import { useCallback, useEffect, useState } from "react";
import {
  api,
  errorText,
  events,
  type DictationState,
  type Meeting,
  type Progress,
  type RecordingState,
} from "./api";
import { Sidebar } from "./components/Sidebar";
import { MeetingView } from "./components/MeetingView";
import { SettingsView } from "./components/SettingsView";
import { RecordButton } from "./components/RecordButton";
import { Onboarding } from "./components/Onboarding";
import { DictationView } from "./components/DictationView";
import { HomeView } from "./components/HomeView";
import { InsightsView } from "./components/InsightsView";
import { DictationEntryView, useDictationHistory } from "./components/DictationHistory";
import { Banner } from "./components/ui";

type View =
  | { kind: "meeting"; id: string; at?: number }
  | { kind: "dictation-entry"; id: number }
  | { kind: "settings" }
  | { kind: "dictation" }
  | { kind: "insights" }
  | { kind: "home" };

export function App() {
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [view, setView] = useState<View>({ kind: "home" });
  const [error, setError] = useState<string | null>(null);
  const [recording, setRecording] = useState<RecordingState>({
    recording: false,
    meeting_id: null,
    elapsed_ms: 0,
  });
  const [dictation, setDictation] = useState<DictationState>({ phase: "idle", error: null });
  // Latest progress per meeting (absent = idle).
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  // Bumped when a meeting's data changes on the Rust side, so views refetch.
  const [version, setVersion] = useState(0);
  // The dictation history listed in the sidebar.
  const dictations = useDictationHistory();
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
    void api.dictationState().then(setDictation);
    api
      .getSettings()
      .then((s) => {
        setOnboarded(s.onboarded);
      })
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
      events.onDictation(setDictation),
      // Meetings deleted elsewhere (e.g. Settings → Storage): leave a
      // meeting that's gone for Home.
      events.onLibraryChanged(async () => {
        const list = await api.listMeetings();
        setMeetings(list);
        setView((v) => (v.kind === "meeting" && !list.some((m) => m.id === v.id) ? { kind: "home" } : v));
      }),
      events.onNavigate((view) => {
        if (view === "home" || view === "dictation" || view === "settings" || view === "insights") {
          setView({ kind: view });
        }
      }),
    ];
    return () => {
      unlisten.forEach((u) => void u.then((f) => f()));
    };
  }, [refresh]);

  if (onboarded === null) return null;
  if (!onboarded) return <Onboarding onDone={() => setOnboarded(true)} />;

  return (
    <div className="app">
      <Sidebar
        meetings={meetings}
        selectedId={view.kind === "meeting" ? view.id : null}
        dictations={dictations}
        selectedDictation={view.kind === "dictation-entry" ? view.id : null}
        onSelectDictation={(id) => setView({ kind: "dictation-entry", id })}
        progress={progress}
        recordingId={recording.meeting_id}
        dictation={dictation.phase}
        onSelect={(id, at) => setView({ kind: "meeting", id, at })}
        onSettings={() => setView({ kind: "settings" })}
        onHome={() => setView({ kind: "home" })}
        onDictation={() => setView({ kind: "dictation" })}
        onInsights={() => setView({ kind: "insights" })}
        current={view.kind}
        header={<RecordButton state={recording} onError={setError} />}
      />
      <main className="main">
        <div className="drag" data-tauri-drag-region />
        {error && (
          <div className="app-error">
            <Banner
              kind="error"
              icon="alert"
              action={
                <button className="ghost" onClick={() => setError(null)}>
                  Dismiss
                </button>
              }
            >
              {error}
            </Banner>
          </div>
        )}
        <div className="main-scroll">
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
                setView({ kind: "home" });
                await refresh();
              }}
            />
          )}
          {view.kind === "settings" && <SettingsView />}
          {view.kind === "insights" && <InsightsView />}
          {view.kind === "dictation" && <DictationView state={dictation} onCleared={dictations.clear} />}
          {view.kind === "dictation-entry" && (
            <DictationEntryView
              key={view.id}
              id={view.id}
              onChanged={dictations.replace}
              onDeleted={(id) => {
                dictations.remove(id);
                setView({ kind: "home" });
              }}
              onOpenDictation={() => setView({ kind: "dictation" })}
            />
          )}
          {view.kind === "home" && (
            <HomeView
              meetings={meetings}
              progress={progress}
              recording={recording}
              dictation={dictation.phase}
              onSelect={(id) => setView({ kind: "meeting", id })}
              onDictation={() => setView({ kind: "dictation" })}
              onInsights={() => setView({ kind: "insights" })}
            />
          )}
        </div>
      </main>
    </div>
  );
}
