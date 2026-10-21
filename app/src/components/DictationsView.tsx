import { useEffect, useState } from "react";
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
import { AppIcon } from "./AppIcon";
import { Banner, Group, Keys, Row, groupByDay, timeOfDay, useCountUp } from "./ui";
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
  const [only, setOnly] = useState<Kind | null>(null);

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
          <KindFilter entries={history.entries} only={only} onChange={setOnly} />
          {groupByDay(
            history.entries.filter((d) => !only || kindOf(d) === only),
            (d) => d.created_at,
          ).map(([label, group]) => {
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
                      <button className={`item-row dictation-row ${kindOf(d)}`} onClick={() => onOpen(d.id)}>
                        <AppIcon id={d.app_id} name={d.app} />
                        <span className="dictation-row-body">
                          <span className="item-text">{dictationTitle(d)}</span>
                          <span className="item-meta">
                            {timeOfDay(d.created_at)}
                            {d.app && <span className="dot-sep">{d.app}</span>}
                            <span className="dot-sep">
                              {wordCount(d)} {wordCount(d) === 1 ? "word" : "words"}
                            </span>
                          </span>
                        </span>
                        <Tags entry={d} />
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
    <div className="stat-tiles">
      {stats.map((s) => (
        <div className="stat-tile" key={s.label}>
          <Icon name={s.icon} size={15} />
          <strong>{s.format(s.value * progress)}</strong>
          <span>{s.label}</span>
        </div>
      ))}
    </div>
  );
}

function wordCount(d: DictationEntry): number {
  const text = (d.corrected ?? d.text).trim();
  return text ? text.split(/\s+/).length : 0;
}

/**
 * What happened to a dictation, which colours it: left on the clipboard
 * (it wasn't typed: the one to act on), corrected by the user, tidied up by
 * clean-up, or typed as spoken.
 */
type Kind = "clipboard" | "corrected" | "tidied" | "spoken";

const KINDS: { kind: Kind; label: string; icon: IconName; hint: string }[] = [
  { kind: "tidied", label: "Tidied up", icon: "sparkle", hint: "Clean-up changed what Whisper heard" },
  { kind: "spoken", label: "As spoken", icon: "mic", hint: "Typed exactly as heard" },
  { kind: "corrected", label: "Corrected", icon: "text", hint: "You fixed it afterwards" },
  { kind: "clipboard", label: "Not typed", icon: "copy", hint: "Left on the clipboard: Quill couldn't paste" },
];

function kindOf(d: DictationEntry): Kind {
  if (!d.pasted) return "clipboard";
  if (d.corrected) return "corrected";
  if (d.raw) return "tidied";
  return "spoken";
}

/** All, or one kind: the chips are also the colours' legend. */
function KindFilter({
  entries,
  only,
  onChange,
}: {
  entries: DictationEntry[];
  only: Kind | null;
  onChange: (kind: Kind | null) => void;
}) {
  const counts = new Map<Kind, number>();
  for (const d of entries) counts.set(kindOf(d), (counts.get(kindOf(d)) ?? 0) + 1);
  const shown = KINDS.filter((k) => counts.get(k.kind));
  // One kind only: nothing to tell apart.
  if (shown.length < 2 && !only) return null;
  return (
    <div className="kind-filter" role="group" aria-label="Show">
      <button className={`kind-chip${only === null ? " on" : ""}`} aria-pressed={only === null} onClick={() => onChange(null)}>
        All <span className="kind-count">{entries.length}</span>
      </button>
      {shown.map((k) => (
        <button
          key={k.kind}
          className={`kind-chip ${k.kind}${only === k.kind ? " on" : ""}`}
          aria-pressed={only === k.kind}
          title={k.hint}
          onClick={() => onChange(only === k.kind ? null : k.kind)}
        >
          <span className="kind-dot" />
          {k.label} <span className="kind-count">{counts.get(k.kind)}</span>
        </button>
      ))}
    </div>
  );
}

/**
 * What's notable about a dictation, at a glance: its kind (unless typed as
 * spoken, the usual case), and whether it's a question, a list, or long.
 */
function Tags({ entry }: { entry: DictationEntry }) {
  const kind = kindOf(entry);
  const text = (entry.corrected ?? entry.text).trim();
  const facts: { label: string; icon?: IconName }[] = [];
  if (/^\s*(?:[-*•]|\d+[.)])\s/m.test(text)) facts.push({ label: "List" });
  else if (text.endsWith("?")) facts.push({ label: "Question" });
  if (entry.audio_ms >= LONG_MS) facts.push({ label: clock(entry.audio_ms), icon: "clock" });
  const k = KINDS.find((x) => x.kind === kind);
  return (
    <span className="dictation-tags">
      {facts.map((f) => (
        <span key={f.label} className="dictation-tag">
          {f.icon && <Icon name={f.icon} size={11} />}
          {f.label}
        </span>
      ))}
      {k && kind !== "spoken" && (
        <span className={`dictation-tag kind ${kind}`} title={k.hint}>
          <Icon name={k.icon} size={11} />
          {k.label}
        </span>
      )}
    </span>
  );
}

/** Long enough to be worth pointing out (most dictations are a few seconds). */
const LONG_MS = 20_000;

/** "0:41", "2:05". */
function clock(ms: number): string {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}


