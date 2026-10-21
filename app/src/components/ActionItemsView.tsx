import { useEffect, useRef, useState } from "react";
import { type ActionItem, errorText } from "../api";
import { byUrgency, dueLabel, dueState, setActionItemDone, today, useActionItems } from "../actionItems";
import { Icon, type IconName } from "./Icon";
import { useCountUp } from "./ui";

/** How long a ticked item stays in the open list, struck through, before it goes. */
const SETTLE_MS = 1400;

/**
 * Action items from every meeting's notes, to tick off: yours first, most
 * pressing at the top, then what others owe you, by person. Ticking one here
 * ticks it in its meeting's notes too.
 */
export function ActionItemsView({ onOpenMeeting }: { onOpenMeeting: (id: string) => void }) {
  const items = useActionItems();
  const [show, setShow] = useState<"open" | "done">("open");
  const settling = useSettling();
  const [error, setError] = useState<string | null>(null);

  if (!items) return null;
  const now = today();
  const open = items.filter((i) => !i.done_at || settling.has(i.id));
  const done = items
    .filter((i) => i.done_at)
    .sort((a, b) => (b.done_at ?? "").localeCompare(a.done_at ?? ""));
  const mine = open.filter((i) => i.mine).sort(byUrgency);
  const theirs = open.filter((i) => !i.mine).sort(byUrgency);
  const people = groupBy(theirs, (i) => i.owner ?? "No one named");

  const toggle = (item: ActionItem) => {
    const done = !item.done_at;
    if (done) settling.add(item.id);
    setError(null);
    setActionItemDone(item.id, done).catch((e) => setError(errorText(e)));
  };
  const row = (item: ActionItem) => (
    <ActionRow key={item.id} item={item} now={now} onToggle={() => toggle(item)} onOpenMeeting={onOpenMeeting} />
  );

  const openMine = items.filter((i) => i.mine && !i.done_at);
  return (
    <div className="page action-items">
      <header className="page-head">
        <div>
          <h1 className="page-title">Action items</h1>
          <p className="page-subtitle">
            {items.length === 0
              ? "What your meetings agreed needs doing, gathered from their notes."
              : `${openMine.length} of yours open · ${items.filter((i) => !i.mine && !i.done_at).length} waiting on others`}
          </p>
        </div>
      </header>

      {items.length === 0 ? (
        <NoActionItems />
      ) : (
        <>
          <Numbers items={items} now={now} />
          <div className="kind-filter" role="group" aria-label="Show">
            {(["open", "done"] as const).map((kind) => (
              <button
                key={kind}
                className={`kind-chip${show === kind ? " on" : ""}`}
                aria-pressed={show === kind}
                onClick={() => setShow(kind)}
              >
                {kind === "open" ? "Open" : "Done"}{" "}
                <span className="kind-count">{kind === "open" ? items.filter((i) => !i.done_at).length : done.length}</span>
              </button>
            ))}
          </div>
          {error && <p className="error-text small">{error}</p>}

          {show === "open" ? (
            open.length === 0 ? (
              <p className="muted filter-none">You're all caught up: everything from your meetings is done.</p>
            ) : (
              <>
                {mine.length > 0 && (
                  <section className="group">
                    <h2 className="group-title">Yours</h2>
                    <ul className="card action-list">{mine.map(row)}</ul>
                  </section>
                )}
                {people.length > 0 && (
                  <section className="group">
                    <h2 className="group-title">Waiting on others</h2>
                    {people.map(([person, list]) => (
                      <div className="action-person" key={person}>
                        <h3 className="action-person-name">
                          <span className="person-initial" aria-hidden>
                            {person.charAt(0)}
                          </span>
                          {person}
                        </h3>
                        <ul className="card action-list">{list.map(row)}</ul>
                      </div>
                    ))}
                  </section>
                )}
              </>
            )
          ) : done.length === 0 ? (
            <p className="muted filter-none">Nothing ticked off yet.</p>
          ) : (
            <section className="group">
              <h2 className="group-title">Done</h2>
              <ul className="card action-list">{done.map(row)}</ul>
            </section>
          )}
        </>
      )}
    </div>
  );
}

/** Ids just ticked off, kept in the open list for a moment so the tick is seen. */
function useSettling() {
  const [ids, setIds] = useState<Set<number>>(new Set());
  const timers = useRef<number[]>([]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);
  return {
    has: (id: number) => ids.has(id),
    add: (id: number) => {
      setIds((s) => new Set(s).add(id));
      timers.current.push(
        window.setTimeout(() => {
          setIds((s) => {
            const next = new Set(s);
            next.delete(id);
            return next;
          });
        }, SETTLE_MS),
      );
    },
  };
}

function groupBy<T>(list: T[], key: (t: T) => string): [string, T[]][] {
  const groups = new Map<string, T[]>();
  for (const item of list) groups.set(key(item), [...(groups.get(key(item)) ?? []), item]);
  // Whoever owes the most first.
  return [...groups].sort((a, b) => b[1].length - a[1].length || a[0].localeCompare(b[0]));
}

