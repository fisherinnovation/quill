// The weave behind the Meetings page's hero: each recent meeting's back and
// forth as a ribbon (the same shape the tiles show: who held the floor,
// slice by slice), coloured by speaker, drifting slowly sideways, the rows
// in alternate directions. Faster and brighter while recording. Canvas,
// like VoiceWave: stops drawing while the window is hidden and holds still
// with reduced motion.
import { useEffect, useRef } from "react";
import type { MeetingOverview } from "../api";

export type WeavePhase = "idle" | "recording" | "writing";

/** Rows drawn at most. */
const ROWS = 7;

/** A meeting's turns as runs: who, from and to (slices), how much was said. */
interface Run {
  slot: number;
  start: number;
  end: number;
  level: number;
}

function runs(o: Pick<MeetingOverview, "speaker" | "level">): Run[] {
  const out: Run[] = [];
  o.speaker.forEach((slot, i) => {
    const last = out[out.length - 1];
    if (slot === 0) return;
    if (last && last.slot === slot && last.end === i) {
      last.end = i + 1;
      last.level = Math.max(last.level, o.level[i]);
    } else {
      out.push({ slot, start: i, end: i + 1, level: o.level[i] });
    }
  });
  return out;
}

/** Made-up conversations for before there are any (seeded, so it's steady). */
function sample(rows: number): Run[][] {
  let seed = 7;
  const random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
  return Array.from({ length: rows }, () => {
    const out: Run[] = [];
    let at = Math.floor(random() * 4);
    while (at < 96) {
      const length = 2 + Math.floor(random() * 9);
      out.push({ slot: 1 + Math.floor(random() * 3), start: at, end: Math.min(96, at + length), level: 40 + random() * 60 });
      at += length + Math.floor(random() * 3);
    }
    return out;
  });
}

export function ConversationWeave({
  overviews,
  phase,
}: {
  /** Recent meetings' overviews, newest first. */
  overviews: MeetingOverview[];
  phase: WeavePhase;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const rowsRef = useRef<Run[][]>([]);
  const phaseRef = useRef(phase);
  const redraw = useRef<() => void>(() => {});
  phaseRef.current = phase;

  useEffect(() => {
    const real = overviews.filter((o) => o.speaker.length > 0).slice(0, ROWS);
    rowsRef.current = real.length >= 3 ? real.map(runs) : sample(ROWS);
    redraw.current();
  }, [overviews]);

  useEffect(() => {
    redraw.current();
  }, [phase]);

  useEffect(() => {
    const el = canvas.current;
    const ctx = el?.getContext("2d");
    if (!el || !ctx) return;
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let width = 0;
    let height = 0;
    let frame = 0;
    let colours: string[] = [];
    let readAt = -1;
    // Eased speed and brightness, so phase changes glide.
    let speed = 1;
    let glow = 0;
    // Where each row has drifted to, in slices.
    let offset = 0;
    let last = 0;

    // The speaker colours, read from the theme (they change with it).
    const readColours = () => {
      const css = getComputedStyle(document.documentElement);
      colours = [css.getPropertyValue("--me"), ...Array.from({ length: 8 }, (_, i) => css.getPropertyValue(`--spk-${i + 1}`))].map(
        (c) => c.trim() || "#888",
      );
    };
    const colour = (slot: number) => (slot === 1 ? colours[0] : colours[1 + ((slot - 2) % 8)]);

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      width = el.clientWidth;
      height = el.clientHeight;
      el.width = Math.round(width * dpr);
      el.height = Math.round(height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };

    const draw = (t: number) => {
      if (width === 0 || height === 0) return;
      if (readAt < 0 || t - readAt > 2) {
        readColours();
        readAt = t;
      }
      const p = phaseRef.current;
      const ease = still ? 1 : 0.04;
      speed += ((p === "recording" ? 3.2 : p === "writing" ? 1.8 : 1) - speed) * ease;
      glow += ((p === "idle" ? 0 : 1) - glow) * ease;
      const dt = last ? Math.min(0.1, t - last) : 0;
      last = t;
      offset += still ? 0 : dt * speed * 1.6;

      ctx.clearRect(0, 0, width, height);
      const rows = rowsRef.current;
      if (rows.length === 0) return;
      const slice = width / 70; // a little over the width shows at once
      const gap = height / (rows.length + 1);
      rows.forEach((row, r) => {
        const direction = r % 2 === 0 ? 1 : -1;
        const shift = (((offset * direction + r * 13) % 96) + 96) % 96;
        const y = gap * (r + 1) + (still ? 0 : Math.sin(t * 0.35 + r * 1.3) * 2.5);
        // Each ribbon twice over, so it wraps without a seam.
        for (const lap of [-96, 0, 96]) {
          for (const run of row) {
            const x = (run.start - shift + lap) * slice;
            const w = (run.end - run.start) * slice - 1.5;
            if (x + w < 0 || x > width) continue;
            const thick = (run.slot === 1 ? 5 : 4) + (run.level / 100) * 3 + glow * 1.5;
            ctx.globalAlpha = (0.35 + (run.level / 100) * 0.5) * (0.75 + glow * 0.25);
            ctx.fillStyle = colour(run.slot);
            ctx.beginPath();
            ctx.roundRect(x, y - thick / 2, Math.max(1.5, w), thick, thick / 2);
            ctx.fill();
          }
        }
      });
      ctx.globalAlpha = 1;
    };

    const loop = (now: number) => {
      draw(now / 1000);
      frame = requestAnimationFrame(loop);
    };
    const start = () => {
      if (still) draw(0);
      else if (!frame && !document.hidden) frame = requestAnimationFrame(loop);
    };
    const stop = () => {
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      last = 0;
    };
    const onVisibility = () => (document.hidden ? stop() : start());
    redraw.current = () => {
      if (still) draw(0);
    };
    const observer = new ResizeObserver(() => {
      resize();
      if (still) draw(0);
    });
    observer.observe(el);
    resize();
    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      observer.disconnect();
      document.removeEventListener("visibilitychange", onVisibility);
      redraw.current = () => {};
    };
  }, []);

  return <canvas ref={canvas} className="conversation-weave" aria-hidden />;
}
