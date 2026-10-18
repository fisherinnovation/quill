// The waveform behind the Dictations page's hero: bars shaped by the last
// 30 days of dictation, mirrored like a voice's waveform. Idle, it
// breathes; while listening it follows the voice (the mic level the pill
// gets); while typing, a shimmer runs across it. Canvas, so dozens of bars
// redraw without touching the DOM.
import { useEffect, useRef } from "react";
import { events } from "../api";

export type WavePhase = "idle" | "listening" | "processing" | "off";

const BAR = 3;
const GAP = 3;
// The palette: fuchsia, grape, sky (the same in both themes; the canvas's
// opacity comes from CSS).
// The fuchsia starts where the text's fade ends, so it shows.
const STOPS: [number, string][] = [
  [0, "#ff1053"],
  [0.35, "#ff1053"],
  [0.68, "#6c6ea0"],
  [1, "#66c7f4"],
];

/** Peak level → 0..1: -48 dBFS (room noise) to -6 dBFS (loud), as the pill. */
function loudness(level: number): number {
  const db = 20 * Math.log10(Math.max(level, 1e-6));
  return Math.min(1, Math.max(0, (db + 48) / 42));
}

/**
 * Each bar's resting height, 0..1: the days resampled across the bars and
 * smoothed, with a little variation between neighbours so it reads as a
 * waveform rather than steps, and tapering at both ends. With no data, a
 * gentle made-up wave.
 */
function shape(days: number[], n: number): number[] {
  const max = Math.max(0, ...days);
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    const x = n > 1 ? i / (n - 1) : 0;
    let v: number;
    if (max > 0 && days.length > 1) {
      const at = x * (days.length - 1);
      const lo = Math.floor(at);
      const hi = Math.min(days.length - 1, lo + 1);
      v = (days[lo] + (days[hi] - days[lo]) * (at - lo)) / max;
    } else {
      v = 0.45 + 0.3 * Math.sin(x * Math.PI * 3.2) * Math.cos(x * Math.PI * 1.3);
    }
    const texture = 0.82 + 0.18 * Math.sin(i * 1.7) * Math.cos(i * 0.37);
    const taper = Math.pow(Math.sin(Math.PI * (i + 0.5) / n), 0.6);
    out.push((0.16 + 0.84 * v) * texture * taper);
  }
  // A light blur, so single days don't spike.
  return out.map((v, i) => (2 * v + (out[i - 1] ?? v) + (out[i + 1] ?? v)) / 4);
}

export function VoiceWave({ days, phase }: { days: number[]; phase: WavePhase }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const phaseRef = useRef(phase);
  const daysRef = useRef(days);
  const target = useRef(0);
  // Redraws a still wave (reduced motion) when something changes.
  const redraw = useRef<() => void>(() => {});
  phaseRef.current = phase;

  useEffect(() => {
    daysRef.current = days;
    redraw.current();
  }, [days]);

  useEffect(() => {
    redraw.current();
  }, [phase]);

  useEffect(() => {
    const unlisten = events.onLevel((level) => {
      target.current = loudness(level);
      redraw.current();
    });
    return () => void unlisten.then((f) => f());
  }, []);

  useEffect(() => {
    const el = canvas.current;
    const ctx = el?.getContext("2d");
    if (!el || !ctx) return;
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    let width = 0;
    let height = 0;
    let base: number[] = [];
    let shapedFrom: number[] | null = null;
    // Eased amounts of each state, so changes blend rather than jump.
    let live = 0;
    let sweep = 0;
    let off = phaseRef.current === "off" ? 1 : 0;
    let level = 0;
    let frame = 0;

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      width = el.clientWidth;
      height = el.clientHeight;
      el.width = Math.round(width * dpr);
      el.height = Math.round(height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      shapedFrom = null;
    };

    const draw = (t: number) => {
      if (width === 0 || height === 0) return;
      const n = Math.max(8, Math.floor(width / (BAR + GAP)));
      if (shapedFrom !== daysRef.current || base.length !== n) {
        base = shape(daysRef.current, n);
        shapedFrom = daysRef.current;
      }
      const p = phaseRef.current;
      const ease = still ? 1 : 0.08;
      live += ((p === "listening" ? 1 : 0) - live) * ease;
      sweep += ((p === "processing" ? 1 : 0) - sweep) * ease;
      off += ((p === "off" ? 1 : 0) - off) * ease;
      level += (target.current - level) * (still ? 1 : 0.25);
      if (p !== "listening") target.current *= 0.9;

      ctx.clearRect(0, 0, width, height);
      const gradient = ctx.createLinearGradient(0, 0, width, 0);
      for (const [at, colour] of STOPS) gradient.addColorStop(at, colour);
      ctx.fillStyle = gradient;

      const mid = height / 2;
      const tall = height * 0.42;
      const offset = (width - n * (BAR + GAP) + GAP) / 2;
      // Where the typing shimmer is: right to left and round again.
      const pos = (1.2 - ((t * 0.55) % 1.6)) * n;
      for (let i = 0; i < n; i++) {
        const b = base[i];
        const breathe = still ? 1 : 0.8 + 0.14 * Math.sin(t * 1.3 + i * 0.42) + 0.06 * Math.sin(t * 0.5 - i * 0.11);
        const centre = Math.sin(Math.PI * (i + 0.5) / n);
        const voice = still
          ? level * centre
          : level * centre * (0.55 + 0.45 * Math.sin(t * 9 + i * 0.83) * Math.sin(t * 3.1 + i * 0.29));
        const shimmer = still ? 0 : Math.exp(-(((i - pos) / (n * 0.07)) ** 2));
        let h = b * breathe * (1 - 0.55 * live) + live * (0.12 * b + 0.9 * voice);
        h *= 1 + 0.45 * sweep * shimmer;
        h *= 1 - 0.35 * off;
        const bar = Math.max(1.5, h * tall);
        ctx.globalAlpha = (0.55 + 0.45 * Math.max(live, sweep * shimmer)) * (1 - 0.5 * off);
        const x = offset + i * (BAR + GAP);
        ctx.beginPath();
        ctx.roundRect(x, mid - bar, BAR, bar * 2, BAR / 2);
        ctx.fill();
      }
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
    };
    // Not animating in a hidden window.
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

  return <canvas ref={canvas} className="voice-wave" aria-hidden />;
}
