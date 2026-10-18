import { useEffect, useRef, useState } from "react";
import {
  api,
  type DictationEntry,
  type DictationSettings,
  type DictationState,
  type Insights,
  type Suggestion,
} from "../api";
import { useDictationHistory, dictationTitle } from "./DictationHistory";
import { PastePermission, withRule } from "./DictationSettings";
import { Icon, type IconName } from "./Icon";
import { duration } from "./Insights";
import { ModelDownload } from "./Models";
import { Banner, Group, Keys, Row, groupByDay, timeOfDay } from "./ui";
import { VoiceWave } from "./VoiceWave";

/**
 * The Dictations page: whether dictation is ready, fixes waiting for the
 * user's OK, and the history. Its settings are under Settings → Dictation.
 */
export function DictationsView({
  state,
  onOpen,
  onSettings,
}: {
  state: DictationState;
  onOpen: (id: number) => void;
  /** Shows Settings → Dictation. */
  onSettings: () => void;
}) {
  const history = useDictationHistory();
  const [settings, setSettings] = useState<DictationSettings | null>(null);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [insights, setInsights] = useState<Insights | null>(null);

  useEffect(() => {
    void api.getSettings().then((s) => setSettings(s.dictation));
    void api.listSuggestions().then(setSuggestions);
  }, []);

  // The numbers, again after each new dictation.
  const latest = history.entries[0]?.id;
  useEffect(() => {
    void api
      .getInsights()
      .then(setInsights)
      .catch(() => setInsights(null));
  }, [latest]);

  async function decide(s: Suggestion, accept: boolean) {
    setSuggestions(await api.setSuggestionStatus(s.id, accept ? "accepted" : "dismissed"));
    if (accept) {
      // Read fresh: the settings may have changed since this page opened.
      const dictation = withRule((await api.getSettings()).dictation, s, true);
      await api.saveDictationSettings(dictation);
      setSettings(dictation);
    }
  }

  if (!settings) return null;
  const pending = suggestions.filter((s) => s.status === "pending");

  return (
    <div className="page dictations">
      <header className="page-head">
        <div>
          <h1 className="page-title">Dictations</h1>
          <p className="page-subtitle">Speak instead of typing, in any app.</p>
        </div>
        <span className="spacer" />
        <button onClick={onSettings}>
          <Icon name="settings" size={14} /> Settings
        </button>
      </header>

      <Hero
        state={state}
        hotkey={settings.hotkey}
        enabled={settings.enabled}
        days={insights && insights.dictation.count > 0 ? insights.days.map((d) => d.words) : null}
        onSettings={onSettings}
      />
      {insights && insights.dictation.count > 0 && <Numbers insights={insights} />}
      <ModelDownload variant="banner" />
      {state.error && (
        <Banner kind="error" icon="alert">
          {state.error}
        </Banner>
      )}
      <PastePermission />

      {pending.length > 0 && (
        <Group title="Learn from your fixes">
          {pending.map((s) => (
            <Row
              key={s.id}
              label={
                s.kind === "vocabulary" ? (
                  <>
                    Always spell <strong>{s.to}</strong> this way
                  </>
                ) : (
                  <>
                    Type <strong>{s.to}</strong> when you say “{s.from}”
                  </>
                )
              }
              hint={`${s.kind === "vocabulary" ? "Adds it to your vocabulary" : "Adds a replacement"}. From your fix${s.app ? ` in ${s.app}` : ""}${s.seen > 1 ? `, seen ${s.seen} times` : ""}.`}
            >
              <span className="row-gap">
                <button className="ghost" onClick={() => void decide(s, false)}>
                  Dismiss
                </button>
                <button className="primary" onClick={() => void decide(s, true)}>
                  Add
                </button>
              </span>
            </Row>
          ))}
        </Group>
      )}

      {history.entries.length === 0 ? (
        <div className="card home-empty">
          <div className="home-card-icon">
            <Icon name="mic" size={17} />
          </div>
          {settings.keep_history ? (
            <div>
              <strong>Your dictations will appear here</strong>
              <p className="muted small">
                Open one to copy it, or to correct it so Quill learns the names and terms it got wrong.
              </p>
            </div>
          ) : (
            <>
              <div>
                <strong>History is off</strong>
                <p className="muted small">Dictation still works; what you say just isn't kept here.</p>
              </div>
              <span className="spacer" />
              <button onClick={onSettings}>Turn on…</button>
            </>
          )}
        </div>
      ) : (
        <>
          {groupByDay(history.entries, (d) => d.created_at).map(([label, group]) => {
            const words = group.reduce((n, d) => n + wordCount(d), 0);
            return (
              <section className="group" key={label}>
                <h2 className="group-title dictation-day">
                  {label}
                  <span className="dictation-day-total">
                    {group.length} {group.length === 1 ? "dictation" : "dictations"} · {words.toLocaleString()}{" "}
                    {words === 1 ? "word" : "words"}
                  </span>
                </h2>
                <ul className="card item-list dictation-list">
                  {group.map((d) => (
                    <li key={d.id}>
                      <button className="item-row dictation-row" onClick={() => onOpen(d.id)}>
                        <AppBadge app={d.app} />
                        <span className="dictation-row-body">
                          <span className="item-text">{dictationTitle(d)}</span>
                          <span className="item-meta">
                            {timeOfDay(d.created_at)}
                            {d.app && <span className="dot-sep">{d.app}</span>}
                            <span className="dot-sep">
                              {wordCount(d)} {wordCount(d) === 1 ? "word" : "words"}
                            </span>
                            {d.corrected && <span className="dot-sep">corrected</span>}
                            {!d.pasted && <span className="dot-sep">left on clipboard</span>}
                          </span>
                        </span>
                        <Icon name="chevron-right" size={14} />
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
          {history.more && (
            <button className="show-more" onClick={history.loadMore}>
              Show more dictations
            </button>
          )}
        </>
      )}
    </div>
  );
}

function Hero({
  state,
  hotkey,
  enabled,
  days,
  onSettings,
}: {
  state: DictationState;
  hotkey: string;
  enabled: boolean;
  /** Words per day over the last 30 days, which shape the wave; null before any. */
  days: number[] | null;
  onSettings: () => void;
}) {
  const phase = enabled ? state.phase : "off";
  return (
    <div className={`hero ${phase}`}>
      <VoiceWave days={days ?? NO_DAYS} phase={phase} />
      <div className="hero-body">
        <span className="hero-status">
          <span className="hero-dot" />
          {phase === "listening"
            ? "Listening"
            : phase === "processing"
              ? "Typing"
              : phase === "off"
                ? "Off"
                : "Ready"}
        </span>
        <div className="hero-title">
          {phase === "listening" ? (
            <>Listening…</>
          ) : phase === "processing" ? (
            <>Typing it out…</>
          ) : phase === "off" ? (
            <>Dictation is off</>
          ) : (
            <>
              Hold <Keys hotkey={hotkey} large /> and speak
            </>
          )}
        </div>
        <p>
          {phase === "listening"
            ? "Let go (or tap the shortcut again) to type it. Esc cancels."
            : phase === "processing"
              ? "The text goes wherever your cursor is."
              : phase === "off"
                ? "Turn it on to dictate in any app."
                : "Let go and it's typed where your cursor is. Or tap to start and tap again to stop."}
        </p>
        {phase === "off" && (
          <button className="primary" onClick={onSettings}>
            Turn on…
          </button>
        )}
      </div>
      {days && phase === "idle" && <span className="hero-caption">Your last 30 days</span>}
    </div>
  );
}

const NO_DAYS: number[] = [];

/** All-time dictation numbers, counting up once when the page opens. */
function Numbers({ insights }: { insights: Insights }) {
  const d = insights.dictation;
  const stats: { icon: IconName; value: number; format: (v: number) => string; label: string }[] = [
    { icon: "text", value: d.words, format: (v) => Math.round(v).toLocaleString(), label: "words dictated" },
    {
      icon: "clock",
      value: d.minutes_saved,
      format: (v) => duration(v * 60_000),
      label: "saved over typing",
    },
    {
      icon: "bolt",
      value: d.speaking_wpm ?? 0,
      format: (v) => (d.speaking_wpm ? `${Math.round(v)} wpm` : "–"),
      label: "when you speak",
    },
    {
      icon: "flame",
      value: insights.streak_days,
      format: (v) => `${Math.round(v)} ${Math.round(v) === 1 ? "day" : "days"}`,
      label: "in a row",
    },
  ];
  const progress = useCountUp();
  return (
    <div className="dictation-numbers">
      {stats.map((s) => (
        <div className="dictation-number" key={s.label}>
          <Icon name={s.icon} size={15} />
          <strong>{s.format(s.value * progress)}</strong>
          <span>{s.label}</span>
        </div>
      ))}
    </div>
  );
}

/** 0 → 1 over most of a second, eased, once per mount (at once with reduced motion). */
function useCountUp(): number {
  const [progress, setProgress] = useState(() =>
    window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 1 : 0,
  );
  const started = useRef<number | null>(null);
  useEffect(() => {
    if (progress >= 1) return;
    let frame = 0;
    const tick = (now: number) => {
      started.current ??= now;
      const t = Math.min(1, (now - started.current) / 900);
      setProgress(1 - Math.pow(1 - t, 3));
      if (t < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
    // Runs once: progress only grows.
  }, []);
  return progress;
}

function wordCount(d: DictationEntry): number {
  const text = (d.corrected ?? d.text).trim();
  return text ? text.split(/\s+/).length : 0;
}

/** The palette's soft tints, one per app (by name), for its badge. */
const TINTS = ["fuchsia", "grape", "sky", "slate"] as const;

/** An app's first letter on a tint of its own, so the list scans by app. */
function AppBadge({ app }: { app: string | null }) {
  if (!app) {
    return (
      <span className="app-badge none" aria-hidden>
        <Icon name="mic" size={13} />
      </span>
    );
  }
  let hash = 0;
  for (const ch of app) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return (
    <span className={`app-badge ${TINTS[hash % TINTS.length]}`} aria-hidden>
      {app.trim().charAt(0).toUpperCase()}
    </span>
  );
}
