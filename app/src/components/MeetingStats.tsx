// A meeting's Stats tab: who talked and how much, when, the moments worth
// noticing and the words that came up. The numbers come from Rust
// (quill_core::stats); this only lays them out.
import { type CSSProperties, useEffect, useState } from "react";
import { api, clock, errorText, type MeetingStats, type SpeakerStats, type Stretch } from "../api";
import { Icon } from "./Icon";
import { Stat } from "./ui";

const ME = "Me";
const number = new Intl.NumberFormat();

/** How the transcript names someone. */
const display = (name: string) => (name === ME ? "You" : name);

/**
 * Each speaker's colour, matching their transcript avatar: "Me" has its own,
 * everyone else takes the next slot in order of first appearance.
 */
function colors(speakers: SpeakerStats[]): Map<string, string> {
  let slot = 0;
  return new Map(
    speakers.map((s) => [s.name, s.name === ME ? "var(--me)" : `var(--spk-${(slot++ % 8) + 1})`]),
  );
}

/** "45 s", "2 min 10 s", "14 min". */
function length(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  return m >= 10 || s % 60 === 0 ? `${Math.round(s / 60)} min` : `${m} min ${s % 60} s`;
}

const percent = (share: number) => `${Math.round(share * 100)}%`;

/** One sentence on the shape of the conversation. */
function headline(stats: MeetingStats): string {
  const people = stats.speakers.filter((s) => s.talk_ms > 0);
  if (people.length === 0) return "";
  if (people.length === 1) {
    return people[0].name === ME ? "Only you were heard on this recording." : `Only ${people[0].name} was heard.`;
  }
  const byTalk = [...people].sort((a, b) => b.talk_ms - a.talk_ms);
  const top = byTalk[0];
  const spread = top.share - byTalk[byTalk.length - 1].share;
  let text =
    people.length >= 3 && spread < 0.15
      ? "The talking was shared evenly."
      : top.share >= 0.5
        ? `${display(top.name)} did most of the talking (${percent(top.share)}).`
        : `${display(top.name)} talked the most (${percent(top.share)}).`;
  const asker = [...people].sort((a, b) => b.questions - a.questions)[0];
  if (stats.questions >= 3 && asker.questions > stats.questions / 2) {
    text += ` ${display(asker.name)} asked ${asker.questions} of the ${stats.questions} questions.`;
  }
  return text;
}

export function MeetingStatsView({ id, version, onJump }: { id: string; version: number; onJump: (ms: number) => void }) {
  const [stats, setStats] = useState<MeetingStats | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .meetingStats(id)
      .then(setStats)
      .catch((e) => setError(errorText(e)));
  }, [id, version]);

  if (error) return <p className="error-text">{error}</p>;
  if (!stats) return null;
  if (stats.speakers.length === 0) {
    return (
      <div className="placeholder">
        <div className="placeholder-icon">
          <Icon name="chart" size={20} />
        </div>
        <strong>No stats yet</strong>
        <p className="muted">Once the recording is transcribed, you'll see who talked, when, and what came up.</p>
      </div>
    );
  }

  const palette = colors(stats.speakers);
  const asker = [...stats.speakers].sort((a, b) => b.questions - a.questions)[0];
  // Bars are scaled to the biggest share, so the top talker fills the row.
  const maxShare = Math.max(...stats.speakers.map((s) => s.share));
  return (
    <section className="meeting-stats">
      <p className="stats-headline">{headline(stats)}</p>

      <div className="stat-grid">
        <Stat
          label="Talking"
          value={length(stats.speech_ms)}
          note={`Of a ${length(stats.span_ms)} recording`}
        />
        <Stat
          label="Words"
          value={number.format(stats.words)}
          note={stats.wpm ? `${Math.round(stats.wpm)} words a minute` : "Too short to time"}
        />
        <Stat
          label="Turns"
          value={number.format(stats.turns)}
          note={
            stats.speakers.length > 1 && stats.turns > 1
              ? `A new voice every ${length(stats.span_ms / stats.turns)}`
              : "One voice throughout"
          }
        />
        <Stat
          label="Questions"
          value={number.format(stats.questions)}
          note={stats.questions > 0 ? `Most from ${display(asker.name)} (${asker.questions})` : "None asked"}
        />
      </div>

      <h2 className="group-title">Who talked</h2>
      <div className="card talkers">
        {stats.speakers.map((s) => (
          <Talker key={s.name} speaker={s} color={palette.get(s.name)} max={maxShare} />
        ))}
      </div>

      <h2 className="group-title">Who spoke when</h2>
      <Timeline stats={stats} palette={palette} onJump={onJump} />

      <Moments stats={stats} onJump={onJump} />

      {(stats.top_words.length > 0 || stats.top_phrases.length > 0) && (
        <>
          <h2 className="group-title">What came up</h2>
          <div className="card words-card">
            {stats.top_words.length > 0 && <TopWords words={stats.top_words} />}
            {stats.top_phrases.length > 0 && (
              <div className="phrases">
                <span className="faint small">Phrases said more than once</span>
                <div className="phrase-list">
                  {stats.top_phrases.map((p) => (
                    <span key={p.text} className="phrase">
                      {p.text} <span className="faint">×{p.count}</span>
                    </span>
                  ))}
                </div>
              </div>
            )}
          </div>
        </>
      )}
    </section>
  );
}

