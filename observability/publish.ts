#!/usr/bin/env -S node --experimental-strip-types
/**
 * Publish the dashboards and the alert rules to a Grafana stack over its HTTP API (issue #57).
 *
 *   publish.ts                publish, then read back and compare
 *   publish.ts --dry-run      resolve datasources and the folder, write nothing
 *   publish.ts --verify-live  list the metrics the files query that the stack has never stored
 *                             (advisory: a counter that has not counted yet is not stored)
 *
 * Settings, from the environment: GRAFANA_URL, GRAFANA_API_TOKEN, GRAFANA_PROM_DS, GRAFANA_LOKI_DS
 * (the last two are datasource names). An empty value counts as missing. Optional:
 * GRAFANA_ALERTS_PAUSED=true imports the alert rules paused, so they are on the stack but do
 * not evaluate or fire.
 *
 * Dashboards: `dashboards/*.json`, with `${DS_PROMETHEUS}` and `${DS_LOKI}` replaced by the
 * datasources' uids, written with overwrite into the folder "Rubato".
 *
 * Alert rules: `alerts/*.rules.yml` stay in Prometheus format, the one `promtool test rules`
 * tests. Each group is sent as it is to Grafana's import route with the namespace "Rubato".
 * Grafana finds or makes the folder with that title in the root, which is the folder the
 * dashboards go into, and keeps the group there as Grafana-managed rules evaluated against the
 * metrics datasource. No parent-folder header is sent: it would nest a second "Rubato" inside. Sending
 * a group replaces that group, so a rule removed from the file is removed from the stack. A
 * group or dashboard removed from the repository is not: delete it in Grafana.
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml, stringify as toYaml } from "yaml";

export const FOLDER = { uid: "rubato", title: "Rubato" };
const REQUIRED = ["GRAFANA_URL", "GRAFANA_API_TOKEN", "GRAFANA_PROM_DS", "GRAFANA_LOKI_DS"] as const;
const CONVERT = "/api/convert/prometheus/config/v1/rules";

export type Settings = { url: string; token: string; prom: string; loki: string; alertsPaused?: boolean };
export type Files = { dashboards: { name: string; text: string }[]; rules: { name: string; text: string }[] };
export type Deps = { fetch?: typeof fetch; dryRun?: boolean; log?: (line: string) => void };
export type Published = { dashboards: number; panels: number; groups: number; rules: number; dryRun: boolean };

type Group = { name: string; interval?: string; rules: Record<string, unknown>[] };
type Dashboard = { uid?: string; title?: string; panels?: unknown[] } & Record<string, unknown>;

/** The settings that are unset or blank. An unset secret reaches a workflow as "". */
export function missingSettings(env: Record<string, string | undefined>): string[] {
  return REQUIRED.filter((name) => !(env[name] ?? "").trim());
}

type Extra = { headers?: Record<string, string>; allow404?: boolean };

