/**
 * The publisher against a stand-in for Grafana's HTTP API: what it sends, what it reads back,
 * and every way it must fail on purpose. Run with `pnpm observability:test`.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";
import { metricNames, missingSettings, publish, resolveDashboard, verifyLive, type Files, type Settings } from "./publish.ts";

const here = dirname(fileURLToPath(import.meta.url));
const files: Files = {
  dashboards: [{ name: "rubato-overview.json", text: readFileSync(resolve(here, "dashboards/rubato-overview.json"), "utf8") }],
  rules: [{ name: "rubato.rules.yml", text: readFileSync(resolve(here, "alerts/rubato.rules.yml"), "utf8") }],
};
const settings: Settings = { url: "https://grafana.test", token: "t0ken", prom: "prom-name", loki: "loki-name" };

type Call = { method: string; path: string; headers: Record<string, string>; body: string };
type Options = {
  folders?: { uid: string; title: string }[];
  datasources?: Record<string, string>;
  fail?: (c: Call) => { status: number; body: string } | undefined;
  /** Change what is read back, to make verification fail. */
  readBack?: { panels?: number; rules?: number };
  metrics?: string[];
};

/** A Grafana that keeps what it is sent. */
function grafana(o: Options = {}) {
  const calls: Call[] = [];
  const folders = [...(o.folders ?? [])];
  const datasources = o.datasources ?? { "prom-name": "prom-uid", "loki-name": "loki-uid" };
  const dashboards = new Map<string, { panels: unknown[]; title: string }>();
  const groups = new Map<string, { name: string; rules: unknown[] }[]>();
  const json = (status: number, body: unknown) => new Response(typeof body === "string" ? body : JSON.stringify(body), { status });
  const fetcher = async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input));
    const call: Call = { method: init?.method ?? "GET", path: url.pathname + url.search, headers: (init?.headers ?? {}) as Record<string, string>, body: String(init?.body ?? "") };
    calls.push(call);
    assert.equal(url.origin, "https://grafana.test");
    assert.equal(call.headers["Authorization"], "Bearer t0ken");
    const failed = o.fail?.(call);
    if (failed) {
      return json(failed.status, failed.body);
    }
    const p = url.pathname;
    if (p.startsWith("/api/datasources/name/")) {
      const uid = datasources[decodeURIComponent(p.slice("/api/datasources/name/".length))];
      return uid ? json(200, { uid }) : json(404, { message: "Data source not found" });
    }
    if (p === "/api/folders" && call.method === "GET") {
      return json(200, folders);
    }
    if (p === "/api/folders" && call.method === "POST") {
      const f = JSON.parse(call.body) as { uid: string; title: string };
      folders.push(f);
      return json(200, f);
    }
    if (p === "/api/dashboards/db") {
      const sent = JSON.parse(call.body) as { dashboard: { uid: string; title: string; panels: unknown[] } };
      dashboards.set(sent.dashboard.uid, sent.dashboard);
      return json(200, { uid: sent.dashboard.uid, status: "success", version: 3 });
    }
    if (p.startsWith("/api/dashboards/uid/")) {
      const d = dashboards.get(p.slice("/api/dashboards/uid/".length));
      return d ? json(200, { dashboard: { ...d, panels: o.readBack?.panels === undefined ? d.panels : d.panels.slice(0, o.readBack.panels) } }) : json(404, {});
    }
    const convert = "/api/convert/prometheus/config/v1/rules/";
    if (p.startsWith(convert) && call.method === "POST") {
      const ns = decodeURIComponent(p.slice(convert.length));
      const group = parseYaml(call.body) as { name: string; rules: unknown[] };
      groups.set(ns, [...(groups.get(ns) ?? []).filter((g) => g.name !== group.name), group]);
      return json(202, { status: "success" });
    }
    if (p.startsWith(convert) && call.method === "GET") {
      const ns = decodeURIComponent(p.slice(convert.length));
      const mine = (groups.get(ns) ?? []).map((g) => ({ ...g, rules: o.readBack?.rules === undefined ? g.rules : g.rules.slice(0, o.readBack.rules) }));
      return json(200, { [ns]: mine });
    }
    if (p === "/api/datasources/proxy/uid/prom-uid/api/v1/label/__name__/values") {
      return json(200, { status: "success", data: o.metrics ?? [] });
    }
    return json(404, { message: `stand-in has no ${call.method} ${p}` });
  };
  return { fetcher, calls, dashboards, groups, folders };
}

const writes = (calls: Call[]) => calls.filter((c) => c.method !== "GET");

