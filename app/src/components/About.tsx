import { useEffect, useState } from "react";
import { type AboutInfo, type AboutLink, api, errorText } from "../api";
import { Icon, Logo } from "./Icon";

/** "2026-10-12" → "Oct 12, 2026" in the user's locale. */
function longDate(day: string): string {
  const date = new Date(`${day}T12:00:00`);
  return Number.isNaN(date.getTime())
    ? day
    : date.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

/** The text "Copy diagnostic info" puts on the clipboard: no keys, no content. */
function diagnostics(info: AboutInfo): string {
  return [
    `Quill ${info.version}${info.debug ? " (debug build)" : ""}`,
    `Build: ${info.commit || "unknown"}${info.commit_date ? ` (${info.commit_date})` : ""}`,
    `System: ${info.os}, ${info.arch}`,
    `Speech model: ${info.speech_model}`,
    `Clean-up model: ${info.cleanup_model ?? "off"}`,
    `Summary model: ${info.summary_model}`,
    `Tell speakers apart: ${info.identify_speakers ? "on" : "off"}`,
  ].join("\n");
}

const LINKS: [AboutLink, string][] = [
  ["repository", "Website"],
  ["releases", "Release notes"],
  ["report_problem", "Report a problem"],
  ["security", "Privacy & security"],
  ["license", "License"],
];

/** The About window (opened from the app menu or the menu-bar icon). */
export function About() {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .aboutInfo()
      .then(setInfo)
      .catch((e) => setError(errorText(e)));
  }, []);

  async function copy() {
    if (!info) return;
    await navigator.clipboard.writeText(diagnostics(info));
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="about">
      <div className="about-drag" data-tauri-drag-region />
      <header className="about-head">
        <Logo size={72} />
        <h1>Quill</h1>
        <p className="about-tagline">Meeting notes and dictation that run on your computer.</p>
        {info && (
          <p className="about-version">
            Version {info.version}
            {info.commit && (
              <>
                {" "}
                · <span className="mono">{info.commit}</span>
                {info.commit_date && ` · ${longDate(info.commit_date)}`}
              </>
            )}
            {info.debug && <span className="about-tag">debug build</span>}
          </p>
        )}
      </header>

      {error && <p className="error-text small">{error}</p>}

      {info && (
        <dl className="about-facts">
          <dt>System</dt>
          <dd>
            {info.os}, {info.arch}
          </dd>
          <dt>Speech</dt>
          <dd>Whisper {info.speech_model}, on this computer</dd>
          <dt>Speakers</dt>
          <dd>{info.identify_speakers ? "Told apart by voice, on this computer" : "Off"}</dd>
          <dt>Clean-up</dt>
          <dd>{info.cleanup_model ?? "Off"}</dd>
          <dt>Notes</dt>
          <dd>{info.summary_model}</dd>
        </dl>
      )}

      <div className="about-actions">
        <button onClick={() => void copy()} disabled={!info}>
          <Icon name={copied ? "check" : "copy"} size={14} /> {copied ? "Copied" : "Copy diagnostic info"}
        </button>
        <button onClick={() => void api.showLogs()}>
          <Icon name="folder" size={14} /> Show log files
        </button>
      </div>

      <nav className="about-links" aria-label="Links">
        {LINKS.map(([link, label]) => (
          <button key={link} className="link" onClick={() => void api.openLink(link)}>
            {label}
          </button>
        ))}
      </nav>

      <section className="about-credits">
        <h2>Built with</h2>
        <p>
          whisper.cpp and OpenAI's Whisper models (MIT), Silero
          VAD (MIT), pyannote segmentation 3.0 (MIT), NVIDIA NeMo TitaNet (CC BY 4.0) through sherpa-onnx and ONNX
          Runtime (Apache-2.0, MIT), WebRTC echo cancellation via sonora, Tauri and React. Summaries and clean-up use
          the model you choose, through OpenRouter or Ollama.
        </p>
      </section>

      <footer className="about-footer">
        © 2026 Fisher Innovation and the Quill contributors
        <br />
        Free and open source under MIT or Apache-2.0
      </footer>
    </div>
  );
}
