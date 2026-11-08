import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parse as parseYaml } from "yaml";
import type { IssueSummary, Overview, PairingInfo, SystemMap } from "../src/protocol.ts";

export const root = resolve(import.meta.dirname, "../..");

export function realMap(): SystemMap {
  return parseYaml(readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8")) as SystemMap;
}

export function overview(partial: Partial<Overview> = {}): Overview {
  return {
    map: { map: realMap(), version: "abc1234", loaded_at: new Date().toISOString() },
    issues: [],
    findings: [],
    companions: [],
    unpinned_sessions: [],
    generated_at: new Date().toISOString(),
    ...partial,
  };
}

export function companion(user: string, connected: boolean, extra: Partial<PairingInfo> = {}): PairingInfo {
  return { companion_id: `00000000-0000-0000-0000-00000000000${user.length}`, user, hostname: `${user}-laptop`, created_at: new Date().toISOString(), connected, ...extra };
}

export function issue(number: number, node: string, extra: Partial<IssueSummary> = {}): IssueSummary {
  const ref = { repo: "m0x6A/rubato", number };
  return {
    issue: ref,
    pins: [{ kind: "node", id: node }],
    progress: { issue: ref, state: "planned", last_seen: new Date().toISOString() },
    ...extra,
  };
}
