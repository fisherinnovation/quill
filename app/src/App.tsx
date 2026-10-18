import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  errorText,
  events,
  type DictationState,
  type Meeting,
  type Progress,
  type RecordingState,
} from "./api";
import { type Section, Sidebar } from "./components/Sidebar";
import { MeetingView } from "./components/MeetingView";
import { MeetingsView } from "./components/MeetingsView";
import { type SettingsTab, SettingsView } from "./components/SettingsView";
import { RecordButton } from "./components/RecordButton";
import { Onboarding } from "./components/Onboarding";
import { DictationsView } from "./components/DictationsView";
import { HelpView } from "./components/HelpView";
import { SamplesReview } from "./components/SamplesReview";
import { HomeView } from "./components/HomeView";
import { DictationEntryView } from "./components/DictationHistory";
import { Banner } from "./components/ui";
import { type Toast, Toasts } from "./components/Toasts";

type View =
  | { kind: "home" }
  | { kind: "meetings" }
  | { kind: "meeting"; id: string; at?: number }
  /** The Dictations page ("dictation", as the backend's navigate event names it). */
  | { kind: "dictation" }
  | { kind: "dictation-entry"; id: number }
  | { kind: "help" }
  /** Reviewing saved dictation samples (from Settings → Dictation). */
  | { kind: "samples" }
  | { kind: "settings"; tab?: SettingsTab };

/** How long a finished write-up's outcome shows on its meeting. */
const WRITTEN_SHOWN_MS = 15 * 60_000;

/** Where the sidebar's collapsed state is remembered, on this computer. */
const COLLAPSED_KEY = "quill.sidebarCollapsed";

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

/** The sidebar item a view belongs to. */
function section(view: View): Section {
  switch (view.kind) {
    case "meeting":
      return "meetings";
    case "dictation-entry":
      return "dictation";
    case "samples":
      return "settings";
    default:
      return view.kind;
  }
}

