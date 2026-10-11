import { useState } from "react";
import { api, errorText, isMac, type AccessReport } from "../api";
import { AccessCheck } from "./AccessCheck";

/** First-run screen: what Quill records, and getting the permissions granted. */
export function Onboarding({ onDone }: { onDone: () => void }) {
  const [report, setReport] = useState<AccessReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ready = report?.microphone.heard && report.system_audio.heard;

  async function finish() {
    try {
      await api.finishOnboarding();
      onDone();
    } catch (e) {
      setError(errorText(e));
    }
  }

  return (
    <div className="onboarding">
      <div className="onboarding-card">
        <h1>Welcome to Quill</h1>
        <p>
          Quill records your meetings and transcribes them on this {isMac ? "Mac" : "PC"}. It works with any meeting
          app, without joining the call as a bot. It records two tracks:
        </p>
        <ul className="tracks">
          <li>
            <span className="speaker me">Me</span> your microphone
          </li>
          <li>
            <span className="speaker others">Others</span> system audio: whatever you hear from Zoom, Meet, Teams…
          </li>
        </ul>
        <p className="muted">
          Audio never leaves this computer. Only the text transcript is sent for summarizing, to the model you pick in
          Settings, or nowhere at all if you use a local Ollama model.
        </p>

        <h2>Allow access</h2>
        <p>
          {isMac
            ? "macOS asks the first time Quill listens. Click Check access and choose Allow in both prompts. Quill then records for a few seconds and plays a short chime to test the system audio."
            : "Click Check access. Quill records for a few seconds and plays a short chime to test the system audio."}
        </p>
        <AccessCheck onResult={setReport} />

        {error && <p className="error-text small">{error}</p>}
        <div className="toolbar onboarding-actions">
          <span className="muted small">You can run this check again from Settings.</span>
          <span className="spacer" />
          <button type="button" className={ready ? "primary" : "subtle"} onClick={finish}>
            {ready ? "Start using Quill" : "Skip for now"}
          </button>
        </div>
      </div>
    </div>
  );
}
