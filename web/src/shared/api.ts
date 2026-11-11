/** The `web-server` seam from the browser side. Types are generated from the protocol schemas. */

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

/**
 * "Needs login": the server answers 401 to any call once the session lacks a fresh team record
 * (design S24). One subscriber (the shell); several 401s in a row raise it once, until
 * `resetNeedsLogin` says a login worked.
 */
let needsLogin = false;
let needsLoginListener: (() => void) | null = null;

/** Subscribe to "needs login". Fires at once when it was raised before anyone listened. Returns unsubscribe. */
export function onNeedsLogin(cb: () => void): () => void {
  needsLoginListener = cb;
  if (needsLogin) {
    cb();
  }
  return () => {
    if (needsLoginListener === cb) {
      needsLoginListener = null;
    }
  };
}

export function resetNeedsLogin(): void {
  needsLogin = false;
}

function raiseNeedsLogin(): void {
  if (needsLogin) {
    return;
  }
  needsLogin = true;
  needsLoginListener?.();
}

/** What to tell the person when a call failed: the server's own reason, or that it was not reached. */
export function failureText(e: unknown): string {
  return e instanceof ApiError ? e.message : "Could not reach the server";
}

export async function request<T>(method: string, url: string, body?: unknown, headers: Record<string, string> = {}): Promise<T> {
  const init: RequestInit = { method, credentials: "same-origin", headers: { ...headers } };
  if (body !== undefined) {
    init.headers = { "Content-Type": "application/json", ...headers };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(url, init);
  if (!res.ok) {
    if (res.status === 401) {
      raiseNeedsLogin();
    }
    throw new ApiError(res.status, (await res.text()) || res.statusText);
  }
  // Some successes carry no body: 204, and the 201 of `POST /api/pins`.
  const text = res.status === 204 ? "" : await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}