export function App() {
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [view, setView] = useState<View>({ kind: "home" });
  const [error, setError] = useState<string | null>(null);
  const [recording, setRecording] = useState<RecordingState>({
    recording: false,
    paused: false,
    meeting_id: null,
    elapsed_ms: 0,
  });
  const [dictation, setDictation] = useState<DictationState>({ phase: "idle", error: null });
  // Latest progress per meeting (absent = idle).
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  // Write-ups finished this session, and when: the meeting shows what came
  // out for a while (until dismissed).
  const [written, setWritten] = useState<Record<string, number>>({});
  // Written up while the user was looking elsewhere, and not opened since.
  const [fresh, setFresh] = useState<Set<string>>(new Set());
  const [toasts, setToasts] = useState<Toast[]>([]);
  // Bumped when a meeting's data changes on the Rust side, so views refetch.
  const [version, setVersion] = useState(0);
  const [collapsed, setCollapsed] = useState(readCollapsed);
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
          // A write-up (transcribing or notes; not re-splitting speakers).
          const writeUp = p.report?.steps.some((s) => s.kind !== "speakers" && s.status !== "skipped");
          if (!writeUp) return;
          const v = viewRef.current;
          const looking = v.kind === "meeting" && v.id === p.meeting_id && document.hasFocus();
          if (p.stage === "done") setWritten((w) => ({ ...w, [p.meeting_id]: Date.now() }));
          if (!looking) {
            if (p.stage === "done") setFresh((f) => new Set(f).add(p.meeting_id));
            const toast: Toast = {
              id: p.meeting_id,
              kind: p.stage === "done" ? "done" : "failed",
              title: p.title ?? "Meeting",
              outcome: p.report?.outcome ?? null,
              message: p.message,
            };
            setToasts((all) => [...all.filter((t) => t.id !== toast.id), toast]);
          }
        }
      }),
      events.onDictation(setDictation),
      // Meetings deleted elsewhere (e.g. Settings → Storage): leave a
      // meeting that's gone for Home.
      events.onLibraryChanged(async () => {
        const list = await api.listMeetings();
        setMeetings(list);
        setView((v) => (v.kind === "meeting" && !list.some((m) => m.id === v.id) ? { kind: "meetings" } : v));
      }),
      events.onNavigate((view) => {
        if (view.startsWith("meeting:")) {
          // From a notification or the wrap-up card.
          setView({ kind: "meeting", id: view.slice("meeting:".length) });
        } else if (view === "home" || view === "dictation" || view === "help" || view === "settings") {
          setView({ kind: view });
        } else if (view === "insights") {
          // Insights are part of Home now.
          setView({ kind: "home" });
        }
      }),
    ];
    return () => {
      unlisten.forEach((u) => void u.then((f) => f()));
    };
  }, [refresh]);

  // Which page is showing, for the progress handler; opening a meeting
  // clears its "New".
  const viewRef = useRef(view);
  useEffect(() => {
    viewRef.current = view;
    if (view.kind === "meeting") {
      const id = view.id;
      setFresh((f) => {
        if (!f.has(id)) return f;
        const next = new Set(f);
        next.delete(id);
        return next;
      });
      setToasts((all) => all.filter((t) => t.id !== id));
    }
  }, [view]);

  // Collapse or expand the sidebar: ⌘\ (Ctrl+\ on Windows), remembered.
  const toggleSidebar = useCallback(() => {
    setCollapsed((was) => {
      try {
        localStorage.setItem(COLLAPSED_KEY, was ? "0" : "1");
      } catch {
        // Not remembered, but still toggled.
      }
      return !was;
    });
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key === "\\") {
        e.preventDefault();
        toggleSidebar();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleSidebar]);

  if (onboarded === null) return null;
  if (!onboarded) return <Onboarding onDone={() => setOnboarded(true)} />;

  return (
    <div className={`app${collapsed ? " sidebar-collapsed" : ""}`}>
      <Sidebar
        current={section(view)}
        onNavigate={(kind) => setView({ kind })}
        recording={recording.recording}
        working={Object.values(progress).filter((p) => p.stage !== "done" && p.stage !== "failed").length}
        dictation={dictation.phase}
        collapsed={collapsed}
        onToggle={toggleSidebar}
        header={<RecordButton state={recording} onError={setError} compact={collapsed} />}
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
              justWritten={written[view.id] !== undefined && Date.now() - written[view.id] < WRITTEN_SHOWN_MS}
              onDismissWriteUp={() =>
                setWritten((w) => {
                  const { [view.id]: _, ...rest } = w;
                  return rest;
                })
              }
              onChanged={refresh}
              onDeleted={async () => {
                setView({ kind: "meetings" });
                await refresh();
              }}
              onBack={() => setView({ kind: "meetings" })}
            />
          )}
          {view.kind === "meetings" && (
            <MeetingsView
              meetings={meetings}
              progress={progress}
              recording={recording}
              fresh={fresh}
              onSelect={(id, at) => setView({ kind: "meeting", id, at })}
            />
          )}
          {view.kind === "help" && <HelpView />}
          {view.kind === "settings" && (
            <SettingsView
              key={view.tab ?? "general"}
              tab={view.tab}
              onReviewSamples={() => setView({ kind: "samples" })}
            />
          )}
          {view.kind === "samples" && <SamplesReview onBack={() => setView({ kind: "settings", tab: "dictation" })} />}
          {view.kind === "dictation" && (
            <DictationsView
              state={dictation}
              onOpen={(id) => setView({ kind: "dictation-entry", id })}
              onSettings={() => setView({ kind: "settings", tab: "dictation" })}
            />
          )}
          {view.kind === "dictation-entry" && (
            <DictationEntryView
              key={view.id}
              id={view.id}
              onDeleted={() => setView({ kind: "dictation" })}
              onBack={() => setView({ kind: "dictation" })}
            />
          )}
          {view.kind === "home" && (
            <HomeView
              meetings={meetings}
              progress={progress}
              recording={recording}
              dictation={dictation.phase}
              fresh={fresh}
              onSelect={(id) => setView({ kind: "meeting", id })}
              onMeetings={() => setView({ kind: "meetings" })}
              onDictationSettings={() => setView({ kind: "settings", tab: "dictation" })}
            />
          )}
        </div>
        <Toasts
          toasts={toasts}
          onOpen={(id) => setView({ kind: "meeting", id })}
          onDismiss={(id) => setToasts((all) => all.filter((t) => t.id !== id))}
        />
      </main>
    </div>
  );
}
