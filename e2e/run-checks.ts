/**
 * The checks e2e/run.ts makes, each with the scenarios of features/ it covers (issue #198,
 * ADR 0069). run.ts records every problem against the check it belongs to and writes the results
 * to e2e/results/run.json; `pnpm scenarios` reads this list to know what run.ts tests without
 * running it.
 */

export const RUN_CHECKS = [
  { key: "pairing", title: "the companion pairs with the code its owner approves", scenarios: ["@companions.pair-with-code"] },
  { key: "progress", title: "each issue goes from Planned to Designed", scenarios: ["@progress.planned-to-designed"] },
  { key: "live-states", title: "the issues are seen Designing or Building while their sessions work", scenarios: ["@progress.live-states"] },
  { key: "hooks", title: "each launched session runs in its worktree with the injected hooks, to its Stop hook", scenarios: ["@launch.session-runs-with-hooks"] },
  { key: "spend", title: "each issue shows spend from its session's transcript", scenarios: ["@progress.spend-from-transcript"] },
  { key: "plans", title: "each issue's plan comes back and passes the plan check", scenarios: ["@plans.plan-comes-back"] },
  { key: "collision", title: "the two plans on auth-contract-pantry show as a seam collision", scenarios: ["@findings.seam-collision"] },
  { key: "session-telemetry", title: "each session runs with telemetry on and the rubato.* resource attributes", scenarios: ["@telemetry.session-attributes"] },
  { key: "otlp", title: "server and companion export the expected metrics, attribute keys and spans", scenarios: ["@telemetry.both-binaries-export"] },
  { key: "trace-link", title: "an envelope's send on the companion and its handling on the server are one trace", scenarios: ["@telemetry.companion-server-one-trace"] },
  { key: "otlp-forbidden", title: "the companion exports no sessions-by-state metric and no session id", scenarios: ["@telemetry.no-per-session-metrics"] },
] as const;

export type CheckKey = (typeof RUN_CHECKS)[number]["key"];

export type RunResults = {
  suite: "run";
  tests: { key: string; title: string; file: string; scenarios: string[]; status: "passed" | "failed"; checked: number; problems: string[] }[];
};

/** Problems per check, and how many things each check the run reached examined. */
export class CheckLog {
  private readonly problems = new Map<CheckKey, string[]>();
  private readonly reached = new Map<CheckKey, number>();
  private readonly ordered: string[] = [];

  problem(key: CheckKey, msg: string): void {
    this.problems.set(key, [...(this.problems.get(key) ?? []), msg]);
    this.ordered.push(msg);
  }

  /** The run made this check over `checked` things (issues, logs, plans, names). */
  reach(key: CheckKey, checked: number): void {
    this.reached.set(key, (this.reached.get(key) ?? 0) + checked);
  }

  /** Every problem, in the order the run found them, for the summary run.ts prints. */
  all(): string[] {
    return [...this.ordered];
  }

  /** A check passed when the run reached it, it examined something, and it has no problem; `stopped` says why the run ended early. */
  results(stopped?: string): RunResults {
    return {
      suite: "run",
      tests: RUN_CHECKS.map((c) => {
        const problems = [...(this.problems.get(c.key) ?? [])];
        const checked = this.reached.get(c.key);
        if (checked === undefined) {
          problems.push(`the run stopped before this check${stopped ? `: ${stopped}` : ""}`);
        } else if (checked === 0) {
          problems.push("the check examined nothing");
        }
        return { key: c.key, title: c.title, file: "e2e/run.ts", scenarios: [...c.scenarios], status: problems.length ? "failed" : "passed", checked: checked ?? 0, problems };
      }),
    };
  }
}
