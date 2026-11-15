import { useWebResource } from "../live/index.ts";
import { failureWords } from "./api.ts";

/**
 * One resource of a launch (its messages, its log), kept current over `/ws/web` by `live`'s
 * `useWebResource`: read when mounted and again on `updated` for exactly this kind and launch.
 */
export function useLaunchResource<T>(kind: "launch_messages" | "launch_log", launchId: string, read: (launchId: string) => Promise<T>) {
  return useWebResource<T>({ kind, launch_id: launchId }, () => read(launchId), failureWords);
}
