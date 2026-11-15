import { spawn } from "node:child_process";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repo = resolve(import.meta.dirname, "../../..");
const cli = resolve(import.meta.dirname, "../src/cli.ts");

const servers: Server[] = [];
afterEach(() => {
  for (const s of servers.splice(0)) {
    s.closeAllConnections();
    s.close();
  }
});

async function collector(): Promise<{ url: string; got: any[] }> {
  const got: any[] = [];
  const server = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => {
      got.push(JSON.parse(raw));
      res.writeHead(200).end("{}");
    });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  servers.push(server);
  return { url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, got };
}

function run(env: Record<string, string>): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((done) => {
    const child = spawn(
      process.execPath,
      ["--experimental-strip-types", "--no-warnings", cli, "--map", "docs/design/rubato-map.yml", "--schema-dir", "schema", "--plans", "docs/plans/140.md"],
      { cwd: repo, env: { PATH: process.env["PATH"] ?? "", ...env } },
    );
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (c) => (stdout += c));
    child.stderr.on("data", (c) => (stderr += c));
    child.on("exit", (code) => done({ code, stdout, stderr }));
  });
}

describe("plan-check telemetry (ADR 0061)", () => {
  it("sends one export with a plan-check command span when an endpoint is set, and prints exactly the same", async () => {
    const c = await collector();
    const bare = await run({});
    const sent = await run({ OTEL_EXPORTER_OTLP_ENDPOINT: c.url, OTEL_RESOURCE_ATTRIBUTES: "service.namespace=rubato" });
    expect(sent).toEqual(bare);
    expect(bare.code).toBe(0);
    expect(c.got).toHaveLength(1);
    const rs = c.got[0].resourceSpans[0];
    const res = Object.fromEntries(rs.resource.attributes.map((a: any) => [a.key, a.value.stringValue]));
    expect(res).toMatchObject({ "service.name": "plan-check", "service.namespace": "rubato" });
    const names = rs.scopeSpans[0].spans.map((s: any) => s.name);
    expect(names.filter((n: string) => n === "rubato.skill.command")).toHaveLength(1);
    const peers = rs.scopeSpans[0].spans
      .flatMap((s: any) => s.attributes)
      .filter((a: any) => a.key === "peer.service")
      .map((a: any) => a.value.stringValue);
    expect(peers).toEqual(expect.arrayContaining(["system-map", "plan-file"]));
  });
});
