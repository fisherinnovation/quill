import type { Segment } from "./api";

/**
 * How far someone has to be into a line, and from its end, for another
 * person starting to count as cutting in. Closer than this is just where
 * the two tracks put their segment boundaries.
 */
const CUT_IN_MS = 1_500;
/** Cutting in with this many words or fewer ("Yeah, makes sense") doesn't take the floor. */
const BACKCHANNEL_WORDS = 3;
// Same rules as `turns` in api.ts.
const TURN_MAX_GAP_MS = 5_000;
const TURN_MAX_LEN_MS = 60_000;

/** A line of the transcript as the Transcript tab shows it. */
export interface Line extends Segment {
  /** Picks up the speaker's sentence after someone cut in. */
  resumed: boolean;
  /** Stops mid-sentence because someone cut in. */
  cut: boolean;
  /** The people who were still talking when this line started. */
  over: string[];
}

/**
 * The transcript in the order things were said, with talking over each
 * other kept visible.
 *
 * Joining segments into turns places a whole turn at its start, so when
 * someone cuts in, their line used to come after everything the first
 * speaker said, even the part that answered them. Here a line someone cuts
 * into is split where they did, and continues after them. Whisper gives no
 * word timings, so the split is placed by how far through the line's time
 * it is, at the nearest word.
 */
export function conversation(segments: Segment[]): Line[] {
  const pieces = segments.flatMap((s) => split(s, cutIns(s, segments)));
  // Someone cutting in comes before the line they cut into goes on.
  pieces.sort((a, b) => a.start_ms - b.start_ms || Number(a.resumed) - Number(b.resumed));

  const lines: Line[] = [];
  for (const p of pieces) {
    const last = lines[lines.length - 1];
    if (
      last &&
      last.speaker === p.speaker &&
      p.start_ms <= last.end_ms + TURN_MAX_GAP_MS &&
      p.end_ms - last.start_ms <= TURN_MAX_LEN_MS
    ) {
      last.text += " " + p.text;
      last.end_ms = Math.max(last.end_ms, p.end_ms);
      last.cut = p.cut;
    } else {
      lines.push(p);
    }
  }
  for (const line of lines) {
    if (!line.resumed) line.over = talkingAt(line, segments);
  }
  return lines;
}

/** Whether `speaker` is talking for a good part of `line`. */
export function talksDuring(line: Line, speaker: string | null | undefined, segments: Segment[]): boolean {
  if ((speaker ?? null) === (line.speaker ?? null)) return true;
  const enough = Math.min(CUT_IN_MS, (line.end_ms - line.start_ms) / 2);
  return segments.some(
    (s) =>
      (s.speaker ?? null) === (speaker ?? null) &&
      Math.min(s.end_ms, line.end_ms) - Math.max(s.start_ms, line.start_ms) >= enough,
  );
}

/** When other people start talking well inside `s`. */
function cutIns(s: Segment, segments: Segment[]): number[] {
  const times = segments
    .filter(
      (o) =>
        o.speaker !== s.speaker &&
        o.start_ms > s.start_ms + CUT_IN_MS &&
        o.start_ms < s.end_ms - CUT_IN_MS &&
        words(o.text).length > BACKCHANNEL_WORDS,
    )
    .map((o) => o.start_ms);
  return [...new Set(times)].sort((a, b) => a - b);
}

/** `s` cut at each of `times`, at the nearest word. */
function split(s: Segment, times: number[]): Line[] {
  const all = words(s.text);
  const length = s.text.length;
  // Where each word ends, in characters, to place the cuts by time.
  const ends: number[] = [];
  let at = 0;
  for (const w of all) {
    at += w.length + 1;
    ends.push(at);
  }
  const cuts: { word: number; ms: number }[] = [];
  for (const ms of times) {
    const target = ((ms - s.start_ms) / (s.end_ms - s.start_ms)) * length;
    let word = 0;
    while (word < ends.length - 1 && Math.abs(ends[word + 1] - target) < Math.abs(ends[word] - target)) word++;
    const prev = cuts[cuts.length - 1]?.word ?? -1;
    // Each piece keeps at least a word.
    if (word > prev && word < all.length - 1) cuts.push({ word, ms });
  }

  const pieces: Line[] = [];
  let from = 0;
  let start = s.start_ms;
  for (const [i, cut] of [...cuts, { word: all.length - 1, ms: s.end_ms }].entries()) {
    pieces.push({
      ...s,
      start_ms: start,
      end_ms: cut.ms,
      text: all.slice(from, cut.word + 1).join(" "),
      resumed: i > 0,
      cut: i < cuts.length,
      over: [],
    });
    from = cut.word + 1;
    start = cut.ms;
  }
  return pieces;
}

function talkingAt(line: Line, segments: Segment[]): string[] {
  const names = segments
    .filter(
      (s) =>
        s.speaker &&
        s.speaker !== line.speaker &&
        s.start_ms <= line.start_ms - CUT_IN_MS &&
        s.end_ms >= line.start_ms + CUT_IN_MS,
    )
    .map((s) => s.speaker as string);
  return [...new Set(names)];
}

function words(text: string): string[] {
  return text.split(/\s+/).filter(Boolean);
}
