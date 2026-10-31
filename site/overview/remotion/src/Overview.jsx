// Quill's overview video: people on calls (generated footage), the real app
// (recorded from the browser preview), a privacy card and an end card, cut
// to the narration, with captions and a music bed that dips under the voice.

import {
  AbsoluteFill,
  Audio,
  Easing,
  Img,
  OffthreadVideo,
  Sequence,
  interpolate,
  spring,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

// The website's light palette and type (site/styles.css).
const C = {
  bg: "#f4f3f0",
  card: "#ffffff",
  line: "#e8e6e1",
  text: "#121218",
  text2: "#5c5c66",
  accent: "#ff1053",
  rose: "#ffe0e8",
};
const FONT =
  '-apple-system, BlinkMacSystemFont, "SF Pro Display", "Segoe UI Variable Display", "Segoe UI", Inter, Roboto, "Helvetica Neue", Arial, sans-serif';

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" };
const easeOut = Easing.bezier(0.2, 0.8, 0.2, 1);

export const Overview = ({ scenes, music, fade, frames }) => {
  const voices = scenes.filter((s) => s.voice).map((s) => [s.voice.from, s.voice.from + s.voice.frames]);
  return (
    <AbsoluteFill style={{ background: C.bg, fontFamily: FONT, color: C.text }}>
      {scenes.map((s, i) => (
        // Each scene runs `fade` frames into the next, which fades in over it.
        <Sequence key={s.id} name={s.id} from={s.from} durationInFrames={s.frames + (i < scenes.length - 1 ? fade : 0)}>
          <FadeIn frames={i === 0 ? 0 : fade}>
            <Shot scene={s} />
          </FadeIn>
        </Sequence>
      ))}
      {scenes.map(
        (s) =>
          s.voice && (
            <Sequence key={`${s.id}-voice`} name={`${s.id} voice`} from={s.voice.from} durationInFrames={s.voice.frames}>
              <Audio src={staticFile(s.voice.src)} />
            </Sequence>
          ),
      )}
      {scenes.map(
        (s) =>
          s.voice &&
          s.kind !== "end" && (
            <Sequence
              key={`${s.id}-caption`}
              name={`${s.id} caption`}
              from={s.voice.from}
              durationInFrames={s.voice.frames + 10}
            >
              <Caption text={s.say} />
            </Sequence>
          ),
      )}
      {music && <Audio src={staticFile(music.src)} volume={(f) => musicVolume(f, frames, voices, music)} />}
    </AbsoluteFill>
  );
};

/** The music: in over half a second, down under the voice, out over the last 1.5 s. */
function musicVolume(f, total, voices, { volume, ducked }) {
  const ramp = 9;
  const duck = Math.max(
    0,
    ...voices.map(([a, b]) => Math.min(interpolate(f, [a - ramp, a], [0, 1], clamp), interpolate(f, [b, b + ramp * 2], [1, 0], clamp))),
  );
  const edges = Math.min(interpolate(f, [0, 15], [0, 1], clamp), interpolate(f, [total - 45, total - 1], [1, 0], clamp));
  return (volume - (volume - ducked) * duck) * edges;
}

const FadeIn = ({ frames, children }) => {
  const f = useCurrentFrame();
  const opacity = frames ? interpolate(f, [0, frames], [0, 1], clamp) : 1;
  return <AbsoluteFill style={{ opacity }}>{children}</AbsoluteFill>;
};

const Shot = ({ scene }) => {
  if (scene.kind === "footage") return <Footage scene={scene} />;
  if (scene.kind === "app") return <App scene={scene} />;
  if (scene.kind === "card") return <Card {...scene.card} />;
  return <End {...scene.end} />;
};

/** People: full frame, the camera drifting in, the clip's cuts played one after another. */
const Footage = ({ scene }) => {
  const f = useCurrentFrame();
  const scale = interpolate(f, [0, scene.frames], [1.02, 1.08], clamp);
  let at = 0;
  return (
    <AbsoluteFill style={{ background: "#000", transform: `scale(${scale})` }}>
      {scene.cuts.map(([a, b], i) => {
        const from = at;
        at += b - a;
        const last = i === scene.cuts.length - 1;
        return (
          <Sequence key={i} from={from} durationInFrames={last ? Infinity : b - a}>
            <OffthreadVideo
              src={staticFile(scene.src)}
              startFrom={a}
              muted
              style={{ width: "100%", height: "100%", objectFit: "cover" }}
            />
          </Sequence>
        );
      })}
    </AbsoluteFill>
  );
};

/** The app, as a window on the site's background, settling in and pushing slowly closer. */
const App = ({ scene }) => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const enter = spring({ frame: f, fps, config: { damping: 200, mass: 0.9 } });
  const push = interpolate(f, [0, scene.frames], [1, 1.035], clamp);
  return (
    <AbsoluteFill style={{ background: glow, alignItems: "center", justifyContent: "center", perspective: 1800 }}>
      <div
        style={{
          width: 1440,
          height: 900,
          borderRadius: 26,
          overflow: "hidden",
          background: C.card,
          border: `1px solid ${C.line}`,
          boxShadow: "0 2px 6px rgb(18 18 24 / .06), 0 40px 90px rgb(18 18 24 / .16)",
          transform: `translateY(${(1 - enter) * 40}px) rotateX(${(1 - enter) * 7}deg) scale(${(0.965 + 0.035 * enter) * push})`,
          marginTop: -30,
        }}
      >
        <OffthreadVideo src={staticFile(scene.src)} muted style={{ width: "100%", height: "100%", objectFit: "cover" }} />
      </div>
    </AbsoluteFill>
  );
};

