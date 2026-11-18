import { spawnSync, spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { createServer, type IncomingMessage, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { ATTR, call, config, parsePairs, parseTraceparent, Recorder, runCommand, SPAN } from "../scripts/otel-emit.ts";

const script = resolve(import.meta.dirname, "../scripts/otel-emit.ts");

interface Received {
  path: string;
  headers: IncomingMessage["headers"];
  body: any;
}

/** A collector on a free port that records every export; `hang` never answers. */
async function collector(hang = false): Promise<{ url: string; got: Received[]; server: Server }> {
  const got: Received[] = [];
  const server = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => {
      got.push({ path: req.url ?? "", headers: req.headers, body: JSON.parse(raw || "{}") });
      if (!hang) {
        res.writeHead(200, { "content-type": "application/json" }).end("{}");
      }
    });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const { port } = server.address() as AddressInfo;
  servers.push(server);
  return { url: `http://127.0.0.1:${port}`, got, server };
}

const servers: Server[] = [];
afterEach(() => {
  for (const s of servers.splice(0)) {
    s.closeAllConnections();
    s.close();
  }
});

function spans(r: Received): any[] {
  return r.body.resourceSpans[0].scopeSpans[0].spans;
}

function resource(r: Received): Record<string, unknown> {
  return Object.fromEntries(r.body.resourceSpans[0].resource.attributes.map((a: any) => [a.key, Object.values(a.value)[0]]));
}

function attrs(s: any): Record<string, unknown> {
  return Object.fromEntries(s.attributes.map((a: any) => [a.key, Object.values(a.value)[0]]));
}

describe("config", () => {
  it("prefers the standard names, falls back to the RUBATO_ mirror, and sends nothing without an endpoint", () => {
    expect(config({})).toBeNull();
    expect(config({ OTEL_EXPORTER_OTLP_ENDPOINT: "http://a:4318/" })?.url).toBe("http://a:4318/v1/traces");
    expect(config({ OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: "http://t/x" })?.url).toBe("http://t/x");
    expect(config({ RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT: "http://m:4318" })?.url).toBe("http://m:4318/v1/traces");
    const both = config({
      OTEL_EXPORTER_OTLP_ENDPOINT: "http://std:4318",
      RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT: "http://mirror:4318",
      RUBATO_OTEL_RESOURCE_ATTRIBUTES: "service.namespace=mirror",
      OTEL_RESOURCE_ATTRIBUTES: "service.namespace=std",
    });
    expect(both?.url).toBe("http://std:4318/v1/traces");
    expect(both?.resource["service.namespace"]).toBe("std");
  });

  it("is in the rubato namespace unless the resource attributes set another (ADR 0080)", () => {
    const base = { OTEL_EXPORTER_OTLP_ENDPOINT: "http://a:4318" };
    expect(config(base)?.resource["service.namespace"]).toBe("rubato");
    expect(config({ ...base, OTEL_RESOURCE_ATTRIBUTES: "deployment.environment=dev" })?.resource).toEqual({ "service.namespace": "rubato", "deployment.environment": "dev" });
    // Must fail on purpose if the default wins over the environment.
    expect(config({ ...base, OTEL_RESOURCE_ATTRIBUTES: "service.namespace=team" })?.resource["service.namespace"]).toBe("team");
    expect(config({ ...base, RUBATO_OTEL_RESOURCE_ATTRIBUTES: "service.namespace=mirror" })?.resource["service.namespace"]).toBe("mirror");
  });

  it("skips grpc, a disabled SDK and a non-otlp exporter at once", () => {
    const e = { OTEL_EXPORTER_OTLP_ENDPOINT: "http://a:4317" };
    expect(config({ ...e, OTEL_EXPORTER_OTLP_PROTOCOL: "grpc" })).toBeNull();
    expect(config({ ...e, RUBATO_OTEL_EXPORTER_OTLP_PROTOCOL: "grpc" })).toBeNull();
    expect(config({ ...e, OTEL_SDK_DISABLED: "true" })).toBeNull();
    expect(config({ ...e, OTEL_TRACES_EXPORTER: "none" })).toBeNull();
    expect(config({ ...e, OTEL_EXPORTER_OTLP_PROTOCOL: "http/protobuf" })).not.toBeNull();
  });

  it("decodes headers and resource attributes", () => {
    expect(parsePairs("Authorization=Basic%20abc,x-y = z ,bad")).toEqual({ Authorization: "Basic abc", "x-y": "z" });
  });

  it("reads a valid traceparent only", () => {
    expect(parseTraceparent("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")).toEqual({
      traceId: "0af7651916cd43dd8448eb211c80319c",
      spanId: "b7ad6b7169203331",
    });
    expect(parseTraceparent("00-00000000000000000000000000000000-b7ad6b7169203331-01")).toBeNull();
    expect(parseTraceparent("garbage")).toBeNull();
    expect(parseTraceparent(undefined)).toBeNull();
  });
});

describe("runCommand and call", () => {
  it("sends one export with the command span, its client spans, the resource and the exit code", async () => {
    const c = await collector();
    const env = {
      RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT: c.url,
      RUBATO_OTEL_EXPORTER_OTLP_HEADERS: "authorization=Bearer%20t",
      RUBATO_OTEL_RESOURCE_ATTRIBUTES: "service.namespace=team,rubato.issue=m0x6A/rubato#142,service.name=claude-code",
      TRACEPARENT: "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
    };
    const code = await runCommand(
      "rubato",
      "plan check",
      async () => {
        call("plan-file", () => "read");
        await call("github", async () => "listed");
        return 1;
      },
      env,
    );
    expect(code).toBe(1);
    expect(c.got).toHaveLength(1);
    const r = c.got[0] as Received;
    expect(r.path).toBe("/v1/traces");
    expect(r.headers["authorization"]).toBe("Bearer t");
    expect(r.headers["content-type"]).toBe("application/json");
    expect(resource(r)).toMatchObject({ "service.name": "rubato", "service.namespace": "team", "rubato.issue": "m0x6A/rubato#142" });
    const s = spans(r);
    expect(s.map((x) => x.name)).toEqual([SPAN.call, SPAN.call, SPAN.command]);
    const cmd = s[2];
    expect(cmd.traceId).toBe("0af7651916cd43dd8448eb211c80319c");
    expect(cmd.parentSpanId).toBe("b7ad6b7169203331");
    expect(cmd.kind).toBe(1);
    expect(cmd.status.code).toBe(2);
    expect(attrs(cmd)).toMatchObject({ [ATTR.skillName]: "rubato", [ATTR.subcommand]: "plan check", [ATTR.exitCode]: "1" });
    for (const [i, peer] of [[0, "plan-file"], [1, "github"]] as const) {
      expect(s[i].kind).toBe(3);
      expect(s[i].parentSpanId).toBe(cmd.spanId);
      expect(attrs(s[i])[ATTR.peerService]).toBe(peer);
    }
  });

  it("starts a new trace without a traceparent, and marks a throw as an error after sending", async () => {
    const c = await collector();
    await expect(
      runCommand("rubato", "map", () => {
        throw new Error("boom");
      }, { OTEL_EXPORTER_OTLP_ENDPOINT: c.url }),
    ).rejects.toThrow("boom");
    const s = spans(c.got[0] as Received);
    expect(s).toHaveLength(1);
    expect(s[0].parentSpanId).toBeUndefined();
    expect(s[0].traceId).toMatch(/^[0-9a-f]{32}$/);
    expect(s[0].status.code).toBe(2);
  });

  it("sends nothing and records nothing without an endpoint", async () => {
    const rec = new Recorder("rubato", {});
    expect(rec.span(SPAN.command, {}, () => 7)).toBe(7);
    expect(rec.recorded).toBe(0);
    expect(await runCommand("rubato", "map", () => 0, {})).toBe(0);
    expect(call("github", () => "plain")).toBe("plain");
  });

  it("gives up on a collector that never answers within a second", async () => {
    const c = await collector(true);
    const t0 = Date.now();
    expect(await runCommand("rubato", "map", () => 0, { OTEL_EXPORTER_OTLP_ENDPOINT: c.url })).toBe(0);
    expect(Date.now() - t0).toBeLessThan(1000);
  });

  it("does not throw when the collector is not there", async () => {
    expect(await runCommand("rubato", "map", () => 0, { OTEL_EXPORTER_OTLP_ENDPOINT: "http://127.0.0.1:1" })).toBe(0);
  });
});

/** Run the CLI with a fresh env, without inheriting the test runner's OTEL settings. */
function runCli(args: string[], env: Record<string, string>): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((done) => {
    const child = spawn(process.execPath, ["--experimental-strip-types", "--no-warnings", script, ...args], {
      env: { PATH: process.env["PATH"] ?? "", ...env },
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (c) => (stdout += c));
    child.stderr.on("data", (c) => (stderr += c));
    child.on("exit", (code) => done({ code, stdout, stderr }));
  });
}

describe("cli", () => {
  it("passes stdout, stderr and the exit code through and sends one span for the command", async () => {
    const c = await collector();
    const r = await runCli(
      ["span", "--skill", "notes", "--peer", "github", "--", process.execPath, "-e", "process.stdout.write('out');process.stderr.write('err');process.exit(3)"],
      { OTEL_EXPORTER_OTLP_ENDPOINT: c.url, OTEL_RESOURCE_ATTRIBUTES: "service.namespace=team" },
    );
    expect(r).toEqual({ code: 3, stdout: "out", stderr: "err" });
    expect(c.got).toHaveLength(1);
    const s = spans(c.got[0] as Received);
    expect(s).toHaveLength(1);
    expect(s[0].name).toBe(SPAN.call);
    expect(s[0].kind).toBe(3);
    expect(attrs(s[0])).toMatchObject({ [ATTR.peerService]: "github", [ATTR.exitCode]: "3", [ATTR.skillName]: "notes" });
    expect(resource(c.got[0] as Received)).toMatchObject({ "service.name": "notes", "service.namespace": "team" });
  });

  it("without an endpoint, behaves exactly like the command alone", async () => {
    const cmd = [process.execPath, "-e", "console.log('hello');process.exit(0)"];
    const bare = spawnSync(cmd[0] as string, cmd.slice(1), { encoding: "utf8" });
    const r = await runCli(["span", "--skill", "notes", "--", ...cmd], {});
    expect(r).toEqual({ code: bare.status, stdout: bare.stdout, stderr: bare.stderr });
  });

  it("exits 127 with one line when the command does not exist, and 2 on bad usage", async () => {
    const missing = await runCli(["span", "--skill", "notes", "--", "no-such-command-rubato"], {});
    expect(missing.code).toBe(127);
    expect(missing.stdout).toBe("");
    expect(missing.stderr.trim().split("\n")).toHaveLength(1);
    expect((await runCli(["span"], {})).code).toBe(2);
  });
});

describe("plan.ts check emits", () => {
  it("one command span and plan-file and system-map client spans, with stdout identical to a run without an endpoint", async () => {
    const repo = resolve(import.meta.dirname, "../../..");
    const local = join(mkdtempSync(join(tmpdir(), "rubato-otel-")), "local.yml");
    writeFileSync(
      local,
      `maps:\n  - repo: m0x6A/rubato\n    path: docs/design/rubato-map.yml\n    local: ${repo}\n    decisions: docs/decisions\nrepos:\n  - name: m0x6A/rubato\n    local: ${repo}\n`,
    );
    const plan = resolve(import.meta.dirname, "../scripts/plan.ts");
    const run = (env: Record<string, string>) =>
      new Promise<{ code: number | null; stdout: string }>((done) => {
        const child = spawn(process.execPath, ["--experimental-strip-types", "--no-warnings", plan, "check", "docs/plans/140.md", "--cwd", repo], {
          env: { PATH: process.env["PATH"] ?? "", HOME: process.env["HOME"] ?? "", RUBATO_LOCAL: local, ...env },
        });
        let stdout = "";
        child.stdout.on("data", (c) => (stdout += c));
        child.on("exit", (code) => done({ code, stdout }));
      });
    const c = await collector();
    const bare = await run({});
    const sent = await run({ RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT: c.url });
    expect(sent).toEqual(bare);
    expect(bare.code).toBe(0);
    expect(c.got).toHaveLength(1);
    const s = spans(c.got[0] as Received);
    expect(attrs(s.find((x) => x.name === SPAN.command))).toMatchObject({ [ATTR.subcommand]: "plan check", [ATTR.exitCode]: "0" });
    const peers = s.filter((x) => x.name === SPAN.call).map((x) => attrs(x)[ATTR.peerService]);
    expect(peers).toContain("plan-file");
    expect(peers).toContain("system-map");
  });
});
