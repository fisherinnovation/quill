// Reviewing saved dictation samples (Settings → Dictation → Testing
// accuracy): listen to each, fix what was typed into what was said, and the
// fix becomes its reference (quill_core::samples). The page scores what was
// typed, and Whisper's own text, against the references as you go;
// `quill-cli dictate` uses the same references for full reruns.
import { useEffect, useRef, useState } from "react";
import { api, errorText, isMac, type Sample, type SampleReview, type WordTally } from "../api";
import { Icon } from "./Icon";
import { BackLink, Banner } from "./ui";

const MOD = isMac ? "⌘" : "Ctrl+";

/** Share of reference words right, 0–100 (errors can outnumber words; floored at 0). */
function rightShare(t: WordTally): number {
  if (t.reference_words === 0) return 100;
  const errors = t.substitutions + t.deletions + t.insertions;
  return Math.max(0, 100 - (errors / t.reference_words) * 100);
}

function when(name: string): string {
  // Names are local times: 2026-10-16_143012 (and -2 for a second one).
  const m = /^(\d{4})-(\d{2})-(\d{2})_(\d{2})(\d{2})(\d{2})/.exec(name);
  if (!m) return name;
  const d = new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
  return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

function seconds(ms: number): string {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function SamplesReview({ onBack }: { onBack: () => void }) {
  const [review, setReview] = useState<SampleReview | null>(null);
  const [index, setIndex] = useState(0);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [played, setPlayed] = useState(0);
  const audio = useRef<HTMLAudioElement | null>(null);
  const editor = useRef<HTMLTextAreaElement>(null);

  async function load(): Promise<SampleReview | null> {
    try {
      const r = await api.sampleReview();
      setReview(r);
      return r;
    } catch (e) {
      setError(errorText(e));
      return null;
    }
  }

  // Start at the top, at the first sample still to review.
  useEffect(() => {
    document.querySelector(".main-scroll")?.scrollTo(0, 0);
    void load().then((r) => {
      if (!r) return;
      const first = r.samples.findIndex((s) => s.reference === null);
      setIndex(first === -1 ? 0 : first);
    });
  }, []);

  const sample: Sample | undefined = review?.samples[index];

  // Each sample: its text in the editor, and its audio, played straight away.
  useEffect(() => {
    if (!sample) return;
    setText(sample.reference ?? sample.typed);
    setConfirmDelete(false);
    let url: string | null = null;
    let cancelled = false;
    setPlayed(0);
    void api
      .sampleAudio(sample.name)
      .then((bytes) => {
        if (cancelled) return;
        const head = new Uint8Array(bytes.slice(0, 4));
        const type = String.fromCharCode(...head) === "RIFF" ? "audio/wav" : "audio/flac";
        url = URL.createObjectURL(new Blob([bytes], { type }));
        const el = new Audio(url);
        el.onplay = () => setPlaying(true);
        el.onpause = () => setPlaying(false);
        el.onended = () => setPlaying(false);
        el.ontimeupdate = () => setPlayed(el.duration ? el.currentTime / el.duration : 0);
        audio.current?.pause();
        audio.current = el;
        void el.play().catch(() => {});
      })
      .catch(() => {});
    editor.current?.focus();
    return () => {
      cancelled = true;
      audio.current?.pause();
      audio.current = null;
      if (url) URL.revokeObjectURL(url);
    };
    // A new sample, not a new list.
  }, [sample?.name]);

  function play() {
    const el = audio.current;
    if (!el) return;
    if (!el.paused) el.pause();
    else {
      if (el.ended) el.currentTime = 0;
      void el.play().catch(() => {});
    }
  }

  /** The next sample still to review after this one, else simply the next. */
  function nextIndex(r: SampleReview, from: number): number {
    const later = r.samples.findIndex((s, i) => i > from && s.reference === null);
    return later !== -1 ? later : Math.min(from + 1, r.samples.length - 1);
  }

  async function save() {
    if (!sample || saving) return;
    setSaving(true);
    try {
      await api.setSampleReference(sample.name, text);
      const r = await load();
      if (r) setIndex(nextIndex(r, index));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!sample) return;
    try {
      await api.deleteSample(sample.name);
      const r = await load();
      if (r) setIndex(Math.min(index, Math.max(0, r.samples.length - 1)));
    } catch (e) {
      setError(errorText(e));
    }
  }

  // ⌘↵ saves and moves on; ⌘P plays again; ⌘↑ / ⌘↓ go back and forward.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = isMac ? e.metaKey : e.ctrlKey;
      if (!mod || !review) return;
      if (e.key === "Enter") {
        e.preventDefault();
        void save();
      } else if (e.key.toLowerCase() === "p") {
        e.preventDefault();
        play();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setIndex((i) => Math.min(i + 1, review.samples.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setIndex((i) => Math.max(i - 1, 0));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (!review) {
    return (
      <div className="page samples-review">
        <BackLink label="Settings" onClick={onBack} />
        {error && (
          <Banner kind="error" icon="alert">
            {error}
          </Banner>
        )}
      </div>
    );
  }

  const total = review.samples.length;
  const acc = review.accuracy;
  const done = acc.reviewed;
  const changed = sample ? text.trim() !== (sample.reference ?? sample.typed).trim() : false;

  return (
    <div className="page samples-review">
      <BackLink label="Settings" onClick={onBack} />
      <header className="page-head">
        <div>
          <h1 className="page-title">Review dictation samples</h1>
          <p className="page-subtitle">
            Listen, then fix what was typed into what you said. Each fix becomes a reference for measuring accuracy on
            your own voice.
          </p>
        </div>
      </header>
      {error && (
        <Banner kind="error" icon="alert">
          {error}
        </Banner>
      )}

      {total === 0 ? (
        <div className="card home-empty">
          <div className="home-card-icon">
            <Icon name="mic" size={17} />
          </div>
          <div>
            <strong>No samples yet</strong>
            <p className="muted small">Turn on Save dictations for testing, then dictate as usual.</p>
          </div>
        </div>
      ) : (
        <>
          <div className="review-progress">
            <span>
              <strong>{done}</strong> of {total} reviewed
            </span>
            <span className="review-bar">
              <span style={{ width: `${(done / total) * 100}%` }} />
            </span>
          </div>

          {done > 0 && <AccuracyCard accuracy={acc} />}

          {sample && (
            <section className={`review-card${sample.reference !== null ? " reviewed" : ""}`}>
              <header className="review-head">
                <button className={`review-play${playing ? " playing" : ""}`} onClick={play} aria-label="Play">
                  <svg className="review-ring-svg" viewBox="0 0 36 36" aria-hidden>
                    <circle cx="18" cy="18" r="16" className="review-ring" />
                    <circle
                      cx="18"
                      cy="18"
                      r="16"
                      className="review-ring-done"
                      style={{ strokeDashoffset: `${100 - played * 100}`, opacity: played > 0 ? 1 : 0 }}
                      pathLength={100}
                    />
                  </svg>
                  <Icon name={playing ? "pause" : "play"} size={16} />
                </button>
                <div className="review-meta">
                  <strong>
                    {index + 1} of {total}
                  </strong>
                  <span>
                    {when(sample.name)}
                    {sample.app && <span className="dot-sep">{sample.app}</span>}
                    <span className="dot-sep">{seconds(sample.duration_ms)}</span>
                    {sample.polish_model && <span className="dot-sep">tidied up</span>}
                  </span>
                </div>
                {sample.reference !== null && (
                  <span className={`review-state${sample.errors.length ? " errors" : ""}`}>
                    <Icon name="check" size={12} />
                    {sample.errors.length === 0
                      ? "Right as typed"
                      : `${sample.errors.length} ${sample.errors.length === 1 ? "mistake" : "mistakes"}`}
                  </span>
                )}
              </header>

              {sample.polish_model && sample.whisper && sample.whisper !== sample.typed && (
                <p className="review-whisper">
                  <span>Whisper heard</span>
                  {sample.whisper}
                </p>
              )}

              <label className="review-label" htmlFor="review-text">
                What you said
              </label>
              <textarea
                id="review-text"
                ref={editor}
                className="review-text"
                value={text}
                spellCheck={false}
                rows={Math.min(8, Math.max(2, Math.ceil(text.length / 70)))}
                onChange={(e) => setText(e.target.value)}
              />
              {sample.errors.length > 0 && !changed && (
                <div className="review-errors">
                  {sample.errors.map((e, i) => (
                    <span key={i}>{e}</span>
                  ))}
                </div>
              )}

              <footer className="review-actions">
                {confirmDelete ? (
                  <span className="confirm">
                    Delete this sample?
                    <button className="danger" onClick={() => void remove()}>
                      Delete
                    </button>
                    <button onClick={() => setConfirmDelete(false)}>Cancel</button>
                  </span>
                ) : (
                  <button className="ghost" onClick={() => setConfirmDelete(true)} title="A muddled or accidental recording">
                    <Icon name="trash" size={13} /> Delete
                  </button>
                )}
                <span className="spacer" />
                <button
                  className="ghost"
                  disabled={index >= total - 1}
                  onClick={() => setIndex((i) => Math.min(i + 1, total - 1))}
                >
                  Skip
                </button>
                <button className="primary" disabled={saving} onClick={() => void save()}>
                  {changed ? "Save fix" : sample.reference !== null ? "Keep" : "Right as typed"}
                  <kbd>{MOD}↵</kbd>
                </button>
              </footer>
              <p className="review-keys">
                <kbd>{MOD}P</kbd> play again · <kbd>{MOD}↑</kbd> <kbd>{MOD}↓</kbd> previous and next
              </p>
            </section>
          )}

          <section className="group">
            <h2 className="group-title">All samples</h2>
            <ul className="card item-list review-list">
              {review.samples.map((s, i) => (
                <li key={s.name}>
                  <button className={`item-row${i === index ? " current" : ""}`} onClick={() => setIndex(i)}>
                    <span
                      className={`review-dot${s.reference === null ? "" : s.errors.length ? " errors" : " right"}`}
                      aria-label={s.reference === null ? "To review" : s.errors.length ? "Mistakes" : "Right"}
                    />
                    <span className="item-text">{s.reference ?? s.typed}</span>
                    <span className="item-meta">
                      {when(s.name)}
                      {s.app && <span className="dot-sep">{s.app}</span>}
                      {s.reference !== null && s.errors.length > 0 && (
                        <span className="dot-sep">
                          {s.errors.length} {s.errors.length === 1 ? "mistake" : "mistakes"}
                        </span>
                      )}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        </>
      )}
    </div>
  );
}

function AccuracyCard({ accuracy: a }: { accuracy: SampleReview["accuracy"] }) {
  const typed = rightShare(a.typed);
  const whisper = rightShare(a.whisper);
  const typedErrors = a.typed.substitutions + a.typed.deletions + a.typed.insertions;
  const whisperErrors = a.whisper.substitutions + a.whisper.deletions + a.whisper.insertions;
  const delta = whisperErrors - typedErrors;
  return (
    <section className="accuracy-card">
      <div className="accuracy-main">
        <strong>{typed.toFixed(1)}%</strong>
        <span>of words typed right</span>
      </div>
      <div className="accuracy-side">
        <p>
          <strong>{whisper.toFixed(1)}%</strong> from Whisper alone, before clean-up
        </p>
        <p>
          {delta > 0
            ? `Clean-up fixed ${delta} ${delta === 1 ? "mistake" : "mistakes"}`
            : delta < 0
              ? `Clean-up added ${-delta} ${delta === -1 ? "mistake" : "mistakes"}`
              : "Clean-up made no difference"}{" "}
          across {a.reviewed} reviewed ({a.typed.reference_words.toLocaleString()} words).
        </p>
        {a.common.length > 0 && (
          <div className="accuracy-common">
            {a.common.map(([error, n]) => (
              <span key={error}>
                {error}
                {n > 1 && <em>×{n}</em>}
              </span>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
