import { useState } from "react";
import { api, errorText, isMac, type AccessReport, type TrackAccess } from "../api";
import { Icon } from "./Icon";

/**
 * The microphone and system-audio rows with a "Check access" button, as a
 * card. Used in onboarding and Settings.
 */
export function AccessCheck({ onResult, framed = true }: { onResult?: (report: AccessReport) => void; framed?: boolean }) {
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
    <div className={framed ? "card" : undefined}>
      <ul className="access-list">
        <Row
          icon="mic"
          label="Microphone"
          what="Your voice, labelled “Me”"
          track={report?.microphone}
          running={running}
          missing="Nothing came through, so Quill isn't allowed to use the microphone."
          pane="microphone"
        />
        <Row
          icon="speaker"
          label="System audio"
          what="The call, labelled “Others”"
          track={report?.system_audio}
          running={running}
          missing={
            isMac
              ? "Quill didn't hear its own test chime, so it isn't allowed to record system audio."
              : "Quill didn't hear its own test chime. Check your default output device."
          }
          pane={isMac ? "system_audio" : undefined}
        />
      </ul>
      <div className="access-footer">
        <span className="muted small" style={{ flex: 1 }}>
          {error ? (
            <span className="error-text">{error}</span>
          ) : running ? (
            "Listening for a few seconds and playing a short chime. Say a few words."
          ) : failed && isMac ? (
            "After allowing Quill in System Settings, quit it from the menu bar, reopen it, and check again."
          ) : report ? (
            "Everything Quill needs is working."
          ) : (
            "Records a few seconds and plays a short chime to confirm Quill hears both."
          )}
        </span>
        <button className={report ? "" : "primary"} disabled={running} onClick={run}>
          {running ? (
            <>
              <span className="spinner" /> Checking…
            </>
          ) : report ? (
            "Check again"
          ) : (
            "Check access"
          )}
        </button>
      </div>
    </div>
  );
}

function Row({
  icon,
  label,
  what,
  track,
  running,
  missing,
  pane,
}: {
  icon: "mic" | "speaker";
  label: string;
  what: string;
  track?: TrackAccess;
  running: boolean;
  missing: string;
  pane?: "microphone" | "system_audio";
}) {
  return (
    <li className={`access-item${track ? (track.heard ? " ok" : " missing") : ""}`}>
      <div className="access-icon">
        <Icon name={icon} size={17} />
      </div>
      <div className="access-body">
        <strong>{label}</strong>
        <p>
          {track && !track.heard ? (
            <>
              {missing}{" "}
              {pane && (
                <button className="link" onClick={() => void api.openPrivacySettings(pane)}>
                  Open {isMac ? "System Settings" : "Settings"}
                </button>
              )}
            </>
          ) : (
            <>
              {what}
              {track && ` · ${track.device}`}
            </>
          )}
        </p>
      </div>
      {running ? (
        <span className="pill accent">
          <span className="spinner" style={{ width: 10, height: 10 }} /> Listening
        </span>
      ) : track ? (
        track.heard ? (
          <span className="pill green">
            <Icon name="check" size={12} /> Working
          </span>
        ) : (
          <span className="pill red">
            <Icon name="x" size={12} /> No access
          </span>
        )
      ) : (
        <span className="pill">Not checked</span>
      )}
    </li>
  );
}
