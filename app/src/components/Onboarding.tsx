import { useState } from "react";
import { api, errorText, isMac, type AccessReport } from "../api";
import { AccessCheck } from "./AccessCheck";
import { Icon, Logo } from "./Icon";
import { ModelDownload } from "./Models";
import { KeySetup } from "./OpenRouterKey";

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
      <div className="onboarding-drag" data-tauri-drag-region />
      <div className="onboarding-card">
        <Logo size={56} />
        <h1>Welcome to Quill</h1>
        <p className="lead">
          Meeting notes and dictation that run on your {isMac ? "Mac" : "PC"}. Works with Zoom, Meet, Teams or any other
          call, without a bot joining.
        </p>

        <div className="features">
          <div className="feature">
            <div className="access-icon me">
              <Icon name="mic" size={17} />
            </div>
            <div>
              <strong>Your microphone becomes “Me”</strong>
              <p>And the people on the call are told apart by voice, so the notes know who said what.</p>
            </div>
          </div>
          <div className="feature">
            <div className="access-icon">
              <Icon name="sparkle" size={17} />
            </div>
            <div>
              <strong>Notes written for you</strong>
              <p>Decisions, action items and a searchable transcript for every meeting.</p>
            </div>
          </div>
        </div>

        <div className="privacy-note">
          <Icon name="lock" size={15} />
          <span>
            Audio never leaves this computer. Only the text transcript is sent for summarizing, to the model you
            choose, or nowhere at all with a local model.
          </span>
        </div>

        <h2>Allow access</h2>
        <p className="muted small" style={{ marginBottom: 10 }}>
          {isMac
            ? "macOS asks the first time Quill listens. Click Check access, then Allow in both prompts."
            : "Click Check access to confirm Quill can hear both."}
        </p>
        <AccessCheck onResult={setReport} />
        <ModelDownload variant="onboarding" />
        <h2>Meeting notes</h2>
        <KeySetup />

        {error && <p className="error-text small" style={{ marginTop: 10 }}>{error}</p>}
        <div className="onboarding-actions">
          <span className="faint small">You can check again any time in Settings.</span>
          <span className="spacer" />
          <button className={ready ? "primary" : "ghost"} onClick={finish}>
            {ready ? "Start using Quill" : "Skip for now"}
            {ready && <Icon name="check" size={14} />}
          </button>
        </div>
      </div>
    </div>
  );
}
