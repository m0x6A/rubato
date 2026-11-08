#!/usr/bin/env -S node --experimental-strip-types
/**
 * List the accepted ADRs from the map repo so a plan does not contradict one.
 *
 *   adrs.ts [--all] [--json]
 */
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { loadWorkspace, type Workspace } from "./lib/workspace.ts";

export interface Adr {
  file: string;
  number: string;
  title: string;
  status: string;
  seam: string;
  decision: string;
}

function field(text: string, name: string): string {
  const m = new RegExp(`^- \\*\\*${name}:\\*\\*\\s*(.+)$`, "m").exec(text);
  return m ? (m[1] as string).trim() : "";
}

function section(text: string, heading: string): string {
  const m = new RegExp(`^## ${heading}\\s*\\n([\\s\\S]*?)(?=^## |\\Z)`, "m").exec(text);
  return m ? (m[1] as string).trim() : "";
}

export function parseAdr(file: string, text: string): Adr | null {
  const title = /^# (\d{4})\.\s*(.+)$/m.exec(text);
  if (!title) {
    return null;
  }
  return {
    file,
    number: title[1] as string,
    title: (title[2] as string).trim(),
    status: field(text, "Status").split(/\s+/)[0] ?? "",
    seam: field(text, "Seam"),
    decision: section(text, "Decision"),
  };
}

export function listAdrs(ws: Workspace): Adr[] {
  const dir = join(ws.map.local, ws.map.decisions ?? "docs/decisions");
  const out: Adr[] = [];
  for (const f of readdirSync(dir).sort()) {
    if (!/^\d{4}-.*\.md$/.test(f) || f.startsWith("0000-")) {
      continue;
    }
    const adr = parseAdr(f, readFileSync(join(dir, f), "utf8"));
    if (adr) {
      out.push(adr);
    }
  }
  return out;
}

export function render(adrs: Adr[]): string {
  return adrs
    .map((a) => `## ${a.number}. ${a.title}\n- status: ${a.status}; seam: ${a.seam}; file: ${a.file}\n\n${a.decision}`)
    .join("\n\n");
}

async function main(argv: string[]): Promise<number> {
  try {
    let adrs = listAdrs(loadWorkspace());
    if (!argv.includes("--all")) {
      adrs = adrs.filter((a) => a.status === "accepted");
    }
    console.log(argv.includes("--json") ? JSON.stringify(adrs, null, 2) : render(adrs));
    return 0;
  } catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  process.exitCode = await main(process.argv.slice(2));
}