test("publishes the committed dashboard and rules, reads them back, and reports the counts", async () => {
  const g = grafana();
  const r = await publish(files, settings, { fetch: g.fetcher });
  const committed = JSON.parse(files.dashboards[0]!.text) as { panels: unknown[] };
  const rules = (parseYaml(files.rules[0]!.text) as { groups: { rules: unknown[] }[] }).groups.flatMap((x) => x.rules);
  assert.deepEqual(r, { dashboards: 1, panels: committed.panels.length, groups: 1, rules: rules.length, dryRun: false });
  console.log(`published ${r.dashboards} dashboard (${r.panels} panels), ${r.rules} alert rules in ${r.groups} group`);

  // The folder did not exist, so it was made once and both kinds went into it.
  assert.deepEqual(g.folders, [{ uid: "rubato", title: "Rubato" }]);
  const sent = JSON.parse(writes(g.calls).find((c) => c.path === "/api/dashboards/db")!.body) as { folderUid: string; overwrite: boolean; dashboard: Record<string, unknown> };
  assert.equal(sent.folderUid, "rubato");
  assert.equal(sent.overwrite, true);
  assert.equal(sent.dashboard["uid"], "rubato-overview");
  assert.equal(sent.dashboard["__inputs"], undefined);
  const text = JSON.stringify(sent.dashboard);
  assert.ok(!text.includes("${DS_"), "no datasource placeholder is left");
  assert.ok(text.includes('"uid":"prom-uid"') && text.includes('"uid":"loki-uid"'));

  // The rules travel as the committed Prometheus rules, evaluated against the metrics datasource.
  const rulesCall = writes(g.calls).find((c) => c.path.startsWith("/api/convert/"))!;
  assert.equal(rulesCall.path, "/api/convert/prometheus/config/v1/rules/Rubato");
  assert.equal(rulesCall.headers["X-Grafana-Alerting-Datasource-UID"], "prom-uid");
  // No parent folder: the namespace is the folder, found by its title in the root.
  assert.equal(rulesCall.headers["X-Grafana-Alerting-Folder-UID"], undefined);
  assert.equal(rulesCall.headers["X-Grafana-Alerting-Alert-Rules-Paused"], undefined);
  const group = parseYaml(rulesCall.body) as { name: string; rules: { alert: string; expr: string }[] };
  assert.equal(group.name, "rubato");
  assert.deepEqual(group.rules, rules);
});

test("an existing folder is reused, whatever its uid", async () => {
  const g = grafana({ folders: [{ uid: "made-by-hand", title: "Rubato" }] });
  await publish(files, settings, { fetch: g.fetcher });
  assert.equal(g.folders.length, 1);
  assert.equal((JSON.parse(writes(g.calls)[0]!.body) as { folderUid: string }).folderUid, "made-by-hand");
});

test("alert rules can be imported paused", async () => {
  const g = grafana();
  await publish(files, { ...settings, alertsPaused: true }, { fetch: g.fetcher });
  assert.equal(writes(g.calls).find((c) => c.path.startsWith("/api/convert/"))!.headers["X-Grafana-Alerting-Alert-Rules-Paused"], "true");
});

test("a dry run reads but writes nothing, says it would make the folder, and still reports the counts", async () => {
  const g = grafana();
  const lines: string[] = [];
  const r = await publish(files, settings, { fetch: g.fetcher, dryRun: true, log: (l) => lines.push(l) });
  assert.ok(lines.some((l) => l.includes('folder "Rubato" does not exist; a publish would make it')));
  assert.equal(r.dryRun, true);
  assert.ok(r.panels > 0 && r.rules > 0);
  assert.deepEqual(writes(g.calls), []);
});

test("publishing twice leaves one dashboard and one group", async () => {
  const g = grafana();
  await publish(files, settings, { fetch: g.fetcher });
  await publish(files, settings, { fetch: g.fetcher });
  assert.equal(g.dashboards.size, 1);
  assert.equal(g.groups.get("Rubato")!.length, 1);
  assert.equal(g.folders.length, 1);
});

// ----- must fail ------------------------------------------------------------------------------

test("missing settings are all named, and an empty value counts as missing", () => {
  assert.deepEqual(missingSettings({ GRAFANA_URL: "https://x", GRAFANA_API_TOKEN: "", GRAFANA_PROM_DS: "  " }), ["GRAFANA_API_TOKEN", "GRAFANA_PROM_DS", "GRAFANA_LOKI_DS"]);
  assert.deepEqual(missingSettings({ GRAFANA_URL: "u", GRAFANA_API_TOKEN: "t", GRAFANA_PROM_DS: "p", GRAFANA_LOKI_DS: "l" }), []);
});

test("an unknown datasource fails before anything is written and names the setting", async () => {
  const g = grafana({ datasources: { "prom-name": "prom-uid" } });
  await assert.rejects(publish(files, settings, { fetch: g.fetcher }), /datasource "loki-name" not found.*GRAFANA_LOKI_DS/);
  assert.deepEqual(writes(g.calls), []);
});

