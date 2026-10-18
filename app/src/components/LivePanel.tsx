import { useEffect, useRef, useState } from "react";
import { api, clock, errorText, events, type LiveItem, type LiveState, type RecordingState } from "../api";
import { Icon, Logo } from "./Icon";
import { useElapsed } from "./RecordButton";

const SECTIONS: [LiveItem["kind"], string][] = [
  ["ask", "Ask before the end"],
  ["follow_up", "Follow-ups"],
  ["decision", "Decisions"],
];

const EMPTY: LiveState = {
  active: false,
  notes: { gist: "", items: [] },
  updated_ms: null,
  next_ms: null,
  updating: false,
  heard: null,
  error: null,
  collapsed: false,
};

/**
 * Live notes during a recording (its own window at the right edge of the
 * screen, hidden from screen sharing; see src-tauri/src/live_notes.rs):
 * what to ask before the end, follow-ups and decisions, updated every
 * minute, with the recording's timer and Pause and Stop. Collapses to a
 * tab showing how many questions are open.
 */
export function LivePanel() {
  const [state, setState] = useState<LiveState>(EMPTY);
  const [recording, setRecording] = useState<RecordingState>({
    recording: false,
    paused: false,
    meeting_id: null,
    elapsed_ms: 0,
  });
  const elapsed = useElapsed(recording);
  // Stop asks once ("Stop?") so a stray click doesn't end the meeting.
  const [confirmStop, setConfirmStop] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  // Recording time, ticking locally between updates, for "updated 1 min ago".
  const [now, setNow] = useState(0);
  const anchor = useRef<{ at: number; ms: number } | null>(null);
  // Items already shown don't animate in again.
  const seen = useRef(new Set<string>());

  useEffect(() => {
    document.documentElement.classList.add("live-window");
    void api.liveNotesState().then(setState);
    void api.recordingState().then(setRecording);
    const unlisten = [events.onLiveNotes(setState), events.onRecording(setRecording)];
    const timer = setInterval(() => setNow(Date.now()), 5_000);
    return () => {
      clearInterval(timer);
      unlisten.forEach((u) => void u.then((f) => f()));
    };
  }, []);

  useEffect(() => {
    for (const item of state.notes.items) seen.current.add(item.id);
  }, [state.notes.items]);

  useEffect(() => {
    if (!confirmStop) return;
    const timer = setTimeout(() => setConfirmStop(false), 3_000);
    return () => clearTimeout(timer);
  }, [confirmStop]);

  async function control(what: "pause" | "resume" | "stop") {
    setBusy(what);
    try {
      if (what === "pause") await api.pauseRecording();
      else if (what === "resume") await api.resumeRecording();
      else await api.stopRecording();
    } catch (e) {
      setState((s) => ({ ...s, error: errorText(e) }));
    } finally {
      setBusy(null);
      setConfirmStop(false);
    }
  }

  // The panel's buttons activate Quill when pressed (macOS): the backend
  // hands focus straight back to the call.
  const pressed = () => void api.liveNotesPressed();
  const hovered = () => void api.liveNotesHovered();
  const open = state.notes.items.filter((i) => i.kind === "ask" && !i.done).length;

  if (state.collapsed) {
    return (
      <button
        className="live-tab"
        onPointerDown={pressed}
        onMouseEnter={hovered}
        onClick={() => void api.liveNotesCollapse(false)}
        title="Show live notes"
      >
        <Icon name="chevron-left" size={14} />
        <span className="rec-dot pulsing" />
        {open > 0 && <span className="live-tab-count">{open}</span>}
      </button>
    );
  }

  const since = state.updated_ms;
  // Rough minutes since the last update and until the next.
  if (since !== null && (!anchor.current || anchor.current.ms !== since)) {
    anchor.current = { at: Date.now(), ms: since };
  }
  const sinceUpdate = anchor.current ? Math.max(0, (now || Date.now()) - anchor.current.at) : 0;
  const ago = since === null ? null : Math.round(sinceUpdate / 60_000);

  return (
    <div className="live-stage" onPointerDown={pressed} onMouseEnter={hovered}>
      <section className="live-panel" aria-label="Live notes">
        <header className="live-head">
          {recording.paused ? (
            <Icon name="pause" size={12} />
          ) : (
            <span className="rec-dot pulsing" />
          )}
          <strong className="live-clock">{recording.paused ? `Paused · ${clock(elapsed)}` : clock(elapsed)}</strong>
          <span className="spacer" />
          <button
            className="ghost icon-btn"
            title={recording.paused ? "Resume recording" : "Pause recording"}
            aria-label={recording.paused ? "Resume recording" : "Pause recording"}
            disabled={busy !== null || !recording.recording}
            onClick={() => void control(recording.paused ? "resume" : "pause")}
          >
            <Icon name={recording.paused ? "play" : "pause"} size={14} />
          </button>
          <button
            className={`live-stop${confirmStop ? " confirm" : ""}`}
            title="Stop recording and write the notes"
            disabled={busy !== null || !recording.recording}
            onClick={() => (confirmStop ? void control("stop") : setConfirmStop(true))}
          >
            <Icon name="stop" size={11} />
            {confirmStop ? "Stop?" : "Stop"}
          </button>
          <button
            className="ghost icon-btn"
            title="Collapse"
            aria-label="Collapse"
            onClick={() => void api.liveNotesCollapse(true)}
          >
            <Icon name="chevron-right" size={14} />
          </button>
        </header>

        <div className="live-body">
          {state.notes.gist ? (
            <p className="live-gist">{state.notes.gist}</p>
          ) : (
            <div className="live-waiting">
              <Logo size={28} />
              <p>{state.updating ? "Writing the first notes…" : "Listening. The first notes appear after about a minute."}</p>
            </div>
          )}

          {SECTIONS.map(([kind, title]) => {
            const items = state.notes.items
              .filter((i) => i.kind === kind)
              .sort((a, b) => Number(a.done) - Number(b.done));
            if (items.length === 0) return null;
            return (
              <div className={`live-section ${kind}`} key={kind}>
                <h3>{title}</h3>
                <ul>
                  {items.map((item) => (
                    <li key={item.id} className={`live-item${item.done ? " done" : ""}${seen.current.has(item.id) ? "" : " new"}`}>
                      {kind === "decision" ? (
                        // Decisions are facts, not to-dos: nothing to tick.
                        <span className="live-bullet" />
                      ) : (
                        <button
                          className="live-tick"
                          aria-label={item.done ? "Mark not done" : "Mark done"}
                          title={item.done ? "Mark not done" : "Mark done"}
                          onClick={() => void api.liveNotesTick(item.id, !item.done)}
                        >
                          {item.done && <Icon name="check" size={11} />}
                        </button>
                      )}
                      <span className="live-text">
                        {item.who && <span className="live-who">{item.who}</span>}
                        {item.text}
                      </span>
                      <button
                        className="ghost icon-btn live-dismiss"
                        aria-label="Dismiss"
                        title="Dismiss"
                        onClick={() => void api.liveNotesDismiss(item.id)}
                      >
                        <Icon name="x" size={12} />
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            );
          })}
        </div>

        <footer className="live-foot">
          <div className="live-status">
            {state.error ? (
              <span className="error-text">Couldn't update: {state.error}</span>
            ) : (
              <span>
                {state.updating
                  ? "Updating…"
                  : ago === null
                    ? "First notes in about a minute"
                    : `Updated ${ago < 1 ? "just now" : `${ago} min ago`}`}
              </span>
            )}
            <span className="spacer" />
            <button className="link-button" disabled={state.updating} onClick={() => void api.liveNotesRefresh()}>
              Update now
            </button>
          </div>
          {state.heard && <span className="live-heard">Heard: “{state.heard}”</span>}
        </footer>
      </section>
    </div>
  );
}
