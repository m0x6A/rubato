import { useCallback, useEffect, useRef, useState } from "react";
import type { WebResource } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import { onWebEvent } from "./events.ts";

/** A resource's identity: an `updated` event concerns a view when the two keys are equal. */
export function resourceKey(r: WebResource): string {
  switch (r.kind) {
    case "launch_messages":
    case "launch_log":
      return `${r.kind}:${r.launch_id}`;
    case "draft":
      return `${r.kind}:${r.draft_id}`;
    case "room":
      return `${r.kind}:${r.repo}#${r.number}:${r.branch}`;
  }
}

/**
 * One resource a view shows (a launch's messages or log, a draft), read when mounted and again
 * whenever `/ws/web` says `updated` for exactly that resource (plan 122). Only the newest read
 * lands, so a slow answer never overwrites a newer one. `read` and `words` may be new on every
 * render; the resource's key decides when to read again.
 */
export function useWebResource<T>(resource: WebResource, read: () => Promise<T>, words: (e: unknown) => string) {
  const key = resourceKey(resource);
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const latest = useRef(0);
  const reader = useRef({ read, words });
  useEffect(() => {
    reader.current = { read, words };
  });

  const reload = useCallback(async () => {
    const n = ++latest.current;
    try {
      const d = await reader.current.read();
      if (n === latest.current) {
        setData(d);
        setError(null);
      }
    } catch (e) {
      // A 401 raised needs-login in `request`; the login page follows, so nothing is said here.
      if (n === latest.current && !(e instanceof ApiError && e.status === 401)) {
        setError(reader.current.words(e));
      }
    }
    // `key` names the resource `read` reads, so it alone decides when to read again.
  }, [key]);

  useEffect(() => {
    void reload();
    return () => {
      latest.current += 1;
    };
  }, [reload]);

  useEffect(
    () =>
      onWebEvent((ev) => {
        if (ev.type === "updated" && resourceKey(ev.resource) === key) {
          void reload();
        }
      }),
    [key, reload],
  );

  return { data, error, reload };
}
