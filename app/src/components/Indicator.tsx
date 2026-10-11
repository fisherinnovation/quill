import { useEffect, useState } from "react";
import { api, events, type DictationState } from "../api";

// Relative heights of the meter's bars: tallest in the middle.
const BARS = [0.5, 0.75, 1, 0.85, 0.6];

/**
 * The floating dictation pill (its own transparent, unfocusable window; see
 * src-tauri/src/indicator.rs). Shows a live mic meter while listening and
 * "Typing…" while the text is processed.
 */
export function Indicator() {
  const [state, setState] = useState<DictationState>({ phase: "idle", error: null });
  const [level, setLevel] = useState(0);

  useEffect(() => {
    document.documentElement.classList.add("indicator-window");
    void api.dictationState().then(setState);
    const unlisten = [events.onDictation(setState), events.onLevel(setLevel)];
    return () => unlisten.forEach((u) => void u.then((f) => f()));
  }, []);

  // Peak level → 0..1 on a log scale: -48 dBFS (room noise) to -6 dBFS (loud).
  const db = 20 * Math.log10(Math.max(level, 1e-6));
  const loudness = Math.min(1, Math.max(0, (db + 48) / 42));

  return (
    <div className="indicator-stage">
      <div className={`indicator ${state.phase}`} role="status" aria-live="polite">
        {state.phase === "processing" ? (
          <>
            <span className="spinner" />
            <span>Typing…</span>
          </>
        ) : (
          <>
            <span className="rec-dot" />
            <span className="meter" aria-label="Listening">
              {BARS.map((weight, i) => (
                <span key={i} style={{ transform: `scaleY(${Math.max(0.15, loudness * weight)})` }} />
              ))}
            </span>
          </>
        )}
      </div>
    </div>
  );
}
