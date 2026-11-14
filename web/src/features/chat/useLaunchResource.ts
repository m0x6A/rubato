import { useCallback, useEffect, useRef, useState } from "react";
import type { WebResource } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import { onWebEvent } from "../live/index.ts";
import { failureWords } from "./api.ts";

/**
 * One resource of a launch (its messages, its log), read when mounted and again whenever
 * `/ws/web` says `updated` for exactly this kind and launch (plan 122 §D2). Only the newest read
 * lands, so a slow answer never overwrites a newer one. `read` must keep its identity across
 * renders (a function of `chatApi`).
 */
export function useLaunchResource<T>(kind: WebResource["kind"], launchId: string, read: (launchId: string) => Promise<T>) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const latest = useRef(0);

  const reload = useCallback(async () => {
    const n = ++latest.current;
    try {
      const d = await read(launchId);
      if (n === latest.current) {
        setData(d);
        setError(null);
      }
    } catch (e) {
      // A 401 raised needs-login in `request`; the login page follows, so nothing is said here.
      if (n === latest.current && !(e instanceof ApiError && e.status === 401)) {
        setError(failureWords(e));
      }
    }
  }, [launchId, read]);

  useEffect(() => {
    void reload();
    return () => {
      latest.current += 1;
    };
  }, [reload]);

  useEffect(
    () =>
      onWebEvent((ev) => {
        if (ev.type === "updated" && ev.resource.kind === kind && ev.resource.launch_id === launchId) {
          void reload();
        }
      }),
    [kind, launchId, reload],
  );

  return { data, error, reload };
}
