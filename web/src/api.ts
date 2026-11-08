/** The `web-server` seam from the browser side. Types are generated from the protocol schemas. */
import type { LaunchRequest, LaunchResponse, Me, Overview, PairApproveRequest, PairingInfo, PinRequest, WebEvent } from "./protocol.ts";

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

async function request<T>(method: string, url: string, body?: unknown, headers: Record<string, string> = {}): Promise<T> {
  const init: RequestInit = { method, credentials: "same-origin", headers: { ...headers } };
  if (body !== undefined) {
    init.headers = { "Content-Type": "application/json", ...headers };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(url, init);
  if (!res.ok) {
    throw new ApiError(res.status, (await res.text()) || res.statusText);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  return (await res.json()) as T;
}

export const api = {
  me: () => request<Me>("GET", "/api/me"),
  overview: () => request<Overview>("GET", "/api/overview"),
  addPin: (req: PinRequest) => request<void>("POST", "/api/pins", req),
  removePin: (req: PinRequest) => request<void>("DELETE", "/api/pins", req),
  launch: (req: LaunchRequest) => request<LaunchResponse>("POST", "/api/launch", req),
  pairings: () => request<PairingInfo[]>("GET", "/api/pairings"),
  approvePairing: (req: PairApproveRequest) => request<PairingInfo>("POST", "/api/pair/approve", req),
  revokePairing: (companionId: string) => request<void>("DELETE", `/api/pairings/${companionId}`),
  reloadMap: () => request<unknown>("POST", "/api/map/reload"),
};

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