const glow = `radial-gradient(1100px 700px at 80% 10%, ${C.rose} 0%, transparent 60%), radial-gradient(900px 600px at 10% 100%, #e6e5fb 0%, transparent 60%), ${C.bg}`;

/** Words rising into place, one after another. */
const Words = ({ text, delay = 0, step = 3, style }) => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <span style={style}>
      {text.split(" ").map((w, i) => {
        const t = spring({ frame: f - delay - i * step, fps, config: { damping: 200 } });
        return (
          <span key={i} style={{ display: "inline-block", opacity: t, transform: `translateY(${(1 - t) * 28}px)` }}>
            {w}&nbsp;
          </span>
        );
      })}
    </span>
  );
};

/** A card's icon: a padlock (the default) or a speech bubble. */
const CARD_ICONS = {
  lock: () => [<rect key="body" x="5" y="11" width="14" height="9" rx="2" />, <path key="shackle" d="M8 11V8a4 4 0 0 1 8 0v3" />],
  chat: () => <path d="M5 5h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-7l-4.5 3.5V17H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z" />,
};

const Card = ({ title, points, icon = "lock" }) => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pop = spring({ frame: f - 4, fps, config: { damping: 12, mass: 0.6 } });
  return (
    <AbsoluteFill style={{ background: glow, alignItems: "center", justifyContent: "center", gap: 44, paddingBottom: 90 }}>
      <div
        style={{
          width: 132,
          height: 132,
          borderRadius: 40,
          background: C.accent,
          display: "grid",
          placeItems: "center",
          transform: `scale(${pop})`,
          boxShadow: "0 18px 40px rgb(255 16 83 / .3)",
        }}
      >
        <svg width="66" height="66" viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          {CARD_ICONS[icon]()}
        </svg>
      </div>
      <Words
        text={title}
        delay={10}
        style={{ fontSize: 92, fontWeight: 800, letterSpacing: "-0.035em", textAlign: "center", maxWidth: 1500, lineHeight: 1.05 }}
      />
      <div style={{ display: "flex", gap: 20 }}>
        {points.map((p, i) => {
          const t = spring({ frame: f - 30 - i * 7, fps, config: { damping: 200 } });
          return (
            <div
              key={p}
              style={{
                opacity: t,
                transform: `translateY(${(1 - t) * 20}px)`,
                display: "flex",
                alignItems: "center",
                gap: 14,
                padding: "18px 28px",
                borderRadius: 999,
                background: C.card,
                border: `1px solid ${C.line}`,
                fontSize: 34,
                fontWeight: 600,
                color: C.text,
                boxShadow: "0 8px 24px rgb(18 18 24 / .06)",
              }}
            >
              <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round">
                <path d="M5 12.5l4.5 4.5L19 7.5" />
              </svg>
              {p}
            </div>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};

const End = ({ url }) => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pop = spring({ frame: f - 2, fps, config: { damping: 13, mass: 0.7 } });
  const rise = (delay) => {
    const t = spring({ frame: f - delay, fps, config: { damping: 200 } });
    return { opacity: t, transform: `translateY(${(1 - t) * 24}px)` };
  };
  return (
    <AbsoluteFill style={{ background: glow, alignItems: "center", justifyContent: "center", textAlign: "center" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 34, transform: `scale(${0.8 + 0.2 * pop})`, opacity: Math.min(1, pop * 1.5) }}>
        <Img src={staticFile("icon.svg")} style={{ width: 168, height: 168 }} />
        <span style={{ fontSize: 150, fontWeight: 800, letterSpacing: "-0.04em" }}>Quill</span>
      </div>
      <div style={{ ...rise(16), marginTop: 46, fontSize: 64, fontWeight: 700, letterSpacing: "-0.025em" }}>
        <span style={{ color: C.accent }}>Meeting notes</span> without the meeting bot
      </div>
      <div style={{ ...rise(28), marginTop: 22, fontSize: 38, color: C.text2 }}>Free for macOS and Windows</div>
      <div
        style={{
          ...rise(40),
          marginTop: 54,
          padding: "18px 36px",
          borderRadius: 999,
          background: C.text,
          color: "#fff",
          fontSize: 36,
          fontWeight: 600,
          letterSpacing: "-0.01em",
        }}
      >
        {url}
      </div>
    </AbsoluteFill>
  );
};

/** What's being said, low in the frame, on a dark pill that reads over anything. */
const Caption = ({ text }) => {
  const f = useCurrentFrame();
  const { durationInFrames } = useVideoConfig();
  const t = interpolate(f, [0, 8], [0, 1], { ...clamp, easing: easeOut });
  const out = interpolate(f, [durationInFrames - 8, durationInFrames], [1, 0], clamp);
  return (
    <AbsoluteFill style={{ justifyContent: "flex-end", alignItems: "center", paddingBottom: 64 }}>
      <div
        style={{
          opacity: t * out,
          transform: `translateY(${(1 - t) * 16}px)`,
          maxWidth: 1640,
          textWrap: "balance",
          padding: "16px 30px",
          borderRadius: 18,
          background: "rgb(18 18 24 / .74)",
          color: "#fff",
          fontSize: 42,
          fontWeight: 600,
          lineHeight: 1.25,
          letterSpacing: "-0.01em",
          textAlign: "center",
        }}
      >
        {text.replace(/\.\.\./g, "…")}
      </div>
    </AbsoluteFill>
  );
};
