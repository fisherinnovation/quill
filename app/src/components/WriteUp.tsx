// A meeting's write-up after the recording stops (pipeline.rs): the steps
// as they happen, time left, the lines being heard, then what came out.
import { useEffect, useRef, useState } from "react";
import type { Outcome, Progress, StepKind, WriteUpStep } from "../api";
import { Icon } from "./Icon";
import { VoiceWave } from "./VoiceWave";

const STEP_LABEL: Record<StepKind, string> = {
  prepare: "Getting the audio ready",
  you: "Transcribing you",
  call: "Transcribing the call",
  speakers: "Telling the speakers apart",
  notes: "Writing the notes",
};

/** The step under way, as a few words for lists and tiles ("Transcribing · 42%"). */
export function writeUpStatus(progress: Progress): string {
  if (progress.stage === "downloading") return "Downloading";
  const active = progress.report?.steps.find((s) => s.status === "active");
  const pct = active?.fraction != null ? ` · ${Math.round(active.fraction * 100)}%` : "";
  switch (active?.kind) {
    case "speakers":
      return "Finding speakers";
    case "notes":
      return "Writing notes";
    case "you":
    case "call":
      return `Transcribing${pct}`;
    default:
      return "Getting ready";
  }
}

/** How far the whole write-up is, 0–1, for a thin bar: steps done, plus the active one's share. */
export function writeUpFraction(progress: Progress): number | null {
  const steps = progress.report?.steps.filter((s) => s.status !== "skipped");
  if (!steps || steps.length === 0) return null;
  const done = steps.filter((s) => s.status === "done").length;
  const active = steps.find((s) => s.status === "active");
  return Math.min(1, (done + (active?.fraction ?? (active ? 0.3 : 0))) / steps.length);
}

/** "0:42", "12:05". */
function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** "A few seconds left", "About 25 s left", "About 4 min left". */
function timeLeft(ms: number): string {
  if (ms < 8_000) return "A few seconds left";
  if (ms < 60_000) return `About ${Math.ceil(ms / 5_000) * 5} s left`;
  return `About ${Math.round(ms / 60_000)} min left`;
}

