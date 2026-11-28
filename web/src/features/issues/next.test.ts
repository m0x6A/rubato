import { describe, expect, it } from "vitest";
import { companion, issue, overview } from "../../../test/fixtures.ts";
import type { IssueSummary, SessionState } from "../../protocol.ts";
import { type Next, nextAction } from "./next.ts";

const me = { login: "m0x6A" };
const REF = { repo: "m0x6A/rubato", number: 7 };
const now = () => new Date().toISOString();

const session = (user: string, status: SessionState["status"]): SessionState => ({ companion_id: "c1", cwd: "/w", last_seen: now(), source: "hook", status, user });
const draft = (owner: string, live: boolean) => ({ at: now(), branch: "issue/7-x", bundle_hash: "b", draft_id: "d", live, owner, plan_sha: "p", rev: 3 });
const review = { at: now(), author: "m0x6A", track: "small", outcome: "passed" } as unknown as NonNullable<IssueSummary["review"]>;
const approval = (version: string) => ({ version }) as unknown as NonNullable<IssueSummary["approval"]>;
const plan = (assumed: { scope: string }[] = []) => ({ path: "docs/plans/7.md", assumed }) as unknown as NonNullable<IssueSummary["plan"]>;
const pr = (open: boolean) => ({ repo: REF.repo, number: 70, url: "u", open, checks: "green", merged_at: null }) as unknown as NonNullable<IssueSummary["pull_request"]>;
const at = (state: IssueSummary["progress"]["state"], reason?: string) => ({ issue: REF, state, last_seen: now(), ...(reason ? { reason } : {}) });

const mine = [companion("m0x6A", true)];
const mineOff = [companion("m0x6A", false)];

/** One row of the design's table: what holds, and what the header offers. */
type Case = { name: string; issue: Partial<IssueSummary>; companions?: ReturnType<typeof companion>[]; signedOut?: boolean; want: Partial<Next> & { kind: Next["kind"] } };

const CASES: Case[] = [
  { name: "finished: nothing starts from here", issue: { finished: true, closed: true }, companions: mine, want: { kind: "none", says: null } },
  { name: "a pull request is open", issue: { pull_request: pr(true), sessions: [session("m0x6A", "working")] }, companions: mine, want: { kind: "section", section: "pr", label: "Review pull request" } },
  { name: "a reviewed draft with no approval waits for a person, even while its session works", issue: { draft: draft("m0x6A", true), review, sessions: [session("m0x6A", "working")] }, companions: mine, want: { kind: "section", section: "plan", label: "Review plan", says: "revision 3 waits for approval" } },
  { name: "a reviewed plan on the branch whose approval is stale waits again", issue: { plan: plan(), review, approval: approval("earlier") }, companions: mine, want: { kind: "section", section: "plan", label: "Review plan", says: "the plan waits for approval" } },
  { name: "a draft the agent has not reviewed yet is still the session's", issue: { draft: draft("m0x6A", true), sessions: [session("m0x6A", "working")] }, companions: mine, want: { kind: "section", section: "sessions", label: "Open session", says: "your session is working" } },
  { name: "blocked on a question at a seam", issue: { progress: at("blocked"), plan: plan([{ scope: "seam" }, { scope: "node" }]), approval: approval("current"), review }, companions: mine, want: { kind: "section", section: "room", label: "Answer in the room", says: "1 question at a seam wait for a person" } },
  { name: "your session is blocked: its reason shows", issue: { progress: at("blocked", "spend cap hit"), sessions: [session("m0x6A", "blocked")] }, companions: mine, want: { kind: "section", section: "sessions", says: "spend cap hit" } },
  { name: "your session waits for you", issue: { sessions: [session("M0X6A", "idle")] }, companions: mine, want: { kind: "section", section: "sessions", says: "your session waits for you" } },
  { name: "a teammate's session is at it: nothing for you", issue: { sessions: [session("alice", "working")] }, companions: mine, want: { kind: "none", says: "alice's session is working" } },
  { name: "an ended session does not hold the issue", issue: { sessions: [session("alice", "ended")] }, companions: mine, want: { kind: "start" } },
  { name: "not on the map yet", issue: { pins: [] }, companions: mine, want: { kind: "pin" } },
  { name: "no companion paired", issue: {}, companions: [companion("alice", true)], want: { kind: "pair" } },
  { name: "signed out: no companion is yours", issue: {}, companions: mine, signedOut: true, want: { kind: "pair" } },
  { name: "paired, not connected: nothing in the browser starts it", issue: {}, companions: mineOff, want: { kind: "none", says: "your companion on m0x6A-laptop is not connected: run rubato-companion run there" } },
  { name: "your parked draft resumes on its branch", issue: { draft: draft("m0x6A", false) }, companions: mine, want: { kind: "resume", branch: "issue/7-x" } },
  { name: "a teammate's parked draft does not: you start your own", issue: { draft: draft("alice", false) }, companions: mine, want: { kind: "start" } },
  { name: "pinned, with a companion connected", issue: {}, companions: mine, want: { kind: "start", says: null } },
  { name: "approved and nobody at it: start, to build", issue: { plan: plan(), review, approval: approval("current") }, companions: mine, want: { kind: "start" } },
];

describe("the one next step of an issue (issue #357)", () => {
  it("each row of the table gives exactly its action", () => {
    for (const c of CASES) {
      const i = issue(7, "web", c.issue);
      const got = nextAction(i, overview({ issues: [i], companions: c.companions ?? [] }), c.signedOut ? null : me);
      expect(got, c.name).toMatchObject(c.want);
    }
    console.log(`checked ${CASES.length} cases of the next-action table; kinds: ${[...new Set(CASES.map((c) => c.want.kind))].sort().join(", ")}`);
    // Every kind the header can offer is exercised.
    expect(new Set(CASES.map((c) => c.want.kind)).size).toBe(6);
  });

  it("must fail: order matters, an open pull request wins over a working session", () => {
    const i = issue(7, "web", { pull_request: pr(true), sessions: [session("m0x6A", "working")] });
    const got = nextAction(i, overview({ issues: [i], companions: mine }), me);
    // A rule list in another order would offer "Open session" here.
    expect(got).not.toMatchObject({ kind: "section", section: "sessions" });
  });
});
