import { useEffect, useState } from "react";
import { api, errorText, type Insights } from "../api";
import { Icon, Logo, type IconName } from "./Icon";

const number = new Intl.NumberFormat();

export function InsightsView() {
  const [insights, setInsights] = useState<Insights | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chart, setChart] = useState<"dictation" | "meetings">("dictation");
  const [confirmReset, setConfirmReset] = useState(false);

  const load = () =>
    api
      .getInsights()
      .then(setInsights)
      .catch((e) => setError(errorText(e)));
  useEffect(() => {
    void load();
  }, []);

  if (error) return <div className="page error-text">{error}</div>;
  if (!insights) return null;
  const { dictation: d, meetings: m } = insights;

  if (d.count === 0 && m.count === 0) {
    return (
      <div className="empty">
        <Logo size={64} />
        <h2>Nothing to show yet</h2>
        <p>Dictate something or record a meeting, and your usage shows up here.</p>
      </div>
    );
  }

  const speedup = d.speaking_wpm ? d.speaking_wpm / insights.typing_wpm : null;

  return (
    <div className="page insights">
      <div className="page-head">
        <div>
          <h1 className="page-title">Insights</h1>
          <p className="page-subtitle">How you've been using Quill.</p>
        </div>
      </div>

      <div className="stat-grid">
        <Stat
          icon="text"
          label="Words dictated"
          value={number.format(d.words)}
          note={d.words_last_7_days > 0 ? `${number.format(d.words_last_7_days)} in the last 7 days` : "None this week"}
        />
        <Stat icon="clock" label="Time saved" value={duration(d.minutes_saved * 60_000)} note="Compared with typing" accent />
        <Stat
          icon="bolt"
          label="Speaking speed"
          value={d.speaking_wpm ? `${Math.round(d.speaking_wpm)} wpm` : "–"}
          note={speedup ? `${speedup.toFixed(1)}× faster than typing` : "After a minute of dictation"}
        />
        <Stat
          icon="flame"
          label="Streak"
          value={`${insights.streak_days} ${insights.streak_days === 1 ? "day" : "days"}`}
          note={insights.streak_days > 0 ? "In a row, and counting" : "Use Quill today to start one"}
        />
      </div>

      <section className="group">
        <div className="row-gap" style={{ marginBottom: 8 }}>
          <h2 className="group-title" style={{ margin: "0 0 0 2px" }}>
            Last 30 days
          </h2>
          <span className="spacer" />
          <div className="segmented" role="tablist">
            <button role="tab" aria-selected={chart === "dictation"} onClick={() => setChart("dictation")}>
              Words dictated
            </button>
            <button role="tab" aria-selected={chart === "meetings"} onClick={() => setChart("meetings")}>
              Meeting time
            </button>
          </div>
        </div>
        <div className="card chart-card">
          <ActivityChart days={insights.days} kind={chart} />
        </div>
      </section>

      <div className="insight-columns">
        <section className="group">
          <h2 className="group-title">Meetings</h2>
          <div className="card insight-card">
            <div className="mini-stats">
              <Mini label="Recorded" value={number.format(m.count)} />
              <Mini label="Total time" value={duration(m.total_ms)} />
              <Mini label="Average" value={m.average_ms ? duration(m.average_ms) : "–"} />
              <Mini label="This week" value={number.format(m.count_last_7_days)} />
            </div>
            {m.my_talk_share != null && <TalkShare share={m.my_talk_share} />}
            <p className="faint small">
              {m.summaries} summaries written
              {m.summary_cost_usd > 0 && ` · $${m.summary_cost_usd.toFixed(m.summary_cost_usd < 1 ? 4 : 2)} in model costs`}
            </p>
          </div>
        </section>

        <section className="group">
          <h2 className="group-title">Dictation</h2>
          <div className="card insight-card">
            <div className="mini-stats">
              <Mini label="Dictations" value={number.format(d.count)} />
              <Mini
                label="Typical wait"
                value={d.median_latency_ms != null ? `${(d.median_latency_ms / 1000).toFixed(1)} s` : "–"}
              />
              <Mini label="Tidied up" value={`${Math.round(d.polished_share * 100)}%`} />
            </div>
            <div>
              <p className="mini-label" style={{ marginBottom: 6 }}>
                When you dictate
              </p>
              <HourChart hours={insights.hours} />
            </div>
          </div>
        </section>
      </div>

      {insights.apps.length > 0 && (
        <section className="group">
          <h2 className="group-title">Top apps</h2>
          <div className="card insight-card">
            <TopApps apps={insights.apps} />
          </div>
        </section>
      )}

      <footer className="insights-footer">
        <p className="faint small">
          Time saved compares speaking with typing at {insights.typing_wpm} words per minute. Everything here is
          counted on this computer; it keeps numbers, not what you said.
        </p>
        {d.count > 0 &&
          (confirmReset ? (
            <div className="confirm">
              Reset dictation numbers?
              <button
                className="danger"
                onClick={async () => {
                  await api.resetDictationStats();
                  setConfirmReset(false);
                  await load();
                }}
              >
                Reset
              </button>
              <button onClick={() => setConfirmReset(false)}>Cancel</button>
            </div>
          ) : (
            <button className="ghost" onClick={() => setConfirmReset(true)}>
              Reset
            </button>
          ))}
      </footer>
    </div>
  );
}

