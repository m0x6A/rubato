/**
 * The version an approval names (ADR 0027): the git blob sha of the plan file with every task
 * checkbox cleared, so ticking tasks keeps an approval current and any other edit does not.
 *
 * Equal to `git hash-object --stdin` of the normalised text. The server computes the same hash
 * (`schema/fixtures/plans/approval-hash/` holds the cases both sides check).
 */
import { createHash } from "node:crypto";

/** A list item's checkbox, `- [x]`, `* [X]`, `+ [x]` or `1. [x]`, at any indent. */
const CHECKBOX = /^(\s*(?:[-*+]|\d+[.)])\s+)\[[xX]\]/gm;

/** The plan text with every ticked checkbox cleared (`[x]` → `[ ]`). Nothing else changes. */
export function normalisePlan(text: string): string {
  return text.replace(CHECKBOX, "$1[ ]");
}

/** Git's blob sha-1 of the given bytes: `sha1("blob <len>\0" + bytes)`. */
export function gitBlobSha(content: string | Uint8Array): string {
  const bytes = typeof content === "string" ? Buffer.from(content, "utf8") : Buffer.from(content);
  return createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

/** The sha an `Approved: plan <sha>` comment names for this plan text. */
export function planSha(text: string): string {
  return gitBlobSha(normalisePlan(text));
}