function Numbers({ items, now }: { items: ActionItem[]; now: string }) {
  const progress = useCountUp();
  const mine = items.filter((i) => i.mine && !i.done_at);
  const weekAgo = Date.now() - 7 * 86_400_000;
  const stats: { icon: IconName; value: number; label: string; tone?: string }[] = [
    { icon: "checklist", value: mine.length, label: "yours open" },
    {
      icon: "alert",
      value: mine.filter((i) => dueState(i, now) === "overdue").length,
      label: "overdue",
      tone: "overdue",
    },
    {
      icon: "clock",
      value: mine.filter((i) => dueState(i, now) === "today" || dueState(i, now) === "soon").length,
      label: "due in the next 3 days",
    },
    {
      icon: "check",
      value: items.filter((i) => i.done_at && Date.parse(i.done_at) >= weekAgo).length,
      label: "done this week",
    },
  ];
  return (
    <div className="stat-tiles action-numbers">
      {stats.map((s) => (
        <div className={`stat-tile${s.tone && s.value > 0 ? ` ${s.tone}` : ""}`} key={s.label}>
          <Icon name={s.icon} size={15} />
          <strong>{Math.round(s.value * progress)}</strong>
          <span>{s.label}</span>
        </div>
      ))}
    </div>
  );
}

function NoActionItems() {
  return (
    <div className="action-empty">
      <Icon name="checklist" size={22} />
      <p>
        When a meeting's notes are written, the things people agreed to do appear here, so they don't stay buried in
        the notes. Tick them off as they're done.
      </p>
    </div>
  );
}

/** One item: a box to tick, the task, when it's due and which meeting it came from. */
function ActionRow({
  item,
  now,
  onToggle,
  onOpenMeeting,
}: {
  item: ActionItem;
  now: string;
  onToggle: () => void;
  onOpenMeeting?: (id: string) => void;
}) {
  const state = dueState(item, now);
  const due = dueLabel(item, now);
  const started = new Date(item.meeting_started_at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return (
    <li className={`action-row${item.done_at ? " done" : ""}`}>
      <CheckBox item={item} onToggle={onToggle} />
      <div className="action-body">
        <span className="action-task">
          <TaskText text={item.task} />
        </span>
        <span className="item-meta">
          {due && (
            <span className={`due-chip${state ? ` ${state}` : ""}`} title={item.due_text ?? undefined}>
              {state === "overdue" ? `Overdue · ${due}` : due}
            </span>
          )}
          {item.done_at && (
            <span>
              Done {new Date(item.done_at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
            </span>
          )}
          {onOpenMeeting && (
            <button className="link-button action-meeting" onClick={() => onOpenMeeting(item.meeting_id)}>
              {item.meeting_title} · {started}
            </button>
          )}
        </span>
      </div>
    </li>
  );
}

function CheckBox({ item, onToggle }: { item: ActionItem; onToggle: () => void }) {
  const done = item.done_at !== null;
  return (
    <button
      className={`action-check${done ? " on" : ""}`}
      role="checkbox"
      aria-checked={done}
      aria-label={`${done ? "Mark not done" : "Mark done"}: ${item.task}`}
      onClick={onToggle}
    >
      <Icon name="check" size={12} />
    </button>
  );
}

/** A task as written in the notes, with its `code` spans shown as code. */
function TaskText({ text }: { text: string }) {
  return (
    <>
      {text.split(/(`[^`]+`)/).map((part, i) =>
        part.startsWith("`") && part.endsWith("`") && part.length > 2 ? <code key={i}>{part.slice(1, -1)}</code> : part,
      )}
    </>
  );
}

/**
 * A meeting's action items as a checklist, in place of the notes' plain
 * list: ticking here ticks them on the Action items page too.
 */
export function ActionChecklist({ items }: { items: ActionItem[] }) {
  const [error, setError] = useState<string | null>(null);
  const now = today();
  return (
    <>
      <ul className="action-checklist">
        {items.map((item) => {
          const state = dueState(item, now);
          const due = dueLabel(item, now);
          return (
            <li key={item.id} className={`action-row${item.done_at ? " done" : ""}`}>
              <CheckBox
                item={item}
                onToggle={() => {
                  setError(null);
                  setActionItemDone(item.id, !item.done_at).catch((e) => setError(errorText(e)));
                }}
              />
              <div className="action-body">
                <span className="action-task">
                  {item.owner && <strong>{item.owner}</strong>}
                  {item.owner && " — "}
                  <TaskText text={item.task} />
                </span>
                {due && (
                  <span className="item-meta">
                    <span className={`due-chip${state ? ` ${state}` : ""}`} title={item.due_text ?? undefined}>
                      {state === "overdue" ? `Overdue · ${due}` : due}
                    </span>
                  </span>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      {error && <p className="error-text small">{error}</p>}
    </>
  );
}