function Stat({
  icon,
  label,
  value,
  note,
  accent,
}: {
  icon: IconName;
  label: string;
  value: string;
  note: string;
  accent?: boolean;
}) {
  return (
    <div className={`stat card${accent ? " accent" : ""}`}>
      <div className="stat-label">
        <Icon name={icon} size={14} /> {label}
      </div>
      <div className="stat-value">{value}</div>
      <div className="stat-note">{note}</div>
    </div>
  );
}

function Mini({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="mini-label">{label}</p>
      <p className="mini-value">{value}</p>
    </div>
  );
}

function ActivityChart({ days, kind }: { days: Insights["days"]; kind: "dictation" | "meetings" }) {
  const values = days.map((day) => (kind === "dictation" ? day.words : day.meeting_ms / 60_000));
  const max = Math.max(...values, 1);
  const label = (v: number) => (kind === "dictation" ? `${number.format(v)} words` : duration(v * 60_000));
  const total = values.reduce((a, b) => a + b, 0);
  return (
    <div className="activity">
      <div className="activity-total">
        <span className="stat-value">{label(total)}</span>
        <span className="faint small">in the last 30 days</span>
      </div>
      <div className="bars-chart" role="img" aria-label={`${kind} per day for the last 30 days`}>
        {days.map((day, i) => (
          <div key={day.date} className="bar-col" title={`${dayLabel(day.date)}: ${label(values[i])}`}>
            <div
              className={`bar ${kind}${values[i] === 0 ? " zero" : ""}`}
              style={{ height: `${Math.max(values[i] / max, values[i] > 0 ? 0.04 : 0) * 100}%` }}
            />
          </div>
        ))}
      </div>
      <div className="axis">
        <span>{dayLabel(days[0]?.date)}</span>
        <span>{dayLabel(days[Math.floor(days.length / 2)]?.date)}</span>
        <span>Today</span>
      </div>
    </div>
  );
}

function HourChart({ hours }: { hours: number[] }) {
  const max = Math.max(...hours, 1);
  return (
    <div className="hours">
      <div className="hours-bars">
        {hours.map((count, h) => (
          <div key={h} className="hour" title={`${hourLabel(h)}: ${count} dictations`}>
            <div style={{ height: `${(count / max) * 100}%` }} />
          </div>
        ))}
      </div>
      <div className="axis">
        <span>12 AM</span>
        <span>6 AM</span>
        <span>12 PM</span>
        <span>6 PM</span>
        <span>11 PM</span>
      </div>
    </div>
  );
}

function TopApps({ apps }: { apps: Insights["apps"] }) {
  const max = Math.max(...apps.map((a) => a.words), 1);
  return (
    <ul className="top-apps">
      {apps.map((app) => (
        <li key={app.app_id}>
          <span className="app-badge">{app.name.slice(0, 1).toUpperCase()}</span>
          <span className="app-name">{app.name}</span>
          <div className="app-bar">
            <div style={{ width: `${(app.words / max) * 100}%` }} />
          </div>
          <span className="app-words">{number.format(app.words)} words</span>
        </li>
      ))}
    </ul>
  );
}

function TalkShare({ share }: { share: number }) {
  const me = Math.round(share * 100);
  return (
    <div className="talk-share">
      <div className="row-gap small">
        <span className="legend me">You {me}%</span>
        <span className="spacer" />
        <span className="legend others">Others {100 - me}%</span>
      </div>
      <div className="talk-bar">
        <div className="me" style={{ width: `${me}%` }} />
        <div className="others" style={{ width: `${100 - me}%` }} />
      </div>
      <p className="faint small">Share of the words spoken in your transcribed meetings.</p>
    </div>
  );
}

/** "27.5 h", "3 h 12 min", "45 min", "<1 min". */
function duration(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return ms > 0 ? "<1 min" : "0 min";
  if (minutes < 60) return `${minutes} min`;
  if (minutes >= 600) return `${number.format(Math.round(minutes / 6) / 10)} h`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? `${h} h ${m} min` : `${h} h`;
}

function dayLabel(date?: string): string {
  if (!date) return "";
  const [y, mo, d] = date.split("-").map(Number);
  return new Date(y, mo - 1, d).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function hourLabel(h: number): string {
  return new Date(2000, 0, 1, h).toLocaleTimeString(undefined, { hour: "numeric" });
}
