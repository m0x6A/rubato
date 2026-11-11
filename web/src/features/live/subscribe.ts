import type { WebEvent } from "../../protocol.ts";
import { ApiError, request } from "../../shared/api.ts";

/**
 * Subscribe to `/ws/web`. Calls `onChanged` for every `WebEvent`, `onState` on connect and
 * disconnect, reconnects with backoff. Returns an unsubscribe function.
 *
 * A browser cannot see why an upgrade was refused, so every close probes `GET /api/me`: a 401
 * means the session needs a login (design S24), which `request` raises, and the retries stop.
 * Anything else (the session holds, or the server cannot be reached) is "disconnected": retry.
 */
export function subscribe(onChanged: (ev: WebEvent) => void, onState: (connected: boolean) => void): () => void {
  let closed = false;
  let socket: WebSocket | null = null;
  let delay = 1000;
  const connect = () => {
    if (closed) {
      return;
    }
    const proto = location.protocol === "https:" ? "wss" : "ws";
    socket = new WebSocket(`${proto}://${location.host}/ws/web`);
    socket.onopen = () => {
      delay = 1000;
      onState(true);
    };
    socket.onmessage = (m) => {
      try {
        onChanged(JSON.parse(String(m.data)) as WebEvent);
      } catch {
        // ignore undecodable frames
      }
    };
    socket.onclose = () => {
      onState(false);
      if (closed) {
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