/** One request. A refusal carries the method, the path, the status and the answer. */
async function call(s: Settings, deps: Deps, method: string, path: string, body?: string, extra: Extra = {}): Promise<{ status: number; text: string }> {
  const headers = extra.headers ?? {};
  const res = await (deps.fetch ?? fetch)(`${s.url.replace(/\/+$/, "")}${path}`, {
    method,
    headers: { Authorization: `Bearer ${s.token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }), ...headers },
    ...(body === undefined ? {} : { body }),
    signal: AbortSignal.timeout(60_000),
  });
  const text = await res.text();
  if (!res.ok && !(extra.allow404 && res.status === 404)) {
    throw new Error(`${method} ${path} answered ${res.status}: ${text.slice(0, 400)}`);
  }
  return { status: res.status, text };
}

/** The answer as JSON or YAML; an answer that is neither (a login page, say) names the call. */
function parsed<T>(answer: { text: string }, method: string, path: string, parse: (text: string) => unknown = JSON.parse): T {
  try {
    return parse(answer.text) as T;
  } catch {
    throw new Error(`${method} ${path} answered something that does not parse: ${answer.text.slice(0, 120)}`);
  }
}

async function datasourceUid(s: Settings, deps: Deps, name: string, setting: string): Promise<string> {
  const path = `/api/datasources/name/${encodeURIComponent(name)}`;
  const r = await call(s, deps, "GET", path, undefined, { allow404: true });
  if (r.status === 404) {
    throw new Error(`datasource ${JSON.stringify(name)} not found on the stack; check ${setting}`);
  }
  return parsed<{ uid: string }>(r, "GET", path).uid;
}

/**
 * The root folder titled exactly "Rubato", by title: one made by hand has another uid, and the
 * rule import finds its folder by this same title.
 */
async function folderUid(s: Settings, deps: Deps): Promise<string> {
  const path = "/api/folders?limit=1000";
  const folders = parsed<{ uid: string; title: string }[]>(await call(s, deps, "GET", path), "GET", path);
  const found = folders.find((f) => f.title === FOLDER.title);
  if (found) {
    return found.uid;
  }
  deps.log?.(`folder ${JSON.stringify(FOLDER.title)} does not exist; ${deps.dryRun ? "a publish would make it" : "making it"}`);
  if (!deps.dryRun) {
    await call(s, deps, "POST", "/api/folders", JSON.stringify(FOLDER));
  }
  return FOLDER.uid;
}

/** The dashboard as Grafana stores it: placeholders resolved, the import inputs dropped. */
export function resolveDashboard(text: string, uids: Record<string, string>, name: string): Dashboard {
  let resolved = text;
  for (const [placeholder, uid] of Object.entries(uids)) {
    resolved = resolved.replaceAll(placeholder, uid);
  }
  const left = resolved.match(/\$\{DS_[A-Z0-9_]+\}/);
  if (left) {
    throw new Error(`${name}: unresolved datasource ${left[0]}`);
  }
  const dashboard = JSON.parse(resolved) as Dashboard;
  if (!dashboard.uid) {
    throw new Error(`${name}: a dashboard needs a uid, or every publish makes a new one`);
  }
  delete dashboard["__inputs"];
  delete dashboard["__requires"];
  dashboard["id"] = null;
  return dashboard;
}

function groupsOf(file: { name: string; text: string }): Group[] {
  const groups = (parseYaml(file.text) as { groups?: Group[] } | null)?.groups ?? [];
  if (groups.length === 0) {
    throw new Error(`${file.name}: no rule groups`);
  }
  return groups;
}

export async function publish(files: Files, s: Settings, deps: Deps = {}): Promise<Published> {
  const log = deps.log ?? (() => {});
  // Everything that can be wrong with the files or the settings is found before the first write.
  const groups = files.rules.flatMap(groupsOf);
  const prom = await datasourceUid(s, deps, s.prom, "GRAFANA_PROM_DS");
  const loki = await datasourceUid(s, deps, s.loki, "GRAFANA_LOKI_DS");
  const dashboards = files.dashboards.map((f) => resolveDashboard(f.text, { "${DS_PROMETHEUS}": prom, "${DS_LOKI}": loki }, f.name));
  const folder = await folderUid(s, deps);
  const out: Published = { dashboards: 0, panels: 0, groups: 0, rules: 0, dryRun: deps.dryRun ?? false };

  for (const dashboard of dashboards) {
    const sent = dashboard.panels?.length ?? 0;
    if (!deps.dryRun) {
      await call(s, deps, "POST", "/api/dashboards/db", JSON.stringify({ dashboard, folderUid: folder, overwrite: true, message: "published by observability/publish.ts" }));
      const path = `/api/dashboards/uid/${dashboard.uid}`;
      const got = parsed<{ dashboard: Dashboard }>(await call(s, deps, "GET", path), "GET", path).dashboard.panels?.length ?? 0;
      if (got !== sent) {
        throw new Error(`dashboard ${dashboard.uid} reads back with ${got} panels, sent ${sent}`);
      }
    }
    log(`dashboard ${dashboard.uid}: ${sent} panels`);
    out.dashboards += 1;
    out.panels += sent;
  }

  for (const group of groups) {
    if (!deps.dryRun) {
      const path = `${CONVERT}/${encodeURIComponent(FOLDER.title)}`;
      await call(s, deps, "POST", path, toYaml(group), {
        headers: {
          "Content-Type": "application/yaml",
          "X-Grafana-Alerting-Datasource-UID": prom,
          ...(s.alertsPaused ? { "X-Grafana-Alerting-Alert-Rules-Paused": "true" } : {}),
        },
      });
      const back = await call(s, deps, "GET", path, undefined, { headers: { Accept: "application/yaml" } });
      const stored = Object.values(parsed<Record<string, Group[]> | null>(back, "GET", path, parseYaml) ?? {}).flat();
      const got = stored.find((g) => g.name === group.name)?.rules.length ?? 0;
      if (got !== group.rules.length) {
        throw new Error(`group ${group.name} reads back with ${got} rules, sent ${group.rules.length}`);
      }
    }
    log(`rule group ${group.name}: ${group.rules.length} rules${s.alertsPaused ? " (paused)" : ""}`);
    out.groups += 1;
    out.rules += group.rules.length;
  }
  return out;
}

// ----- what the stack has seen ----------------------------------------------------------------

const NOT_METRICS = new Set(["by", "without", "on", "ignoring", "group_left", "group_right", "and", "or", "unless", "bool", "offset", "inf", "nan"]);

/** Every metric name an expression of the dashboards or the rules queries. */
export function metricNames(files: Files): Set<string> {
  const exprs: string[] = [];
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) {
      node.forEach(walk);
    } else if (node && typeof node === "object") {
      // A Loki panel's queries are `expr` too; only Prometheus panels and variables are read.
      const ds = (node as { datasource?: { type?: string } }).datasource;
      if (ds && typeof ds === "object" && ds.type !== undefined && ds.type !== "prometheus") {
        return;
      }
      for (const [key, value] of Object.entries(node)) {
        if ((key === "expr" || key === "query") && typeof value === "string") {
          exprs.push(value);
        } else {
          walk(value);
        }
      }
    }
  };
  files.dashboards.forEach((f) => walk(JSON.parse(f.text)));
  files.rules.forEach((f) => groupsOf(f).forEach((g) => g.rules.forEach((r) => exprs.push(String(r["expr"] ?? "")))));
  const names = new Set<string>();
  for (const expr of exprs) {
    // Label matchers, groupings, strings and durations hold words that are not metrics.
    const bare = expr
      .replace(/"(?:[^"\\]|\\.)*"/g, '""')
      .replace(/label_values\(\s*([a-zA-Z_:][\w:]*)\s*,[^)]*\)/g, "$1")
      .replace(/label_values\([^)]*\)/g, " ")
      .replace(/\boffset\s+-?[\w.]+/g, " ")
      .replace(/\{[^}]*\}/g, "")
      .replace(/\b(by|without|on|ignoring|group_left|group_right)\s*\([^)]*\)/g, " ")
      .replace(/\[[^\]]*\]/g, "")
      .replace(/\$\{?\w+\}?/g, " ");
    // Not a word that continues a number, such as the e3 of 1e3.
    for (const m of bare.matchAll(/(?<![\w.])[a-zA-Z_:][\w:]*/g)) {
      const word = m[0];
      const next = bare.slice((m.index ?? 0) + word.length).trimStart()[0];
      if (next !== "(" && !NOT_METRICS.has(word.toLowerCase())) {
        names.add(word);
      }
    }
  }
  return names;
}

export async function verifyLive(files: Files, s: Settings, deps: Deps = {}): Promise<{ checked: number; missing: string[] }> {
  const names = metricNames(files);
  const prom = await datasourceUid(s, deps, s.prom, "GRAFANA_PROM_DS");
  const path = `/api/datasources/proxy/uid/${prom}/api/v1/label/__name__/values`;
  const data = parsed<{ data?: string[] }>(await call(s, deps, "GET", path), "GET", path).data;
  if (!Array.isArray(data)) {
    throw new Error(`GET ${path} answered no list of metric names`);
  }
  const stored = new Set(data);
  return { checked: names.size, missing: [...names].filter((n) => !stored.has(n)).sort() };
}

// ----- command line ---------------------------------------------------------------------------

function readFiles(here: string): Files {
  const read = (dir: string, end: string) =>
    readdirSync(resolve(here, dir))
      .filter((n) => n.endsWith(end))
      .sort()
      .map((name) => ({ name, text: readFileSync(resolve(here, dir, name), "utf8") }));
  return { dashboards: read("dashboards", ".json"), rules: read("alerts", ".rules.yml") };
}

const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  const argv = process.argv.slice(2);
  const missing = missingSettings(process.env);
  if (missing.length > 0) {
    console.error(`observability publish: missing setting(s): ${missing.join(", ")}`);
    process.exit(2);
  }
  const env = process.env as Record<string, string>;
  const s: Settings = { url: env["GRAFANA_URL"]!, token: env["GRAFANA_API_TOKEN"]!, prom: env["GRAFANA_PROM_DS"]!, loki: env["GRAFANA_LOKI_DS"]!, alertsPaused: (env["GRAFANA_ALERTS_PAUSED"] ?? "").trim().toLowerCase() === "true" };
  const files = readFiles(dirname(fileURLToPath(import.meta.url)));
  try {
    if (argv.includes("--verify-live")) {
      const r = await verifyLive(files, s);
      console.log(`observability verify-live: ${r.checked} metric names queried; ${r.missing.length} never stored on the stack`);
      r.missing.forEach((m) => console.log(`  - ${m}`));
    } else {
      const r = await publish(files, s, { dryRun: argv.includes("--dry-run"), log: (l) => console.log(`  ${l}`) });
      console.log(`observability publish${r.dryRun ? " (dry run, nothing written)" : ""}: ${r.dashboards} dashboard(s) with ${r.panels} panels, ${r.rules} alert rule(s) in ${r.groups} group(s)`);
    }
  } catch (e) {
    console.error(`observability publish failed: ${(e as Error).message}`);
    process.exit(1);
  }
}
