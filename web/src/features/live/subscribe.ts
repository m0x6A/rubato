import type { WebEvent } from "../../protocol.ts";
import { ApiError, request } from "../../shared/api.ts";
import { currentTenant } from "../../shared/workspace.ts";
import { emitWebEvent } from "./events.ts";

/**
 * Subscribe to `/ws/web`. Calls `onChanged` for every `WebEvent`, and hands it to the views that
 * listen with `onWebEvent`; `onState` on connect and disconnect; reconnects with backoff. Returns
 * an unsubscribe function.
 *
 * A browser cannot see why an upgrade was refused, so every close probes `GET /api/me`: a 401
 * means the session needs a login (design S24), which `request` raises, and the retries stop.
 * Anything else (the session holds, or the server cannot be reached) is "disconnected": retry.
 *
 * `onRemoved`: the server closed the socket with 4403, because the person left the tenant.
 */
/** The close code of a socket whose person is no longer a member (`web_link::routes::CLOSE_NOT_A_MEMBER`). */
export const NOT_A_MEMBER = 4403;

export function subscribe(onChanged: (ev: WebEvent) => void, onState: (connected: boolean) => void, onRemoved?: () => void): () => void {
  let closed = false;
  let socket: WebSocket | null = null;
  let delay = 1000;
  const connect = () => {
    if (closed) {
      return;
    }
    const proto = location.protocol === "https:" ? "wss" : "ws";
    // One socket per tenant (ADR 0089): the server sends it that tenant's events only.
    const tenant = currentTenant();
    socket = new WebSocket(`${proto}://${location.host}/ws/web${tenant ? `?t=${encodeURIComponent(tenant)}` : ""}`);
    socket.onopen = () => {
      delay = 1000;
      onState(true);
    };
    socket.onmessage = (m) => {
      let ev: WebEvent;
      try {
        ev = JSON.parse(String(m.data)) as WebEvent;
      } catch {
        return; // ignore undecodable frames
      }
      try {
        onChanged(ev);
      } finally {
        emitWebEvent(ev);
      }
    };
    socket.onclose = (ev?: CloseEvent) => {
      onState(false);
      if (closed) {
        return;
      }
      // The server closed it because the person is no longer a member of this tenant (ADR 0088):
      // asking again would only be refused.
      if (ev?.code === NOT_A_MEMBER) {
        closed = true;
        onRemoved?.();
        return;
      }
      const wait = delay;
      delay = Math.min(delay * 2, 30_000);
      request<unknown>("GET", "/api/me")
        .then(() => true)
        .catch((e: unknown) => !(e instanceof ApiError && e.status === 401))
        .then((retry) => {
          if (retry && !closed) {
            setTimeout(connect, wait);
          }
        });
    };
    socket.onerror = () => socket?.close();
  };
  connect();
  return () => {
    closed = true;
    socket?.close();
  };
}