/** One row of "Who talked": share of the talking, and how they talked. */
function Talker({ speaker: s, color, max }: { speaker: SpeakerStats; color?: string; max: number }) {
  const details = [
    length(s.talk_ms),
    `${number.format(s.words)} words`,
    s.wpm && `${Math.round(s.wpm)} wpm`,
    `${s.turns} ${s.turns === 1 ? "turn" : "turns"}`,
    s.questions > 0 && `${s.questions} ${s.questions === 1 ? "question" : "questions"}`,
    s.longest_stretch_ms >= 20_000 && `longest ${length(s.longest_stretch_ms)}`,
    s.fillers > 0 && `${s.fillers} ${s.fillers === 1 ? "filler" : "fillers"}`,
  ].filter(Boolean);
  return (
    <div className="talker" style={{ "--c": color } as CSSProperties}>
      <span className="talker-name">
        <span className="swatch" />
        {display(s.name)}
      </span>
      <div className="talker-bar" role="img" aria-label={`${display(s.name)}: ${percent(s.share)} of the talking`}>
        <div style={{ width: `${max > 0 ? (s.share / max) * 100 : 0}%` }} />
      </div>
      <span className="talker-share">{percent(s.share)}</span>
      <p className="talker-details">
        {details.join(" · ")}
        {s.signature_words.length > 0 && (
          <>
            {" · "}
            <span className="faint">kept saying</span> {s.signature_words.map((w) => `“${w}”`).join(", ")}
          </>
        )}
      </p>
    </div>
  );
}

/** A lane per speaker, each stretch of talking placed in time. */
function Timeline({
  stats,
  palette,
  onJump,
}: {
  stats: MeetingStats;
  palette: Map<string, string>;
  onJump: (ms: number) => void;
}) {
  const [hover, setHover] = useState<{ stretch: Stretch; x: number; lane: number } | null>(null);
  const span = Math.max(stats.span_ms, 1);
  const ticks = [0, 0.25, 0.5, 0.75, 1];
  return (
    <div className="card timeline">
      <div className="timeline-lanes" onMouseLeave={() => setHover(null)}>
        {stats.speakers.map((s, lane) => (
          <div key={s.name} className="timeline-lane" style={{ "--c": palette.get(s.name) } as CSSProperties}>
            <span className="timeline-name">{display(s.name)}</span>
            <div className="timeline-track">
              {stats.stretches
                .filter((st) => st.speaker === s.name)
                .map((st) => {
                  const left = (st.start_ms / span) * 100;
                  const width = ((st.end_ms - st.start_ms) / span) * 100;
                  const show = (x: number) => setHover({ stretch: st, x, lane });
                  return (
                    <button
                      key={st.start_ms}
                      className="timeline-span"
                      style={{ left: `${left}%`, width: `${width}%` }}
                      aria-label={`${display(s.name)}, ${clock(st.start_ms)} to ${clock(st.end_ms)}: ${st.preview}`}
                      onMouseEnter={() => show(left + width / 2)}
                      onFocus={() => show(left + width / 2)}
                      onBlur={() => setHover(null)}
                      onClick={() => onJump(st.start_ms)}
                    >
                      <span />
                    </button>
                  );
                })}
            </div>
          </div>
        ))}
        {hover && (
          <div
            className="timeline-tip"
            style={{ left: `clamp(0px, calc(${hover.x}% - 130px), calc(100% - 260px))`, top: (hover.lane + 1) * 30 + 4 }}
          >
            <strong>{display(hover.stretch.speaker)}</strong>
            <span className="faint">
              {" "}
              {clock(hover.stretch.start_ms)}–{clock(hover.stretch.end_ms)} ·{" "}
              {length(hover.stretch.end_ms - hover.stretch.start_ms)}
            </span>
            <p>“{hover.stretch.preview}…”</p>
            <span className="faint small">Click to read it in the transcript</span>
          </div>
        )}
      </div>
      <div className="timeline-axis">
        {ticks.map((t) => (
          <span key={t}>{clock(t * stats.span_ms)}</span>
        ))}
      </div>
    </div>
  );
}

