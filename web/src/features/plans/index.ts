/** Plans: the board of every plan Rubato can read, proposed and open, with the finished ones on
 * request (#265); where each fits the system, and *Accept plan* as yourself (issue #80, ADRs
 * 0029–0031); an issue's plan draft in Rubato, its revisions, what changed since you last looked
 * (ADR 0053), and its text as Markdown (ADR 0079). */
export { DraftSection } from "./DraftSection.tsx";
export { PlanFileNote, PlansBoard } from "./PlansBoard.tsx";
export { WhereItFits } from "./WhereItFits.tsx";
export { groupPlans } from "./group.ts";
