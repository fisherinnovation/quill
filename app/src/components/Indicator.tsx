import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { api, type Caption, events, type DictationState } from "../api";

// Relative heights of the meter's bars: tallest in the middle.
const BARS = [0.5, 0.75, 1, 0.85, 0.6];

/** Words kept in the captions (only the last two lines show). */
const CAPTION_WORDS = 48;
/** Caption lines: 14.5 px text at 1.45 line height. */
const LINE_PX = 21;
const MAX_LINES = 2;

/**
 * The floating dictation pill (its own transparent, unfocusable window; see
 * src-tauri/src/indicator.rs). Shows a live mic meter while listening and
 * "Typing…" while the text is processed, with captions of what's been said
 * above it (Settings → Dictation → Show what you're saying).
 */
export function Indicator() {
  const [state, setState] = useState<DictationState>({ phase: "idle", error: null });
  const [level, setLevel] = useState(0);
  const [caption, setCaption] = useState<Caption>({ committed: "", tentative: "" });

  useEffect(() => {
    document.documentElement.classList.add("indicator-window");
    void api.dictationState().then(setState);
    const unlisten = [
      events.onDictation((s) => {
        setState(s);
        // A new dictation starts with a clean slate.
        if (s.phase === "idle") setCaption({ committed: "", tentative: "" });
      }),
      events.onLevel(setLevel),
      events.onCaption(setCaption),
    ];
    return () => unlisten.forEach((u) => void u.then((f) => f()));
  }, []);

  // Peak level → 0..1 on a log scale: -48 dBFS (room noise) to -6 dBFS (loud).
  const db = 20 * Math.log10(Math.max(level, 1e-6));
  const loudness = Math.min(1, Math.max(0, (db + 48) / 42));

  return (
    <div className="indicator-stage">
      {state.phase !== "idle" && (caption.committed || caption.tentative) && <Captions caption={caption} />}
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

/** A word on screen: its place in the whole dictation keeps it stable. */
interface Word {
  key: string;
  text: string;
  locked: boolean;
}

/**
 * What's been said, above the pill: a fixed-width box (so words never shift
 * sideways) showing the last two lines. New words fade up into place a beat
 * apart, words turn from dim to white as they lock in, and when a line is
 * added the older ones glide up rather than jump.
 */
function Captions({ caption }: { caption: Caption }) {
  const words = useMemo(() => {
    const split = (text: string) => text.split(/\s+/).filter(Boolean);
    const all: Word[] = [
      ...split(caption.committed).map((text) => ({ text, locked: true })),
      ...split(caption.tentative).map((text) => ({ text, locked: false })),
    ].map((w, i) => ({ ...w, key: `${i}:${w.text}` }));
    return all.slice(-CAPTION_WORDS);
  }, [caption]);

  const text = useRef<HTMLSpanElement>(null);
  const [height, setHeight] = useState(LINE_PX);
  const lastHeight = useRef(0);
  // Each word's fade-in delay, fixed when it first appears: new words arrive
  // a beat apart. (The pill's meter re-renders this 20 times a second, so a
  // delay worked out per render would be lost mid-animation.)
  const delays = useRef(new Map<string, number>());
  let arriving = 0;
  for (const w of words) {
    if (!delays.current.has(w.key)) delays.current.set(w.key, Math.min(arriving++ * 45, 360));
  }

  useLayoutEffect(() => {
    const shown = new Set(words.map((w) => w.key));
    for (const key of delays.current.keys()) if (!shown.has(key)) delays.current.delete(key);
    const el = text.current;
    if (!el) return;
    const h = el.offsetHeight;
    setHeight(Math.min(h, MAX_LINES * LINE_PX));
    // The text sits at the bottom, so a new line pushes the rest up at once:
    // start it back down where it was and let it glide.
    const grew = h - lastHeight.current;
    if (lastHeight.current > 0 && grew > 0 && !reducedMotion()) {
      el.animate([{ transform: `translateY(${grew}px)` }, { transform: "translateY(0)" }], {
        duration: 320,
        easing: "cubic-bezier(0.2, 0.8, 0.2, 1)",
      });
    }
    lastHeight.current = h;
  }, [words]);

  return (
    // Not announced word by word: the pill already says it's listening.
    <div className="caption" aria-hidden="true">
      <div className="caption-lines" style={{ height }}>
        <span className="caption-text" ref={text}>
          {words.map((w) => {
            // The space between words is where lines can wrap.
            return (
              <Fragment key={w.key}>
                <span
                  className={`caption-word${w.locked ? " locked" : ""}`}
                  style={{ animationDelay: `${delays.current.get(w.key) ?? 0}ms` }}
                >
                  {w.text}
                </span>{" "}
              </Fragment>
            );
          })}
        </span>
      </div>
    </div>
  );
}

function reducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}
