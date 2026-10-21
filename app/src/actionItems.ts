import { useEffect, useState } from "react";
import { type ActionItem, api } from "./api";

/**
 * Every meeting's action items, shared by the sidebar's count, the Action
 * items page and the checkboxes in a meeting's notes, so a tick shows
 * everywhere at once.
 */
let items: ActionItem[] | null = null;
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((listener) => listener());

/** Reloads them, e.g. after a meeting's notes are written. */
export async function refreshActionItems() {
  try {
    items = await api.actionItems();
    notify();
  } catch {
    // Keep showing the last list.
  }
}

/** Ticks one off (or back on): on screen at once, then saved. */
export async function setActionItemDone(id: number, done: boolean) {
  const before = items;
  const update = (item: ActionItem) =>
    items?.map((i) => (i.id === id ? item : i)) ?? null;
  const current = items?.find((i) => i.id === id);
  if (current) {
    items = update({ ...current, done_at: done ? new Date().toISOString() : null });
    notify();
  }
  try {
    items = update(await api.setActionItemDone(id, done));
    notify();
  } catch (e) {
    items = before;
    notify();
    throw e;
  }
}

/** The action items (null until first loaded). */
export function useActionItems(): ActionItem[] | null {
  const [, rerender] = useState(0);
  useEffect(() => {
    const listener = () => rerender((n) => n + 1);
    listeners.add(listener);
    if (items === null) void refreshActionItems();
    return () => {
      listeners.delete(listener);
    };
  }, []);
  return items;
}

/** Today's date here, as YYYY-MM-DD. */
export function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

function addDays(date: string, days: number): string {
  const d = new Date(`${date}T12:00:00`);
  d.setDate(d.getDate() + days);
  return d.toLocaleDateString("en-CA");
}

/** How pressing an open item's due date is: past it, today, within three days, or later. */
export function dueState(item: ActionItem, now = today()): "overdue" | "today" | "soon" | "later" | null {
  if (!item.due_date || item.done_at) return null;
  if (item.due_date < now) return "overdue";
  if (item.due_date === now) return "today";
  return item.due_date <= addDays(now, 3) ? "soon" : "later";
}

/** "Today", "Tomorrow", "Thursday" (this week), "Oct 30"; or the words as written. */
export function dueLabel(item: ActionItem, now = today()): string | null {
  if (!item.due_date) return item.due_text;
  if (item.due_date === now) return "Today";
  if (item.due_date === addDays(now, 1)) return "Tomorrow";
  if (item.due_date === addDays(now, -1)) return "Yesterday";
  const date = new Date(`${item.due_date}T12:00:00`);
  if (item.due_date > now && item.due_date < addDays(now, 7)) {
    return date.toLocaleDateString(undefined, { weekday: "long" });
  }
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** Open items in the order to do them: overdue and soonest first, then those with no date, newest meeting first. */
export function byUrgency(a: ActionItem, b: ActionItem): number {
  if (a.due_date && b.due_date) return a.due_date.localeCompare(b.due_date);
  if (a.due_date) return -1;
  if (b.due_date) return 1;
  return b.meeting_started_at.localeCompare(a.meeting_started_at);
}
