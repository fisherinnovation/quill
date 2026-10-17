import { useEffect, useState } from "react";
import { api, type CallPrompt, events } from "../api";
import { Logo } from "./Icon";

/** Unanswered, the prompt goes away after this long (Not now / Keep recording). */
const TIMEOUT_MS = { record: 30_000, stop: 60_000 };

/**
 * The call prompt (its own transparent window at the top right that never
 * takes focus; see src-tauri/src/calls.rs): "Zoom is using your microphone.
 * Record this call?", or at the end, "Stop recording?".
 */
export function CallPromptCard() {
  const [prompt, setPrompt] = useState<CallPrompt | null>(null);
  const [hovered, setHovered] = useState(false);

  useEffect(() => {
    document.documentElement.classList.add("prompt-window");
    void api.callPrompt().then(setPrompt);
    const unlisten = events.onCallPrompt(setPrompt);
    return () => void unlisten.then((f) => f());
  }, []);

  // Not while the pointer is on it: the user may be about to click.
  useEffect(() => {
    if (!prompt || hovered) return;
    const timer = setTimeout(() => void api.answerCallPrompt("dismiss"), TIMEOUT_MS[prompt.kind]);
    return () => clearTimeout(timer);
  }, [prompt, hovered]);

  if (!prompt) return null;
  const answer = (a: "record" | "stop" | "dismiss") => {
    setPrompt(null);
    void api.answerCallPrompt(a);
  };

  return (
    <div className="prompt-stage">
      <div
        className="call-prompt"
        role="dialog"
        aria-label="Quill"
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      >
        <Logo size={30} />
        <div className="call-prompt-text">
          {prompt.kind === "record" ? (
            <>
              <strong>{prompt.app} is using your microphone</strong>
              <span>Record this call with Quill?</span>
            </>
          ) : (
            <>
              <strong>Your {prompt.app} call has ended</strong>
              <span>Stop recording and write the notes?</span>
            </>
          )}
        </div>
        <div className="call-prompt-actions">
          {prompt.kind === "record" ? (
            <>
              <button onClick={() => answer("dismiss")}>Not now</button>
              <button className="primary" onClick={() => answer("record")}>
                <span className="rec-dot" /> Record
              </button>
            </>
          ) : (
            <>
              <button onClick={() => answer("dismiss")}>Keep recording</button>
              <button className="primary" onClick={() => answer("stop")}>
                Stop
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
