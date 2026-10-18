import { useLayoutEffect, useRef, useState } from "react";
import { FEATHER } from "./Icon";

/** The heartbeat from the mark, scaled to px: one beat, then flat. */
const BEAT = 0.06;
/** Where the line sits in its 22 px strip, and where the beat starts. */
const LINE_Y = 12;
const BEAT_X = 6;

// The line draws itself once per launch, the first time Home shows.
let drawn = false;

/**
 * Quill's name for the top of Home: the quill on its own (not the app
 * icon's tile) beside "Quill" in a serif, with the mark's heartbeat run
 * out under the word like a signature line, from the quill's tip.
 */
export function Wordmark() {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [animate] = useState(() => {
    const first = !drawn && !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    drawn = true;
    return first;
  });

  useLayoutEffect(() => {
    setWidth(box.current?.offsetWidth ?? 0);
  }, []);

  const s = (n: number) => +(n * BEAT).toFixed(2);
  const line = `M0 ${LINE_Y}h${BEAT_X}l${s(30)} ${s(-50)} ${s(40)} ${s(100)} ${s(40)} ${s(-150)} ${s(40)} ${s(120)} ${s(30)} ${s(-20)}H${width}`;

  return (
    <div className={`wordmark${animate ? " drawing" : ""}`} ref={box} role="img" aria-label="Quill">
      <svg className="wordmark-quill" viewBox="300 220 480 560" aria-hidden>
        <path d={FEATHER} fill="currentColor" />
      </svg>
      <span className="wordmark-name" aria-hidden>
        Quill
      </span>
      {width > 0 && (
        <svg className="wordmark-line" width={width} height="22" aria-hidden>
          <path d={line} pathLength={1} />
        </svg>
      )}
    </div>
  );
}