/** The moments worth noticing, each with a way to go and read it. */
function Moments({ stats, onJump }: { stats: MeetingStats; onJump: (ms: number) => void }) {
  const people = stats.speakers.filter((s) => s.talk_ms > 0);
  const timed = people.filter((s) => s.wpm !== null);
  const fastest = timed.length >= 2 ? [...timed].sort((a, b) => (b.wpm ?? 0) - (a.wpm ?? 0))[0] : null;
  const fillers = [...people].sort((a, b) => b.fillers - a.fillers)[0];
  const cards: { icon: Parameters<typeof Icon>[0]["name"]; title: string; text: string; at?: number }[] = [];
  const longest = stats.longest_stretch;
  if (longest && longest.end_ms - longest.start_ms >= 20_000) {
    cards.push({
      icon: "speaker",
      title: `${display(longest.speaker)} held the floor for ${length(longest.end_ms - longest.start_ms)}`,
      text: `The longest stretch without a break, from ${clock(longest.start_ms)}: “${longest.preview}…”`,
      at: longest.start_ms,
    });
  }
  if (stats.busiest_pair && people.length >= 2) {
    const { a, b, handovers } = stats.busiest_pair;
    cards.push({
      icon: "users",
      title: `${display(a)} and ${display(b)} went back and forth ${handovers} times`,
      text: people.length > 2 ? "More than any other two people on the call." : "The rhythm of the conversation.",
    });
  }
  if (stats.longest_silence) {
    const { start_ms, end_ms } = stats.longest_silence;
    cards.push({
      icon: "clock",
      title: `${length(end_ms - start_ms)} of quiet`,
      text: `The longest pause, at ${clock(start_ms)}.`,
      at: start_ms,
    });
  }
  if (fastest?.wpm && stats.wpm) {
    cards.push({
      icon: "bolt",
      title: `${display(fastest.name)} spoke fastest, at ${Math.round(fastest.wpm)} words a minute`,
      text: `The call averaged ${Math.round(stats.wpm)}.`,
    });
  }
  if (fillers && fillers.fillers >= 3) {
    cards.push({
      icon: "wave",
      title: `${display(fillers.name)} used the most filler words (${fillers.fillers})`,
      text: "Counting “um”, “you know”, “basically” and the like, as far as the transcript caught them.",
    });
  }
  if (cards.length === 0) return null;
  return (
    <>
      <h2 className="group-title">Moments</h2>
      <div className="moments">
        {cards.map((c) => (
          <div key={c.title} className="card moment">
            <Icon name={c.icon} size={16} />
            <div>
              <strong>{c.title}</strong>
              <p className="muted small">{c.text}</p>
              {c.at !== undefined && (
                <button className="link small" onClick={() => onJump(c.at as number)}>
                  Read it in the transcript
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

/** The most used words, as a ranked list of bars in one colour. */
function TopWords({ words }: { words: MeetingStats["top_words"] }) {
  const max = Math.max(...words.map((w) => w.count), 1);
  // Ranked down the first column, then the second.
  return (
    <ol className="top-words" style={{ gridTemplateRows: `repeat(${Math.ceil(words.length / 2)}, auto)` }}>
      {words.map((w) => (
        <li key={w.text}>
          <span className="top-word">{w.text}</span>
          <div className="top-word-bar">
            <div style={{ width: `${(w.count / max) * 100}%` }} />
          </div>
          <span className="top-word-count">{w.count}</span>
        </li>
      ))}
    </ol>
  );
}
