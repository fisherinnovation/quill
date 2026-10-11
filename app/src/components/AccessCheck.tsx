import { useState } from "react";
import { api, errorText, isMac, type AccessReport, type TrackAccess } from "../api";

/** "Check access" button and per-source results, used in onboarding and Settings. */
export function AccessCheck({ onResult }: { onResult?: (report: AccessReport) => void }) {
  const [running, setRunning] = useState(false);
  const [report, setReport] = useState<AccessReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    setRunning(true);
    setError(null);
    try {
      const r = await api.checkAudioAccess();
      setReport(r);
      onResult?.(r);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setRunning(false);
    }
  }

  const failed = report && !(report.microphone.heard && report.system_audio.heard);
  return (
    <div className="access-check">
      <div className="toolbar">
        <button type="button" className={report ? "" : "primary"} disabled={running} onClick={run}>
          {report ? "Check again" : "Check access"}
        </button>
        {running && (
          <span className="muted">
            <span className="spinner" /> Listening — say a few words…
          </span>
        )}
      </div>
      {report && (
        <ul className="access-list">
          <Row
            label="Microphone"
            track={report.microphone}
            missing="Nothing came through, so Quill isn't allowed to use the microphone."
            pane="microphone"
          />
          <Row
            label="System audio"
            track={report.system_audio}
            missing={
              isMac
                ? "Quill didn't hear its own test chime, so it isn't allowed to record system audio."
                : "Quill didn't hear its own test chime. Check that your speakers or headphones are the default output device."
            }
            pane={isMac ? "system_audio" : undefined}
          />
        </ul>
      )}
      {failed && isMac && (
        <p className="muted small">
          After allowing Quill in System Settings, quit Quill from the menu-bar icon, reopen it, and check again.
        </p>
      )}
      {error && <p className="error-text small">{error}</p>}
    </div>
  );
}

function Row({
  label,
  track,
  missing,
  pane,
}: {
  label: string;
  track: TrackAccess;
  missing: string;
  pane?: "microphone" | "system_audio";
}) {
  return (
    <li className={track.heard ? "ok" : "missing"}>
      <span className="mark" aria-hidden>
        {track.heard ? "✓" : "✕"}
      </span>
      <div>
        <strong>{label}</strong> <span className="muted small">{track.device}</span>
        <p className="small">
          {track.heard ? "Working." : missing}
          {!track.heard && pane && (
            <>
              {" "}
              <button type="button" className="link" onClick={() => void api.openPrivacySettings(pane)}>
                Open {isMac ? "System Settings" : "Settings"}
              </button>
            </>
          )}
        </p>
      </div>
    </li>
  );
}
