// Updates: the Home banner, the Settings rows and the line in About.
// Rust checks and installs (updates.rs); these only show where it stands.
import { useEffect, useState } from "react";
import { api, errorText, events, type Settings, type UpdateInfo } from "../api";
import { Banner, Row, Switch } from "./ui";

/** The update state, kept current by `update-status` events. */
export function useUpdate(): UpdateInfo | null {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  useEffect(() => {
    void api.updateInfo().then(setInfo);
    const unlisten = events.onUpdate(setInfo);
    return () => void unlisten.then((f) => f());
  }, []);
  return info;
}

/** Install and restart, or a check, with the error from either. */
function useUpdateActions() {
  const [error, setError] = useState<string | null>(null);
  const run = (action: () => Promise<unknown>) => {
    setError(null);
    action().catch((e) => setError(errorText(e)));
  };
  return {
    error,
    install: () => run(api.installUpdate),
    check: () => run(api.checkForUpdates),
  };
}

function percent(done: number, total: number | null): number | null {
  return total ? Math.min(100, Math.round((done / total) * 100)) : null;
}

/** "today at 9:14 AM", or "Oct 12 at 9:14 AM". */
function checkedWhen(ms: number): string {
  const d = new Date(ms);
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const today = d.toDateString() === new Date().toDateString();
  const day = today ? "today" : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return `${day} at ${time}`;
}

/** One line on where updating stands. */
function describe(info: UpdateInfo): string {
  const s = info.status;
  switch (s.state) {
    case "checking":
      return "Checking for updates…";
    case "available":
      return `Quill ${s.version} is available.`;
    case "downloading": {
      const p = percent(s.done, s.total);
      return `Downloading Quill ${s.version}${p === null ? "…" : ` · ${p}%`}`;
    }
    case "installing":
      return `Installing Quill ${s.version}. Quill will restart.`;
    case "up_to_date":
      return info.checked_at ? `Up to date · checked ${checkedWhen(info.checked_at)}` : "Up to date";
    case "idle":
      return "Not checked yet";
  }
}

/** On Home, once an update is found. */
export function UpdateBanner() {
  const info = useUpdate();
  const { error, install } = useUpdateActions();
  const s = info?.status;
  if (!info || !s || (s.state !== "available" && s.state !== "downloading" && s.state !== "installing")) return null;
  const p = s.state === "downloading" ? percent(s.done, s.total) : null;
  return (
    <Banner
      kind={error ? "warning" : "info"}
      icon="download"
      action={s.state === "available" ? <button onClick={install}>Restart to update</button> : undefined}
    >
      {describe(info)}
      {s.state === "available" && " Recordings and settings stay as they are."}
      {s.state === "downloading" && p !== null && (
        <div className="progress">
          <div style={{ width: `${p}%` }} />
        </div>
      )}
      {error && <div className="small">{error}</div>}
    </Banner>
  );
}

/** Settings → Updates. */
export function UpdateRows({ settings, update }: { settings: Settings; update: (patch: Partial<Settings>) => void }) {
  const info = useUpdate();
  const { error, install, check } = useUpdateActions();
  const state = info?.status.state;
  return (
    <>
      <Row
        label="Check for updates automatically"
        hint="Once a day, Quill asks GitHub whether there's a new version. Nothing about you or your meetings is sent, and nothing installs until you choose."
      >
        <Switch
          label="Check for updates automatically"
          checked={settings.check_for_updates}
          onChange={(check_for_updates) => update({ check_for_updates })}
        />
      </Row>
      <Row
        label={info ? `Quill ${info.current_version}` : "Quill"}
        hint={
          <>
            {info && describe(info)}
            {error && <span className="error-text"> {error}</span>}
          </>
        }
      >
        {state === "available" ? (
          <button className="primary" onClick={install}>
            Restart to update
          </button>
        ) : (
          <button onClick={check} disabled={!info || (state !== "idle" && state !== "up_to_date")}>
            Check now
          </button>
        )}
      </Row>
    </>
  );
}

/** The line under the version in the About window. */
export function AboutUpdate() {
  const info = useUpdate();
  const { error, install, check } = useUpdateActions();
  if (!info) return null;
  const state = info.status.state;
  return (
    <p className="about-update">
      {error ? <span className="error-text">{error}</span> : describe(info)}{" "}
      {state === "available" ? (
        <button className="link" onClick={install}>
          Restart to update
        </button>
      ) : (
        (state === "idle" || state === "up_to_date") && (
          <button className="link" onClick={check}>
            Check for updates
          </button>
        )
      )}
    </p>
  );
}
