import { useEffect, useRef, useState, type MouseEvent, type PointerEvent } from "react";
import {
  api,
  clock,
  errorText,
  events,
  type LiveItem,
  type LiveState,
  type Progress,
  type RecordingState,
} from "../api";
import { Icon, Logo } from "./Icon";
import { useElapsed } from "./RecordButton";
import { WriteUpCard } from "./WriteUp";

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
  wrapping: null,
};

/** A press that moves this far (CSS pixels) is a drag, not a click. */
const DRAG_AFTER = 4;

/**
 * Drags the window by an element (the header, the tab, the card): the
 * window follows the pointer and goes there from now on. A press that
 * doesn't move is still a click, so buttons inside work; double-clicking
 * outside them puts the panel and the tab back at the right edge.
 */
function useWindowDrag() {
  const drag = useRef<{ x: number; y: number; dx: number; dy: number; moved: boolean; frame: number } | null>(null);
  // The click that ends a drag isn't one.
  const dragged = useRef(false);

  function end() {
    const d = drag.current;
    drag.current = null;
    if (!d?.moved) return;
    cancelAnimationFrame(d.frame);
    void api.liveNotesDrag(d.dx, d.dy);
    void api.liveNotesDragEnd();
    dragged.current = true;
  }

  return {
    onPointerDown(e: PointerEvent<HTMLElement>) {
      if (e.button !== 0) return;
      drag.current = { x: e.screenX, y: e.screenY, dx: 0, dy: 0, moved: false, frame: 0 };
    },
    onPointerMove(e: PointerEvent<HTMLElement>) {
      const d = drag.current;
      if (!d) return;
      d.dx = e.screenX - d.x;
      d.dy = e.screenY - d.y;
      if (!d.moved) {
        if (Math.hypot(d.dx, d.dy) < DRAG_AFTER) return;
        d.moved = true;
        // Only now: capturing on the press would take clicks from buttons.
        e.currentTarget.setPointerCapture(e.pointerId);
        void api.liveNotesDragStart();
      }
      // One move a frame: the window moves in Rust.
      d.frame ||= requestAnimationFrame(() => {
        d.frame = 0;
        void api.liveNotesDrag(d.dx, d.dy);
      });
    },
    onPointerUp: end,
    onPointerCancel: end,
    onClickCapture(e: MouseEvent<HTMLElement>) {
      if (!dragged.current) return;
      dragged.current = false;
      e.preventDefault();
      e.stopPropagation();
    },
    onDoubleClick(e: MouseEvent<HTMLElement>) {
      if (!(e.target as Element).closest("button, a")) void api.liveNotesResetPlace();
    },
  };
}

/**
 * Live notes during a recording (its own window at the right edge of the
 * screen, hidden from screen sharing; see src-tauri/src/live_notes.rs):
 * what to ask before the end, follow-ups and decisions, updated every
 * minute, with the recording's timer and Pause and Stop. Collapses to a
 * tab showing how many questions are open. Dragged by its header, the
 * tab or the card, it stays where it's put.
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
  const dragHandle = useWindowDrag();

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

  const open = state.notes.items.filter((i) => i.kind === "ask" && !i.done).length;

  if (state.wrapping) return <WrapUp key={state.wrapping.meeting_id} wrapping={state.wrapping} />;

  if (state.collapsed) {
    return (
      <button
        className="live-tab"
        {...dragHandle}
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
    <div className="live-stage">
      <section className="live-panel" aria-label="Live notes">
        <header className="live-head" {...dragHandle}>
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

/** How long the card stays once the notes are ready (unless pointed at). */
const READY_STAYS_MS = 45_000;

/**
 * After the recording stops, the panel follows the write-up as a small
 * card at the same edge of the screen: the steps, then "Notes ready" with
 * Open notes. It fits the panel to itself, and goes away a while after
 * the notes are ready, or when closed.
 */
function WrapUp({ wrapping }: { wrapping: NonNullable<LiveState["wrapping"]> }) {
  const [progress, setProgress] = useState<Progress | null>(null);
  const [pointed, setPointed] = useState(false);
  const card = useRef<HTMLDivElement>(null);
  const dragHandle = useWindowDrag();

  useEffect(() => {
    const unlisten = events.onProgress((p) => {
      if (p.meeting_id === wrapping.meeting_id) setProgress(p);
    });
    return () => void unlisten.then((f) => f());
  }, [wrapping.meeting_id]);

  // The panel is as tall as the card.
  useEffect(() => {
    const el = card.current;
    if (!el) return;
    let last = 0;
    const observer = new ResizeObserver(() => {
      // The stage's padding around the card, top and bottom.
      const height = Math.ceil(el.getBoundingClientRect().height) + 16;
      if (Math.abs(height - last) > 2) {
        last = height;
        void api.liveNotesFit(height);
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const done = progress?.stage === "done";
  useEffect(() => {
    if (!done || pointed) return;
    const timer = setTimeout(() => void api.liveNotesClose(), READY_STAYS_MS);
    return () => clearTimeout(timer);
  }, [done, pointed]);

  const shown: Progress = progress ?? {
    meeting_id: wrapping.meeting_id,
    stage: "transcribing",
    fraction: null,
    message: null,
    report: null,
    title: null,
  };
  const open = () => void api.liveNotesOpenMeeting();
  return (
    <div className="live-stage wrapup-stage" onMouseEnter={() => setPointed(true)} onMouseLeave={() => setPointed(false)}>
      <div ref={card} className="live-drag" {...dragHandle}>
        <WriteUpCard
          compact
          title={wrapping.title}
          progress={shown}
          durationMs={0}
          onRetry={open}
          onDismiss={() => void api.liveNotesClose()}
          onOpen={open}
        />
      </div>
    </div>
  );
}