/** "48 s", "2 min 5 s". */
function took(ms: number): string {
  const s = Math.max(1, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  return s % 60 ? `${m} min ${s % 60} s` : `${m} min`;
}

/** Re-renders every `ms`, so local clocks move between events. */
function useTick(ms: number, on: boolean) {
  const [, setNow] = useState(0);
  useEffect(() => {
    if (!on) return;
    const t = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(t);
  }, [ms, on]);
}

export function WriteUpCard({
  progress,
  durationMs,
  compact,
  title,
  onRetry,
  onDismiss,
  onOpen,
}: {
  progress: Progress;
  /** The recording's length, for the outcome (0: not shown). */
  durationMs: number;
  /** The small card on the live notes panel, after the recording. */
  compact?: boolean;
  /** The meeting's title, shown on the compact card. */
  title?: string;
  onRetry: () => void;
  onDismiss: () => void;
  /** Opens the meeting in Quill (the compact card). */
  onOpen?: () => void;
}) {
  const report = progress.report;
  const working = progress.stage !== "done" && progress.stage !== "failed";
  // When this event arrived: the clocks run on from it until the next.
  const received = useRef({ progress, at: Date.now() });
  if (received.current.progress !== progress) received.current = { progress, at: Date.now() };
  useTick(500, working);
  const since = Date.now() - received.current.at;

  if (progress.stage === "done" && report?.outcome) {
    return (
      <Ready
        outcome={report.outcome}
        durationMs={durationMs}
        title={compact ? title : undefined}
        onDismiss={onDismiss}
        onOpen={onOpen}
      />
    );
  }

  const failed = progress.stage === "failed";
  const elapsed = (report?.elapsed_ms ?? 0) + (working ? since : 0);
  const eta = report?.eta_ms != null ? Math.max(0, report.eta_ms - since) : null;

  return (
    <section className={`writeup ${failed ? "failed" : "working"}${compact ? " compact" : ""}`} aria-live="polite">
      {!failed && <VoiceWave days={NO_DAYS} phase="processing" />}
      <header className="writeup-head">
        <span className="writeup-kicker">
          {failed ? <Icon name="alert" size={13} /> : <span className="writeup-dot" />}
          {failed ? "Couldn't finish" : "Writing up"}
        </span>
        <h2>{compact && title ? title : failed ? "The write-up stopped" : "Turning the call into notes"}</h2>
        <p className="writeup-time">
          {clock(elapsed)}
          {!failed && ` · ${eta != null ? timeLeft(eta) : "Working out how long"}`}
        </p>
      </header>

      {report && (
        <ol className="writeup-steps">
          {report.steps.map((step) => (
            <StepRow key={step.kind} step={step} since={step.status === "active" ? since : 0} progress={progress} />
          ))}
        </ol>
      )}

      {failed ? (
        <div className="writeup-error">
          <p>{progress.message}</p>
          <button className="primary" onClick={onRetry}>
            {compact ? (
              "Open in Quill"
            ) : (
              <>
                <Icon name="refresh" size={13} /> Try again
              </>
            )}
          </button>
          {compact && (
            <button className="ghost icon-btn" aria-label="Close" title="Close" onClick={onDismiss}>
              <Icon name="x" size={13} />
            </button>
          )}
        </div>
      ) : compact ? (
        <div className="writeup-compact-foot">
          {report && report.heard.length > 0 && (
            <p className="writeup-compact-heard">
              <span className={`writeup-who${report.heard[report.heard.length - 1].you ? " you" : ""}`}>
                {report.heard[report.heard.length - 1].you ? "You" : "Call"}
              </span>
              {report.heard[report.heard.length - 1].text}
            </p>
          )}
          {onOpen && (
            <button className="link-button" onClick={onOpen}>
              Open in Quill
            </button>
          )}
        </div>
      ) : (
        report &&
        report.heard.length > 0 && (
          <div className="writeup-heard" aria-label="Just heard">
            {report.heard.map((h, i) => (
              <p key={`${h.text}-${i}`} style={{ opacity: 0.45 + (0.55 * (i + 1)) / report.heard.length }}>
                <span className={`writeup-who${h.you ? " you" : ""}`}>{h.you ? "You" : "Call"}</span>
                {h.text}
              </p>
            ))}
          </div>
        )
      )}
    </section>
  );
}

const NO_DAYS: number[] = [];

function StepRow({ step, since, progress }: { step: WriteUpStep; since: number; progress: Progress }) {
  const downloading = step.status === "active" && progress.stage === "downloading";
  const fraction = downloading ? progress.fraction : step.fraction;
  const right =
    step.status === "done"
      ? took(step.ms ?? 0)
      : step.status === "skipped"
        ? "Not needed"
        : step.status === "active" && fraction != null
          ? `${Math.round(fraction * 100)}%`
          : step.status === "active"
            ? clock((step.ms ?? 0) + since)
            : "";
  return (
    <li className={`writeup-step ${step.status}`}>
      <span className="writeup-icon" aria-hidden>
        {step.status === "done" ? (
          <Icon name="check" size={12} />
        ) : step.status === "failed" ? (
          <Icon name="x" size={12} />
        ) : step.status === "active" ? (
          <span className="spinner" />
        ) : null}
      </span>
      <span className="writeup-label">
        {STEP_LABEL[step.kind]}
        {downloading && progress.message && <span className="writeup-sub">{progress.message}</span>}
      </span>
      <span className="writeup-right">{right}</span>
      {step.status === "active" && (
        <span className={`writeup-bar${fraction == null ? " unknown" : ""}`}>
          <span style={fraction != null ? { width: `${fraction * 100}%` } : undefined} />
        </span>
      )}
    </li>
  );
}

function Ready({
  outcome,
  durationMs,
  title,
  onDismiss,
  onOpen,
}: {
  outcome: Outcome;
  durationMs: number;
  /** On the compact card: which meeting. */
  title?: string;
  onDismiss: () => void;
  onOpen?: () => void;
}) {
  const facts: string[] = [];
  if (durationMs > 0) facts.push(`${Math.max(1, Math.round(durationMs / 60_000))} min`);
  if (outcome.people > 0) facts.push(`${outcome.people} ${outcome.people === 1 ? "person" : "people"}`);
  if (outcome.notes) {
    facts.push(`${outcome.action_items} action ${outcome.action_items === 1 ? "item" : "items"}`);
    facts.push(`${outcome.decisions} ${outcome.decisions === 1 ? "decision" : "decisions"}`);
  }
  if (outcome.words > 0) facts.push(`${outcome.words.toLocaleString()} words`);
  return (
    <section className={`writeup ready${onOpen ? " compact" : ""}`} aria-live="polite">
      <span className="writeup-check" aria-hidden>
        <Icon name="check" size={18} />
      </span>
      <div className="writeup-ready-body">
        <h2>{outcome.notes ? "Your notes are ready" : "Your transcript is ready"}</h2>
        <p className="writeup-time">
          {title ? `${title} · ` : ""}Written up in {took(outcome.took_ms)}
        </p>
        <div className="writeup-facts">
          {facts.map((f, i) => (
            <span key={f} style={{ animationDelay: `${120 + i * 70}ms` }}>
              {f}
            </span>
          ))}
        </div>
        {onOpen && (
          <button className="primary writeup-open" onClick={onOpen}>
            Open notes <Icon name="chevron-right" size={13} />
          </button>
        )}
      </div>
      <button className="ghost icon-btn" aria-label="Dismiss" title="Dismiss" onClick={onDismiss}>
        <Icon name="x" size={14} />
      </button>
    </section>
  );
}
