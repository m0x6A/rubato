/**
 * Parse a plan Markdown file into the `Plan` shape from `rubato-protocol` (ADR 0009).
 *
 * Frontmatter carries `issue`, `title`, `spend_cap_usd`, `attempt_limit`. The tables under
 * `## Affected parts`, `## O-rows` and `## Assumed, not yet approved` are parsed by column
 * name. Output must equal what the Rust parser produces for the same file; the fixtures under
 * `schema/fixtures/plans/` hold the expected JSON for both.
 */
import { parse as parseYaml } from "yaml";

export interface IssueRef {
  repo: string;
  number: number;
}

export type PartStatus = "affected" | "new" | "quiet";
export type AssumptionScope = "node" | "seam";

export interface AffectedPart {
  node: string;
  status: PartStatus;
  change: string;
  seams?: string[];
}

export interface ORow {
  seam: string;
  breaks_like?: string;
  signal: string;
  panel: string;
  owner: string;
}

export interface Assumption {
  id: string;
  decision: string;
  chosen: string;
  alternatives?: string;
  scope: AssumptionScope;
}

/** One row of the `## Flows` table (ADR 0021). */
export interface PlanFlow {
  from: string;
  to: string;
  seam: string;
  kind: "data" | "control";
  carries: string;
}

export interface Plan {
  issue: IssueRef;
  title?: string;
  path: string;
  spend_cap_usd?: number;
  attempt_limit?: number;
  affected_parts: AffectedPart[];
  o_rows?: ORow[];
  assumed?: Assumption[];
  flows?: PlanFlow[];
}

export class PlanParseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PlanParseError";
  }
}

