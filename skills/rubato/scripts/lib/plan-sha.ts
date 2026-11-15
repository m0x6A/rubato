/**
 * The version an approval names (ADR 0027, ADR 0054): the git blob sha of the plan file with its
 * approval record removed and every task checkbox cleared, so ticking tasks or writing the record
 * keeps an approval current and any other edit does not.
 *
 * Equal to `git hash-object --stdin` of the normalised text. The server computes the same hash
 * (`rubato_protocol::plan_hash`) and so does plan-check (`actions/plan-check/src/plan-sha.ts`);
 * `schema/fixtures/plans/approval-hash/` holds the cases all three check.
 */
import { createHash } from "node:crypto";

/** The front-matter keys of the approval record, left out of the hash. */
export const APPROVAL_KEYS = ["approved_by", "approved_at", "approved_sha"] as const;

/** A list item's checkbox, `- [x]`, `* [X]`, `+ [x]` or `1. [x]`, at any indent. */
const CHECKBOX = /^(\s*(?:[-*+]|\d+[.)])\s+)\[[xX]\]/gm;

/** Whole lines of the first front-matter block that carry an approval key. Line endings kept. */
export function stripApprovalRecord(text: string): string {
  const lines = text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  if (lines.length === 0 || lines[0]?.trim() !== "---") {
    return text;
  }
  let out = lines[0] as string;
  let inFront = true;
  for (const line of lines.slice(1)) {
    if (inFront && line.trim() === "---") {
      inFront = false;
    } else if (inFront && APPROVAL_KEYS.some((k) => line.startsWith(`${k}:`))) {
      continue;
    }
    out += line;
  }
  return out;
}

/** The plan text the hash is taken over: the approval record removed, ticked boxes cleared. */
export function normalisePlan(text: string): string {
  return stripApprovalRecord(text).replace(CHECKBOX, "$1[ ]");
}

/** Git's blob sha-1 of the given bytes: `sha1("blob <len>\0" + bytes)`. */
export function gitBlobSha(content: string | Uint8Array): string {
  const bytes = typeof content === "string" ? Buffer.from(content, "utf8") : Buffer.from(content);
  return createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

/** The sha an approval names for this plan text. */
export function planSha(text: string): string {
  return gitBlobSha(normalisePlan(text));
}
