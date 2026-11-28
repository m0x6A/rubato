import type { IssueSummary, Me, Overview, SessionState } from "../../protocol.ts";
import { isFinished } from "../../shared/finished.ts";
import type { IssueSection } from "../../shared/route.ts";
import { ownConnectedCompanions, ownLanes } from "./own.ts";

/**
 * The one next step an issue's header offers (issue #357, design §4). Derived, like progress:
 * from whether the issue is finished, its pull request, a plan waiting for a person, its sessions
 * (whose, and alive or not) and the viewer's companions. The first rule that holds wins, so the
 * header never offers two things.
 */
export type Next =
  /** Nothing for this person to do here; `says` is why, or who it waits for. */
  | { kind: "none"; says: string | null }
  /** Go to a section of the issue, where that section's own primary action is. */
  | { kind: "section"; section: IssueSection; label: string; says: string | null }
  | { kind: "pin"; says: string }
  | { kind: "pair"; says: string }
  | { kind: "resume"; branch: string; says: string }
  | { kind: "start"; says: string | null };

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** A session someone may still be at: working, waiting for input, or blocked. Ended and unknown are not. */
const alive = (s: SessionState) => s.status === "working" || s.status === "idle" || s.status === "blocked";

export function nextAction(issue: IssueSummary, overview: Overview, me: Me | null): Next {
  if (isFinished(issue)) {
    return { kind: "none", says: null };
  }
  const pr = issue.pull_request;
  if (pr?.open) {
    return { kind: "section", section: "pr", label: "Review pull request", says: `pull request #${pr.number} is open · checks ${pr.checks}` };
  }
  // A plan waits for a person once its agent has reviewed it and no approval covers it as it is.
  const approved = issue.approval?.version === "current";
  if (!approved && issue.review && (issue.draft || issue.plan)) {
    return { kind: "section", section: "plan", label: "Review plan", says: issue.draft ? `revision ${issue.draft.rev} waits for approval` : "the plan waits for approval" };
  }
  const seamQuestions = (issue.plan?.assumed ?? []).filter((a) => a.scope === "seam").length;
  if (issue.progress.state === "blocked" && seamQuestions > 0) {
    return { kind: "section", section: "room", label: "Answer in the room", says: `${seamQuestions} question${seamQuestions === 1 ? "" : "s"} at a seam wait for a person` };
  }
  const sessions = (issue.sessions ?? []).filter(alive);
  const mine = me ? sessions.find((s) => same(s.user, me.login)) : undefined;
  if (mine) {
    return { kind: "section", section: "sessions", label: "Open session", says: mine.status === "blocked" ? (issue.progress.reason ?? "your session is blocked") : mine.status === "idle" ? "your session waits for you" : "your session is working" };
  }
  const theirs = sessions[0];
  if (theirs) {
    return { kind: "none", says: `${theirs.user}'s session is ${theirs.status === "idle" ? "waiting" : theirs.status}` };
  }
  if (issue.pins.length === 0) {
    return { kind: "pin", says: "not on the map yet" };
  }
  if (ownLanes(overview, me).length === 0) {
    return { kind: "pair", says: "a companion on your machine starts sessions" };
  }
  const connected = ownConnectedCompanions(overview, me);
  if (connected.length === 0) {
    const host = ownLanes(overview, me)[0]?.hostname;
    return { kind: "none", says: `your companion${host ? ` on ${host}` : ""} is not connected: run rubato-companion run there` };
  }
  const draft = issue.draft;
  if (draft && !draft.live && me && same(draft.owner, me.login)) {
    return { kind: "resume", branch: draft.branch, says: `revision ${draft.rev} is parked` };
  }
  return { kind: "start", says: null };
}
