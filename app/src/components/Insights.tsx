// Usage numbers, shown on Home below the meetings.
import { useState } from "react";
import { api, type Insights } from "../api";
import { AppIcon } from "./AppIcon";

const number = new Intl.NumberFormat();

/** All-time dictation numbers and the streak, as one line of facts. */
export function UsageFacts({ insights }: { insights: Insights }) {
  const d = insights.dictation;
  const facts: [string, string][] = [];
  if (d.words > 0) facts.push([number.format(d.words), "words dictated"]);
  if (d.minutes_saved > 0) facts.push([duration(d.minutes_saved * 60_000), "saved over typing"]);
  if (d.speaking_wpm) facts.push([`${Math.round(d.speaking_wpm)} wpm`, "when you speak"]);
  if (insights.streak_days > 1) facts.push([`${insights.streak_days} days`, "in a row"]);
  if (facts.length === 0) return null;
  return (
    <p className="usage-facts">
      {facts.map(([value, label]) => (
        <span key={label}>
          <strong>{value}</strong> {label}
        </span>
      ))}
    </p>
  );
}

/**
 * The rest of the usage numbers: the last 30 days, meetings and dictation
 * in detail, top apps, and resetting the dictation numbers.
 */
export function InsightDetails({ insights, onReset }: { insights: Insights; onReset: () => void }) {
  const [chart, setChart] = useState<"dictation" | "meetings">("dictation");
  const [confirmReset, setConfirmReset] = useState(false);
  const { dictation: d, meetings: m } = insights;

  return (
    <>
      <section className="group">
        <div className="row-gap" style={{ marginBottom: 8 }}>
          <h2 className="group-title" style={{ margin: 0 }}>
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
                value={
                  // After you stop speaking, once measured; before that, after the key's release.
                  d.wait
                    ? seconds(d.wait.median_ms)
                    : d.median_latency_ms != null
                      ? seconds(d.median_latency_ms)
                      : "–"
                }
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
                  onReset();
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
    </>
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

/**
 * Round steps for a chart's vertical axis: 0 and up to three more, ending
 * at or just above `max`. Words in 1/2/2.5/5 × 10ⁿ; minutes in a quarter
 * hour, half hour and whole hours.
 */
function axisTicks(max: number, kind: "dictation" | "meetings"): number[] {
  const steps =
    kind === "meetings"
      ? [5, 10, 15, 30, 60, 120, 180, 240, 360, 480, 720]
      : [1, 2, 2.5, 5].flatMap((m) => [1, 10, 100, 1_000, 10_000, 100_000].map((p) => m * p)).sort((a, b) => a - b);
  const step = steps.find((s) => s * 3 >= max) ?? steps[steps.length - 1];
  const count = Math.max(1, Math.ceil(max / step));
  return Array.from({ length: count + 1 }, (_, i) => i * step);
}

/** "1.2k", "500", "2 h", "45 min": short, for an axis. */
function axisLabel(v: number, kind: "dictation" | "meetings"): string {
  if (kind === "meetings") return v === 0 ? "0" : v >= 60 ? `${+(v / 60).toFixed(1)} h` : `${v} min`;
  return v >= 1000 ? `${+(v / 1000).toFixed(1)}k` : String(v);
}

function ActivityChart({ days, kind }: { days: Insights["days"]; kind: "dictation" | "meetings" }) {
  const [hover, setHover] = useState<number | null>(null);
  const values = days.map((day) => (kind === "dictation" ? day.words : day.meeting_ms / 60_000));
  const max = Math.max(...values, 1);
  const ticks = axisTicks(max, kind);
  const top = ticks[ticks.length - 1];
  const label = (v: number) => (kind === "dictation" ? `${number.format(v)} words` : duration(v * 60_000));
  const total = values.reduce((a, b) => a + b, 0);
  const n = days.length;
  const shown = hover !== null ? days[hover] : null;
  return (
    <div className="activity">
      <div className="activity-total">
        <span className="stat-value">{label(total)}</span>
        <span className="faint small">in the last 30 days</span>
      </div>
      <div className="chart-plot">
        <div className="chart-y" aria-hidden>
          {ticks.map((t) => (
            <span key={t} style={{ bottom: `${(t / top) * 100}%` }}>
              {axisLabel(t, kind)}
            </span>
          ))}
        </div>
        <div className="chart-area">
          {ticks.slice(1).map((t) => (
            <span key={t} className="chart-grid" style={{ bottom: `${(t / top) * 100}%` }} aria-hidden />
          ))}
          <div
            className="bars-chart"
            role="img"
            aria-label={`${kind === "dictation" ? "Words dictated" : "Meeting time"} per day for the last 30 days`}
            onMouseLeave={() => setHover(null)}
          >
            {days.map((day, i) => (
              <div
                key={day.date}
                className={`bar-col${hover === i ? " hovered" : ""}`}
                onMouseEnter={() => setHover(i)}
              >
                <div
                  className={`bar ${kind}${values[i] === 0 ? " zero" : ""}`}
                  style={{ height: `${Math.max(values[i] / top, values[i] > 0 ? 0.03 : 0) * 100}%` }}
                />
              </div>
            ))}
          </div>
          {shown && hover !== null && (
            <div
              className={`chart-tip${hover < 4 ? " start" : hover > n - 5 ? " end" : ""}`}
              style={{
                left: `${((hover + 0.5) / n) * 100}%`,
                bottom: `calc(${Math.min(1, values[hover] / top) * 100}% + 8px)`,
              }}
            >
              <span className="chart-tip-day">{hover === n - 1 ? "Today" : weekdayLabel(shown.date)}</span>
              <strong>{values[hover] === 0 ? (kind === "dictation" ? "No dictation" : "No meetings") : label(values[hover])}</strong>
              {kind === "dictation" && shown.dictations > 0 && (
                <span className="chart-tip-sub">
                  {shown.dictations} {shown.dictations === 1 ? "dictation" : "dictations"}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
      <div className="axis chart-x">
        <span>{dayLabel(days[0]?.date)}</span>
        <span>{dayLabel(days[Math.floor(days.length / 2)]?.date)}</span>
        <span>Today</span>
      </div>
    </div>
  );
}

/** "0.7 s", "1.2 s". */
export function seconds(ms: number): string {
  return `${(ms / 1000).toFixed(1)} s`;
}

/**
 * Time to insert: from when you stop speaking to the text appearing, the
 * wait you notice. The last week's median against the week before, how it
 * splits between holding the key and Quill, and 30 days of it, day by day.
 */
export function TimeToInsert({ insights }: { insights: Insights }) {
  const w = insights.dictation.wait;
  if (!w) {
    return (
      <section className="card wait-panel">
        <h2 className="wait-title">Time to insert</h2>
        <p className="wait-note">
          How long you wait from when you stop speaking to the text appearing. Measured from your next dictation.
        </p>
      </section>
    );
  }
  const week = w.last_7_days_ms ?? w.median_ms;
  const change =
    w.last_7_days_ms !== null && w.previous_7_days_ms !== null ? w.last_7_days_ms - w.previous_7_days_ms : null;
  return (
    <section className="card wait-panel">
      <div className="wait-head">
        <div>
          <h2 className="wait-title">Time to insert</h2>
          <p className="wait-note">From when you stop speaking to the text appearing: the wait you notice.</p>
        </div>
        <div className="wait-figure">
          <strong>{seconds(week)}</strong>
          <span>{w.last_7_days_ms !== null ? "median, last 7 days" : "median, last 30 days"}</span>
          {change !== null && Math.abs(change) >= 50 && (
            <span className={`wait-trend ${change < 0 ? "better" : "worse"}`}>
              {seconds(Math.abs(change))} {change < 0 ? "quicker" : "slower"} than the week before
            </span>
          )}
        </div>
      </div>
      <p className="wait-split">
        Typically <strong>{seconds(w.held_ms)}</strong> still holding the key after you stop, then{" "}
        <strong>{seconds(w.quill_ms)}</strong> for Quill to transcribe and type. The slowest 1 in 10 take{" "}
        <strong>{seconds(w.p90_ms)}</strong> or more.
      </p>
      <WaitChart days={insights.days} />
    </section>
  );
}

/** Steps for a seconds axis: three or four round steps up to `max` ms. */
function waitTicks(max: number): number[] {
  const step = [250, 500, 1_000, 2_000, 5_000, 10_000].find((s) => s * 3 >= max) ?? 10_000;
  const count = Math.max(1, Math.ceil(max / step));
  return Array.from({ length: count + 1 }, (_, i) => i * step);
}

/** Each day's median time to insert as a column, with its 90th percentile as a tick above. */
function WaitChart({ days }: { days: Insights["days"] }) {
  const [hover, setHover] = useState<number | null>(null);
  const max = Math.max(...days.map((d) => d.wait_p90_ms ?? d.wait_ms ?? 0), 500);
  const ticks = waitTicks(max);
  const top = ticks[ticks.length - 1];
  const n = days.length;
  const shown = hover !== null ? days[hover] : null;
  const pct = (ms: number) => `${Math.min(1, ms / top) * 100}%`;
  return (
    <div className="activity wait-chart">
      <div className="chart-plot">
        <div className="chart-y" aria-hidden>
          {ticks.map((t) => (
            <span key={t} style={{ bottom: pct(t) }}>
              {t === 0 ? "0" : `${+(t / 1000).toFixed(2)} s`}
            </span>
          ))}
        </div>
        <div className="chart-area">
          {ticks.slice(1).map((t) => (
            <span key={t} className="chart-grid" style={{ bottom: pct(t) }} aria-hidden />
          ))}
          <div
            className="bars-chart"
            role="img"
            aria-label="Median time to insert per day for the last 30 days, with the slowest 1 in 10 marked above"
            onMouseLeave={() => setHover(null)}
          >
            {days.map((day, i) => (
              <div
                key={day.date}
                className={`bar-col${hover === i ? " hovered" : ""}`}
                onMouseEnter={() => setHover(i)}
              >
                {day.wait_ms !== null && <div className="bar wait" style={{ height: pct(day.wait_ms) }} />}
                {day.wait_p90_ms !== null && day.wait_p90_ms > (day.wait_ms ?? 0) && (
                  <span className="wait-p90" style={{ bottom: pct(day.wait_p90_ms) }} />
                )}
              </div>
            ))}
          </div>
          {shown && hover !== null && (
            <div
              className={`chart-tip${hover < 4 ? " start" : hover > n - 5 ? " end" : ""}`}
              style={{
                left: `${((hover + 0.5) / n) * 100}%`,
                bottom: `calc(${pct(shown.wait_p90_ms ?? shown.wait_ms ?? 0)} + 8px)`,
              }}
            >
              <span className="chart-tip-day">{hover === n - 1 ? "Today" : weekdayLabel(shown.date)}</span>
              <strong>{shown.wait_ms !== null ? `${seconds(shown.wait_ms)} median` : "Not measured"}</strong>
              {shown.wait_ms !== null && (
                <span className="chart-tip-sub">
                  {shown.wait_p90_ms !== null && `Slowest 1 in 10: ${seconds(shown.wait_p90_ms)} · `}
                  {shown.dictations} {shown.dictations === 1 ? "dictation" : "dictations"}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
      <div className="axis chart-x">
        <span>{dayLabel(days[0]?.date)}</span>
        <span>{dayLabel(days[Math.floor(days.length / 2)]?.date)}</span>
        <span>Today</span>
      </div>
      <div className="wait-legend" aria-hidden>
        <span>
          <i className="wait-key median" /> Median
        </span>
        <span>
          <i className="wait-key p90" /> Slowest 1 in 10
        </span>
      </div>
    </div>
  );
}

function HourChart({ hours }: { hours: number[] }) {
  const [hover, setHover] = useState<number | null>(null);
  const max = Math.max(...hours, 1);
  return (
    <div className="hours">
      <div className="hours-bars" onMouseLeave={() => setHover(null)}>
        {hours.map((count, h) => (
          <div key={h} className={`hour${hover === h ? " hovered" : ""}`} onMouseEnter={() => setHover(h)}>
            <div style={{ height: `${(count / max) * 100}%` }} />
          </div>
        ))}
        {hover !== null && (
          <div
            className={`chart-tip${hover < 3 ? " start" : hover > 20 ? " end" : ""}`}
            style={{ left: `${((hover + 0.5) / 24) * 100}%`, bottom: "calc(100% + 6px)" }}
          >
            <span className="chart-tip-day">
              {hourLabel(hover)} – {hourLabel((hover + 1) % 24)}
            </span>
            <strong>
              {hours[hover]} {hours[hover] === 1 ? "dictation" : "dictations"}
            </strong>
          </div>
        )}
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
          <AppIcon id={app.app_id} name={app.name} />
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
export function duration(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return ms > 0 ? "<1 min" : "0 min";
  if (minutes < 60) return `${minutes} min`;
  if (minutes >= 600) return `${number.format(Math.round(minutes / 6) / 10)} h`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? `${h} h ${m} min` : `${h} h`;
}

/** "Tue, Oct 14". */
function weekdayLabel(date: string): string {
  const [y, mo, d] = date.split("-").map(Number);
  return new Date(y, mo - 1, d).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

function dayLabel(date?: string): string {
  if (!date) return "";
  const [y, mo, d] = date.split("-").map(Number);
  return new Date(y, mo - 1, d).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function hourLabel(h: number): string {
  return new Date(2000, 0, 1, h).toLocaleTimeString(undefined, { hour: "numeric" });
}
