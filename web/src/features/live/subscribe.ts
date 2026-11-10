import type { WebEvent } from "../../protocol.ts";

/**
 * Subscribe to `/ws/web`. Calls `onChanged` for every `WebEvent`, `onState` on connect and
 * disconnect, reconnects with backoff. Returns an unsubscribe function.
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
      if (!closed) {
        setTimeout(connect, delay);
        delay = Math.min(delay * 2, 30_000);
      }
    };
    socket.onerror = () => socket?.close();
  };
  connect();
  return () => {
    closed = true;
    socket?.close();
  };
}
