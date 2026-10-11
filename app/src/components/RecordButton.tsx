import { useEffect, useState } from "react";
import { api, clock, errorText, type RecordingState } from "../api";

interface Props {
  state: RecordingState;
  onError: (message: string) => void;
}

/** Record / Stop with a live timer. */
export function RecordButton({ state, onError }: Props) {
  const [busy, setBusy] = useState(false);
  const elapsed = useElapsed(state);

  async function toggle() {
    setBusy(true);
    try {
      if (state.recording) await api.stopRecording();
      else await api.startRecording();
    } catch (e) {
      onError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <button className={`record${state.recording ? " active" : ""}`} onClick={toggle} disabled={busy}>
      {state.recording ? (
        <>
          <span className="stop-square" /> Stop · {clock(elapsed)}
        </>
      ) : busy ? (
        "Starting…"
      ) : (
        <>
          <span className="rec-dot" /> Record
        </>
      )}
    </button>
  );
}

/** Milliseconds recorded so far, ticking locally between state updates. */
export function useElapsed(state: RecordingState): number {
  const [now, setNow] = useState(() => Date.now());
  // Last known elapsed time from Rust, and when we learned it.
  const [anchor, setAnchor] = useState(() => ({ at: Date.now(), elapsed: state.elapsed_ms }));

  useEffect(() => {
    setAnchor({ at: Date.now(), elapsed: state.elapsed_ms });
  }, [state.elapsed_ms, state.recording]);

  useEffect(() => {
    if (!state.recording) return;
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [state.recording]);

  return state.recording ? anchor.elapsed + Math.max(0, now - anchor.at) : 0;
}
