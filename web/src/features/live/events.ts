import type { WebEvent } from "../../protocol.ts";

/**
 * Every `WebEvent` the page's one `/ws/web` socket receives, for a view that refetches its own
 * resource on an `updated` event (plan 122). The shell still decides alone when the overview is
 * refetched (`concernsUs`).
 */
const listeners = new Set<(ev: WebEvent) => void>();

/** Hear every event from `/ws/web`. Returns a function that stops listening. */
export function onWebEvent(cb: (ev: WebEvent) => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Hands one event to every listener; one that throws does not keep it from the others. Called by `subscribe`. */
export function emitWebEvent(ev: WebEvent): void {
  for (const cb of [...listeners]) {
    try {
      cb(ev);
    } catch (e) {
      console.error("a /ws/web listener failed", e);
    }
  }
}
