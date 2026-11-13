/**
 * How the skill reads an issue's `Approved:`, `Review:` and `Decision:` comments, the same way
 * the server does (ADR 0027, 0035). Pure: no git, no gh.
 *
 * Mirrors, rule by rule:
 * - `ApprovalRecord::parse_body` and `sha_token` in `crates/rubato-protocol/src/github.rs`;
 * - `trusted_author` in the same file, with Bot from `user.type` (`kernel/github/api.rs`);
 * - prefix matching in `crates/rubato-server/src/plans/hash.rs`;
 * - the newest trusted approval counts (`overview/build.rs`).
 * The shared fixtures in `schema/fixtures/comments/` keep the two readings equal.
 */

/** A comment as `gh api repos/{o}/{r}/issues/{n}/comments` returns it (the fields we read). */
export interface GhComment {
  id: number;
  user: { login: string; type: string };
  author_association: string;
  body: string;
  created_at: string;
  updated_at: string;
  html_url: string;
}

export const APPROVED = "Approved:";

/** A word that reads as a sha: 7 to 40 hex digits once backticks and trailing punctuation go. */
export function shaToken(word: string): string | null {
  const w = word.replace(/^[`.,]+|[`.,]+$/g, "");
  return w.length >= 7 && w.length <= 40 && /^[0-9a-fA-F]+$/.test(w) ? w.toLowerCase() : null;
}

/** The sha and branch an approval names, or null when the first line is not an approval. */
export function parseApproval(body: string): { sha: string | null; branch: string | null } | null {
  const first = (body.split("\n")[0] ?? "").trim();
  if (!first.startsWith(APPROVED)) {
    return null;
  }
  const words = first.slice(APPROVED.length).trim().split(/\s+/).filter(Boolean);
  let sha: string | null = null;
  let branch: string | null = null;
  for (let i = 0; i < words.length; i++) {
    if (words[i] === "plan" && i + 1 < words.length) {
      sha = shaToken(words[i + 1] as string);
      i++;
    } else if (words[i] === "on" && i + 1 < words.length) {
      branch = (words[i + 1] as string).replace(/^`+|`+$/g, "");
      i++;
    }
  }
  return { sha, branch };
}

/** Someone Rubato trusts to approve: not a Bot, and OWNER, MEMBER or COLLABORATOR. */
export function trustedAuthor(association: string, isBot: boolean): boolean {
  return !isBot && (association === "OWNER" || association === "MEMBER" || association === "COLLABORATOR");
}

export function isTrusted(c: GhComment): boolean {
  return trustedAuthor(c.author_association, c.user.type === "Bot");
}

/** Whether an approval's sha token names `full`: the full sha starts with it. */
export function shaMatches(token: string | null, full: string): boolean {
  return token !== null && full.toLowerCase().startsWith(token);
}

/** GitHub keeps `updated_at` equal to `created_at` until a comment is edited. */
export function isEdited(c: GhComment): boolean {
  return c.updated_at !== c.created_at;
}

/** The newest approval from a trusted author, edited or not: the one the server counts. */
export function currentApproval(comments: GhComment[]): GhComment | null {
  const approvals = comments.filter((c) => isTrusted(c) && parseApproval(c.body) !== null);
  approvals.sort((a, b) => (a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : a.id - b.id));
  return approvals.at(-1) ?? null;
}

export type CommentEvent = "approved" | "approved_earlier" | "untrusted" | "edited" | "review" | "decision";

/** What one comment means to a session waiting on the plan at `sha`; null when nothing. */
export function classify(c: GhComment, sha: string): CommentEvent | null {
  const first = (c.body.split("\n")[0] ?? "").trim();
  const approval = parseApproval(c.body);
  if (approval) {
    if (!isTrusted(c)) {
      return "untrusted";
    }
    if (isEdited(c)) {
      return "edited";
    }
    return shaMatches(approval.sha, sha) ? "approved" : "approved_earlier";
  }
  if (first.startsWith("Review:") && isTrusted(c)) {
    return "review";
  }
  if (first.startsWith("Decision:") && isTrusted(c)) {
    return "decision";
  }
  return null;
}

/**
 * Whether the issue's comments approve the plan at `sha` for acceptance (ADR 0035): the newest
 * trusted approval names it and was never edited. Returns that comment, or why not.
 */
export function approvalFor(comments: GhComment[], sha: string): { ok: true; comment: GhComment } | { ok: false; reason: string } {
  const c = currentApproval(comments);
  if (!c) {
    return { ok: false, reason: "no_approval" };
  }
  if (isEdited(c)) {
    return { ok: false, reason: "edited" };
  }
  if (!shaMatches(parseApproval(c.body)?.sha ?? null, sha)) {
    return { ok: false, reason: "earlier_sha" };
  }
  return { ok: true, comment: c };
}
