// The browser preview's stand-in for a meeting's write-up after recording
// (app/src-tauri/src/pipeline.rs): steps that move along over about 13 s,
// lines heard from the sample transcript, time left, and what came out.
// Started by stopping a recording, or by ?writeup (?writeup=failed fails at
// the notes, as with no OpenRouter key).
import { emit } from "@tauri-apps/api/event";
import type { Outcome, Progress, StepKind, WriteUp, WriteUpStep } from "../api";

interface Plan {
  kind: StepKind;
  /** How long the step takes, ms. */
  ms: number;
  /** Lines it "hears", from the sample transcript. */
  lines?: string[];
}

export function simulateWriteUp({
  id,
  title,
  lines,
  fail,
  onTranscribed,
  onDone,
}: {
  id: string;
  title: string;
  /** The sample transcript: who and what. */
  lines: { you: boolean; text: string }[];
  fail: boolean;
  /** The transcript exists now (the meeting shows it). */
  onTranscribed: () => void;
  onDone: (ok: boolean) => void;
}) {
  const plan: Plan[] = [
    { kind: "prepare", ms: 1200 },
    { kind: "you", ms: 3400, lines: lines.filter((l) => l.you).map((l) => l.text) },
    { kind: "call", ms: 3800, lines: lines.filter((l) => !l.you).map((l) => l.text) },
    { kind: "speakers", ms: 1900 },
    { kind: "notes", ms: 2600 },
  ];
  const total = plan.reduce((n, p) => n + p.ms, 0);
  const started = Date.now();
  const heard: { you: boolean; text: string }[] = [];
  let said = new Map<StepKind, number>();

  const report = (elapsed: number, failedAt?: StepKind, outcome?: Outcome): WriteUp => {
    let at = 0;
    const steps: WriteUpStep[] = plan.map((p) => {
      const begin = at;
      at += p.ms;
      if (failedAt) {
        if (p.kind === failedAt) return { kind: p.kind, status: "failed", fraction: null, ms: elapsed - begin };
        return begin < elapsed
          ? { kind: p.kind, status: "done", fraction: 1, ms: p.ms }
          : { kind: p.kind, status: "pending", fraction: null, ms: null };
      }
      if (outcome || elapsed >= at) return { kind: p.kind, status: "done", fraction: 1, ms: p.ms };
      if (elapsed < begin) return { kind: p.kind, status: "pending", fraction: null, ms: null };
      const into = elapsed - begin;
      // Transcription knows how far it is; the rest doesn't.
      const measured = p.kind === "you" || p.kind === "call";
      return { kind: p.kind, status: "active", fraction: measured ? into / p.ms : null, ms: into };
    });
    // Heard lines arrive through the transcription steps.
    for (const p of plan) {
      const step = steps.find((s) => s.kind === p.kind)!;
      if (!p.lines || step.status !== "active" || step.fraction == null) continue;
      const due = Math.floor(step.fraction * p.lines.length);
      for (let i = said.get(p.kind) ?? 0; i < due; i++) {
        heard.push({ you: p.kind === "you", text: p.lines[i] });
        if (heard.length > 3) heard.shift();
      }
      said.set(p.kind, due);
    }
    return {
      steps,
      elapsed_ms: elapsed,
      eta_ms: outcome || failedAt || elapsed < 1500 ? null : Math.max(0, total - elapsed) * 1.1,
      heard: outcome ? [] : [...heard],
      outcome: outcome ?? null,
    };
  };

  const stageOf = (r: WriteUp): Progress["stage"] => {
    const active = r.steps.find((s) => s.status === "active")?.kind;
    return active === "speakers" ? "identifying" : active === "notes" ? "summarizing" : "transcribing";
  };
  const send = (p: Partial<Progress> & { report: WriteUp }) =>
    void emit("meeting-progress", {
      meeting_id: id,
      stage: stageOf(p.report),
      fraction: p.report.steps.find((s) => s.status === "active")?.fraction ?? null,
      message: null,
      title: null,
      ...p,
    } satisfies Progress);

  let transcribed = false;
  const tick = setInterval(() => {
    const elapsed = Date.now() - started;
    const notesAt = total - plan[plan.length - 1].ms;
    if (!transcribed && elapsed >= notesAt - plan[3].ms) {
      transcribed = true;
      onTranscribed();
    }
    if (fail && elapsed >= notesAt + 1400) {
      clearInterval(tick);
      onDone(false);
      send({
        stage: "failed",
        message: "No OpenRouter key yet. Add one in Settings → OpenRouter, or choose a local Ollama model in Settings → Models.",
        title,
        report: report(elapsed, "notes"),
      });
      return;
    }
    if (elapsed >= total) {
      clearInterval(tick);
      // Saved before it's announced, as the app does.
      onDone(true);
      const words = lines.reduce((n, l) => n + l.text.split(/\s+/).length, 0);
      send({
        stage: "done",
        title,
        report: report(elapsed, undefined, {
          words: words * 18,
          people: 4,
          action_items: 3,
          decisions: 3,
          notes: true,
          took_ms: elapsed,
        }),
      });
      return;
    }
    send({ report: report(elapsed) });
  }, 150);
  said = new Map();
}