/** `owner/name#n` → IssueRef. */
export function parseIssueRef(text: string): IssueRef {
  const m = /^([^\s/#]+\/[^\s/#]+)#(\d+)$/.exec(text.trim());
  if (!m) {
    throw new PlanParseError(`issue must be owner/name#number, got ${JSON.stringify(text)}`);
  }
  return { repo: m[1] as string, number: Number(m[2]) };
}

interface Frontmatter {
  body: string;
  data: Record<string, unknown>;
}

function splitFrontmatter(text: string): Frontmatter {
  const lines = text.split(/\r?\n/);
  if (lines[0]?.trim() !== "---") {
    return { body: text, data: {} };
  }
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === "---");
  if (end < 0) {
    throw new PlanParseError("frontmatter is not closed");
  }
  const raw = lines.slice(1, end).join("\n");
  const data = (parseYaml(raw) ?? {}) as Record<string, unknown>;
  return { body: lines.slice(end + 1).join("\n"), data };
}

/** Split a table row on `|` outside backticks. */
function splitRow(line: string): string[] {
  const cells: string[] = [];
  let cur = "";
  let inCode = false;
  for (const ch of line) {
    if (ch === "`") {
      inCode = !inCode;
    }
    if (ch === "|" && !inCode) {
      cells.push(cur);
      cur = "";
    } else {
      cur += ch;
    }
  }
  cells.push(cur);
  // Leading and trailing pipes produce empty edge cells.
  if (cells.length && cells[0]?.trim() === "") {
    cells.shift();
  }
  if (cells.length && cells[cells.length - 1]?.trim() === "") {
    cells.pop();
  }
  return cells.map((c) => c.trim());
}

export interface Table {
  headers: string[];
  rows: string[][];
}

/** The first table after the heading matching `pattern`, or null if the section is absent. */
export function tableAfterHeading(body: string, pattern: RegExp): Table | null {
  const lines = body.split(/\r?\n/);
  const start = lines.findIndex((l) => /^#{1,6}\s/.test(l) && pattern.test(l.replace(/^#{1,6}\s*(\d+(\.\d+)*\.?\s*)?/, "")));
  if (start < 0) {
    return null;
  }
  let i = start + 1;
  while (i < lines.length && !/^#{1,6}\s/.test(lines[i] as string) && !(lines[i] as string).trimStart().startsWith("|")) {
    i++;
  }
  if (i >= lines.length || /^#{1,6}\s/.test(lines[i] as string)) {
    return null;
  }
  const tableLines: string[] = [];
  while (i < lines.length && (lines[i] as string).trimStart().startsWith("|")) {
    tableLines.push((lines[i] as string).trim());
    i++;
  }
  const [headerLine, separator, ...rest] = tableLines;
  if (!headerLine || !separator || !/^\|?\s*:?-{3,}/.test(separator)) {
    throw new PlanParseError(`table after heading ${pattern} has no header separator row`);
  }
  const headers = splitRow(headerLine).map((h) => h.toLowerCase());
  const rows = rest.map(splitRow).filter((r) => r.some((c) => c !== ""));
  return { headers, rows };
}

function column(headers: string[], ...patterns: RegExp[]): number {
  for (const p of patterns) {
    const i = headers.findIndex((h) => p.test(h));
    if (i >= 0) {
      return i;
    }
  }
  return -1;
}

function requireColumn(table: Table, section: string, name: string, ...patterns: RegExp[]): number {
  const i = column(table.headers, ...patterns);
  if (i < 0) {
    throw new PlanParseError(`${section}: missing column ${name} (headers: ${table.headers.join(", ")})`);
  }
  return i;
}

/** Strip backticks and emphasis from an id cell. */
export function cleanId(cell: string): string {
  return cell.replace(/[`*_]/g, "").trim();
}

function idList(cell: string): string[] {
  return cell
    .split(",")
    .map(cleanId)
    .filter((s) => s !== "" && s !== "—" && s !== "-");
}

function firstWord(cell: string): string {
  return cleanId(cell).split(/\s+/)[0]?.toLowerCase() ?? "";
}

export const AFFECTED_PARTS_HEADING = /^affected parts/i;
export const O_ROWS_HEADING = /^o-rows/i;
export const ASSUMED_HEADING = /^assumed, not yet approved/i;
export const FLOWS_HEADING = /^flows/i;

export function parsePlan(text: string, path: string): Plan {
  const { body, data } = splitFrontmatter(text);
  if (typeof data["issue"] !== "string") {
    throw new PlanParseError("frontmatter needs issue: owner/name#number");
  }
  const plan: Plan = {
    issue: parseIssueRef(data["issue"]),
    path,
    affected_parts: [],
  };
  if (typeof data["title"] === "string" && data["title"].trim() !== "") {
    plan.title = data["title"].trim();
  }
  if (typeof data["spend_cap_usd"] === "number") {
    plan.spend_cap_usd = data["spend_cap_usd"];
  }
  if (typeof data["attempt_limit"] === "number") {
    plan.attempt_limit = data["attempt_limit"];
  }

  const parts = tableAfterHeading(body, AFFECTED_PARTS_HEADING);
  if (!parts) {
    throw new PlanParseError("no table under an 'Affected parts' heading");
  }
  const nodeCol = requireColumn(parts, "affected parts", "node", /^node/);
  const statusCol = requireColumn(parts, "affected parts", "status", /^status/);
  const changeCol = requireColumn(parts, "affected parts", "change", /^what changes/, /^change/);
  const seamsCol = column(parts.headers, /^seam/);
  for (const row of parts.rows) {
    const part: AffectedPart = {
      node: cleanId(row[nodeCol] ?? ""),
      status: firstWord(row[statusCol] ?? "") as PartStatus,
      change: row[changeCol] ?? "",
    };
    const seams = seamsCol >= 0 ? idList(row[seamsCol] ?? "") : [];
    if (seams.length) {
      part.seams = seams;
    }
    plan.affected_parts.push(part);
  }

  const oRows = tableAfterHeading(body, O_ROWS_HEADING);
  if (oRows) {
    const seamCol = requireColumn(oRows, "o-rows", "seam", /^seam/);
    const breaksCol = column(oRows.headers, /^breaks/);
    const signalCol = requireColumn(oRows, "o-rows", "signal", /^signal/);
    const panelCol = requireColumn(oRows, "o-rows", "panel", /^panel/);
    const ownerCol = requireColumn(oRows, "o-rows", "owner", /^owner/);
    const list: ORow[] = [];
    for (const row of oRows.rows) {
      const o: ORow = {
        seam: cleanId(row[seamCol] ?? ""),
        signal: row[signalCol] ?? "",
        panel: row[panelCol] ?? "",
        owner: cleanId(row[ownerCol] ?? ""),
      };
      const breaks = breaksCol >= 0 ? (row[breaksCol] ?? "").trim() : "";
      if (breaks) {
        o.breaks_like = breaks;
      }
      list.push(o);
    }
    if (list.length) {
      plan.o_rows = list;
    }
  }

  const assumed = tableAfterHeading(body, ASSUMED_HEADING);
  if (assumed) {
    const idCol = requireColumn(assumed, "assumed", "id", /^#$/, /^id/);
    const decisionCol = requireColumn(assumed, "assumed", "decision", /^decision/);
    const chosenCol = requireColumn(assumed, "assumed", "chosen", /^chosen/);
    const altCol = column(assumed.headers, /^alternatives/);
    const scopeCol = requireColumn(assumed, "assumed", "scope", /^scope/);
    const list: Assumption[] = [];
    for (const row of assumed.rows) {
      const a: Assumption = {
        id: cleanId(row[idCol] ?? ""),
        decision: row[decisionCol] ?? "",
        chosen: row[chosenCol] ?? "",
        scope: firstWord(row[scopeCol] ?? "") as AssumptionScope,
      };
      const alt = altCol >= 0 ? (row[altCol] ?? "").trim() : "";
      if (alt) {
        a.alternatives = alt;
      }
      list.push(a);
    }
    if (list.length) {
      plan.assumed = list;
    }
  }

  const flows = tableAfterHeading(body, FLOWS_HEADING);
  if (flows) {
    const fromCol = requireColumn(flows, "flows", "from", /^from/);
    const toCol = requireColumn(flows, "flows", "to", /^to/);
    const seamCol = requireColumn(flows, "flows", "seam", /^seam/);
    const kindCol = requireColumn(flows, "flows", "kind", /^kind/);
    const carriesCol = requireColumn(flows, "flows", "carries", /^carries/);
    const list: PlanFlow[] = [];
    for (const row of flows.rows) {
      const kind = firstWord(row[kindCol] ?? "");
      if (kind !== "data" && kind !== "control") {
        throw new PlanParseError(`flows: kind must be data or control, got ${JSON.stringify(kind)}`);
      }
      list.push({ from: cleanId(row[fromCol] ?? ""), to: cleanId(row[toCol] ?? ""), seam: cleanId(row[seamCol] ?? ""), kind, carries: row[carriesCol] ?? "" });
    }
    if (list.length) {
      plan.flows = list;
    }
  }
  return plan;
}
