import { useEffect, useState } from "react";
import { api } from "../api";
import { Icon } from "./Icon";

/**
 * App icons, looked up once per app and shared: every AppIcon asking in the
 * same moment goes in one request.
 */
const icons = new Map<string, string | null>();
const waiting = new Map<string, Set<() => void>>();
let queued: string[] = [];

function request(id: string, done: () => void) {
  const listeners = waiting.get(id);
  if (listeners) {
    listeners.add(done);
    return;
  }
  waiting.set(id, new Set([done]));
  queued.push(id);
  if (queued.length === 1) {
    queueMicrotask(() => {
      const ids = queued;
      queued = [];
      api
        .appIcons(ids)
        .catch(() => ({}) as Record<string, string | null>)
        .then((found) => {
          for (const id of ids) {
            icons.set(id, found[id] ?? null);
            for (const notify of waiting.get(id) ?? []) notify();
            waiting.delete(id);
          }
        });
    });
  }
}

/**
 * The app's own icon, or its first letter until (or unless) there is one.
 * Decorative: the app's name is always written beside it.
 */
export function AppIcon({ id, name }: { id: string | null | undefined; name: string | null | undefined }) {
  const [, rerender] = useState(0);
  const icon = id ? icons.get(id) : null;
  useEffect(() => {
    if (id && !icons.has(id)) request(id, () => rerender((n) => n + 1));
  }, [id]);

  if (icon) {
    return (
      <span className="app-badge has-icon" aria-hidden>
        <img src={icon} alt="" draggable={false} />
      </span>
    );
  }
  return (
    <span className="app-badge" aria-hidden>
      {name ? name.trim().charAt(0).toUpperCase() : <Icon name="mic" size={13} />}
    </span>
  );
}