test("a refused call fails with the method, path, status and Grafana's answer, never the token", async () => {
  const g = grafana({ fail: (c) => (c.path === "/api/dashboards/db" ? { status: 403, body: '{"message":"Access denied"}' } : undefined) });
  await assert.rejects(publish(files, settings, { fetch: g.fetcher }), (e: Error) => {
    assert.match(e.message, /POST \/api\/dashboards\/db answered 403.*Access denied/);
    assert.ok(!e.message.includes("t0ken"));
    return true;
  });
});

test("a dashboard that reads back with fewer panels fails", async () => {
  const g = grafana({ readBack: { panels: 2 } });
  await assert.rejects(publish(files, settings, { fetch: g.fetcher }), /rubato-overview reads back with 2 panels, sent \d+/);
});

test("rules that read back short fail", async () => {
  const g = grafana({ readBack: { rules: 1 } });
  await assert.rejects(publish(files, settings, { fetch: g.fetcher }), /group rubato reads back with 1 rules, sent \d+/);
});

test("a datasource placeholder nobody resolves is refused", () => {
  const d = { uid: "x", title: "x", panels: [{ title: "p", datasource: { type: "tempo", uid: "${DS_TEMPO}" } }] };
  assert.throws(() => resolveDashboard(JSON.stringify(d), { "${DS_PROMETHEUS}": "a" }, "x.json"), /x\.json: unresolved datasource \$\{DS_TEMPO\}/);
});

test("a dashboard without a uid, and a rules file without groups, are refused", async () => {
  const g = grafana();
  await assert.rejects(publish({ dashboards: [{ name: "no-uid.json", text: '{"title":"t","panels":[]}' }], rules: [] }, settings, { fetch: g.fetcher }), /no-uid\.json: a dashboard needs a uid/);
  await assert.rejects(publish({ dashboards: [], rules: [{ name: "empty.rules.yml", text: "groups: []" }] }, settings, { fetch: g.fetcher }), /empty\.rules\.yml: no rule groups/);
  assert.deepEqual(writes(g.calls), []);
});

test("a missing dashboard on read-back, and an answer that is not JSON, name the call", async () => {
  const gone = grafana({ fail: (c) => (c.path.startsWith("/api/dashboards/uid/") ? { status: 404, body: "{}" } : undefined) });
  await assert.rejects(publish(files, settings, { fetch: gone.fetcher }), /GET \/api\/dashboards\/uid\/rubato-overview answered 404/);
  const html = grafana({ fail: (c) => (c.path.startsWith("/api/folders") ? { status: 200, body: "<html>login</html>" } : undefined) });
  await assert.rejects(publish(files, settings, { fetch: html.fetcher }), /GET \/api\/folders\?limit=1000 answered something that does not parse/);
});

test("verify-live fails when the stack gives no list of names, instead of calling everything missing", async () => {
  const g = grafana({ fail: (c) => (c.path.includes("/api/v1/label/") ? { status: 200, body: '{"status":"error"}' } : undefined) });
  await assert.rejects(verifyLive(files, settings, { fetch: g.fetcher }), /answered no list of metric names/);
});

// ----- what the stack has seen ----------------------------------------------------------------

test("metric names are read from the dashboard's and the rules' expressions", () => {
  const names = metricNames(files);
  assert.ok(names.has("rubato_companion_heartbeat"));
  assert.ok(names.has("rubato_map_loaded_at"));
  for (const word of ["sum", "by", "rate", "time", "max", "rubato_companion_id", "and", "increase"]) {
    assert.ok(!names.has(word), `${word} is not a metric`);
  }
  const odd: Files = {
    dashboards: [
      {
        name: "odd.json",
        text: JSON.stringify({
          panels: [
            { datasource: { type: "prometheus" }, targets: [{ expr: "rate(a_total[5m] offset 1h) * 1e3 > bool on (x) b" }] },
            { datasource: { type: "loki" }, targets: [{ expr: 'sum(count_over_time({app="x"} | json [5m]))' }] },
          ],
          templating: { list: [{ query: "label_values(job)" }, { query: "label_values(c_total, job)" }] },
        }),
      },
    ],
    rules: [],
  };
  assert.deepEqual([...metricNames(odd)].sort(), ["a_total", "b", "c_total"]);
  console.log(`found ${names.size} metric names`);
});

test("verify-live lists the metrics the stack has never stored", async () => {
  const all = [...metricNames(files)].sort();
  const g = grafana({ metrics: all.slice(1) });
  const r = await verifyLive(files, settings, { fetch: g.fetcher });
  assert.deepEqual(r, { checked: all.length, missing: [all[0]] });
  assert.deepEqual(writes(g.calls), []);
});
