import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { IssueSummary, Me, Plan } from "../../protocol.ts";
import { buildGraph } from "../map/index.ts";
import { ConnectedRepos } from "../repos/index.ts";
import { PlansPanel, WhereItFits, groupPlans } from "./index.ts";

const ref = (n: number) => ({ repo: "m0x6A/rubato", number: n });
const now = new Date().toISOString();

const plan: Plan = {
  issue: ref(80),
  path: "docs/plans/80.md",
  git_ref: "issue/80-plans-in-view",
  affected_parts: [
    { node: "rubato-server", status: "affected", change: "a new slice", seams: ["web-server"] },
    { node: "web", status: "affected", change: "the Plans tab", seams: ["web-server"] },
  ],
  o_rows: [{ seam: "web-server", signal: "s", panel: "p", owner: "rubato-server" }],
  data_models: [
    { model: "user_tokens", kind: "table", part: "rubato-server", change: "new" },
    { model: "PlanFit", kind: "type", part: "rubato-protocol", change: "new", crosses: "web-server" },
  ],
};

function planned(n: number, extra: Partial<IssueSummary> = {}): IssueSummary {
  return issue(n, "web", { plan: { ...plan, issue: ref(n) }, plan_sha: "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9", fit: { fits: true }, ...extra });
}

const me: Me = { login: "m0x6A", can_act_on_github: true };

afterEach(() => vi.restoreAllMocks());

describe("plans (issue #80)", () => {
  it("groups plans: proposed without a counted approval, open with one, merged neither", () => {
    const approval = (version: "current" | "does_not_fit") => ({ record: { issue: ref(2), comment_id: 1, author: "m0x6A", sha: "f3c5e84", created_at: now, url: "u" }, version });
    const ov = overview({
      issues: [
        planned(1),
        planned(2, { approval: approval("current") }),
        planned(3, { approval: approval("does_not_fit"), fit: { fits: false, reasons: [{ kind: "missing_o_row", seam: "web-server" }] } }),
        planned(4, { approval: approval("current"), pull_request: { repo: "m0x6A/rubato", number: 9, branch: "b", open: false, draft: false, checks: "green", merged_at: now, updated_at: now, url: "u" } }),
        issue(5, "web"),
      ],
    });
    const { proposed, open } = groupPlans(ov);
    expect(proposed.map((i) => i.issue.number)).toEqual([1, 3]);
    expect(open.map((i) => i.issue.number)).toEqual([2]);
  });

  it("shows where a plan fits: the map strip, the parts, the data models by part", () => {
    const i = planned(80);
    render(<WhereItFits issue={i} map={overview().map.map} me={me} onNotice={() => {}} />);
    expect(screen.getByTestId("fit-verdict")).toHaveTextContent("fits the map");
    const strip = screen.getByTestId("system-strip");
    expect(within(strip).getByText("web")).toHaveClass("affected");
    expect(within(strip).getByText("github")).toHaveClass("untouched");
    expect(within(strip).getByText("⇄ web-server")).toBeInTheDocument();
    expect(screen.getByTestId("parts-affected")).toHaveTextContent("the Plans tab");
    const models = screen.getByTestId("data-models");
    expect(models).toHaveTextContent("rubato-server");
    expect(models).toHaveTextContent("user_tokens");
    expect(models).toHaveTextContent("crosses web-server");
  });

  it("says why a plan does not fit, and Accept is disabled with the reason", () => {
    const i = planned(81, { fit: { fits: false, reasons: [{ kind: "unknown_ids", ids: ["web-nowhere"] }, { kind: "model_off_part", model: "x", part: "github" }] } });
    render(<WhereItFits issue={i} map={overview().map.map} me={me} onNotice={() => {}} />);
    expect(screen.getByTestId("fit-reasons")).toHaveTextContent("names ids the map does not have: web-nowhere");
    expect(screen.getByTestId("fit-reasons")).toHaveTextContent("data model x sits on github");
    expect(screen.getByRole("button", { name: "Accept plan" })).toBeDisabled();
    expect(screen.getByTestId("accept-why")).toHaveTextContent("does not fit the map");
  });

  it("tells a person Rubato cannot comment as them, and why", () => {
    render(<WhereItFits issue={planned(82)} map={overview().map.map} me={{ login: "m0x6A", can_act_on_github: false, cannot_act_reason: "log_in_again" }} onNotice={() => {}} />);
    expect(screen.getByRole("button", { name: "Accept plan" })).toBeDisabled();
    expect(screen.getByTestId("accept-why")).toHaveTextContent("Log in again");
  });

  it("accepts as you: posts the plan's sha once and says so", async () => {
    const fetch = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({ state: "posted", body: "Approved: plan f3c5e84f2b on issue/80-plans-in-view" }), { status: 200 }));
    const said: string[] = [];
    render(<PlansPanel overview={overview({ issues: [planned(80)] })} me={me} onSelect={() => {}} onNotice={(t) => said.push(t)} />);
    fireEvent.click(within(screen.getByTestId("plan-row-80")).getByRole("button", { name: "Accept plan" }));
    await waitFor(() => expect(said).toHaveLength(1));
    expect(fetch).toHaveBeenCalledTimes(1);
    const [url, init] = fetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/plans/approve");
    expect(JSON.parse(init.body as string)).toEqual({ issue: ref(80), sha: "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9" });
    expect(said[0]).toContain("posted");
  });

  it("explains a refusal in words", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({ reason: "plan_changed" }), { status: 409 }));
    const said: string[] = [];
    render(<PlansPanel overview={overview({ issues: [planned(80)] })} me={me} onSelect={() => {}} onNotice={(t) => said.push(t)} />);
    fireEvent.click(screen.getByRole("button", { name: "Accept plan" }));
    await waitFor(() => expect(said[0]).toBe("The plan changed since you saw it. Look again, then accept."));
  });

  it("puts an unpinned issue on the parts its plan names, with its models", () => {
    const i = planned(83, { pins: [] });
    const g = buildGraph(overview({ issues: [i] }));
    expect(g.nodes.find((n) => n.id === "web")?.planned.map((p) => p.issue.number)).toEqual([83]);
    expect(g.nodes.find((n) => n.id === "rubato-server")?.models).toBe(1);
    expect(g.nodes.find((n) => n.id === "github")?.planned).toEqual([]);
  });
});

describe("connected repos (issue #80)", () => {
  it("shows each repo's coverage and counts, and sessions not tied to an issue", () => {
    const ov = overview({
      repos: [
        { repo: "m0x6A/rubato", in_map: true, covered: true, open_issues: 5, plans: 2, sessions: 1, unattributed: 1 },
        { repo: "acme/other", in_map: false, covered: false, open_issues: 0, plans: 1, sessions: 0, unattributed: 0 },
      ],
      unpinned_sessions: [{ companion_id: "00000000-0000-0000-0000-000000000000", user: "m0x6A", cwd: "/w", branch: "main", repo: "m0x6A/rubato", status: "working", source: "hook", last_seen: now }],
    });
    render(<ConnectedRepos overview={ov} />);
    const row = screen.getByTestId("repo-m0x6A/rubato");
    expect(row).toHaveTextContent("covered");
    expect(row).toHaveTextContent("5");
    expect(screen.getByTestId("repo-acme/other")).toHaveTextContent("not covered");
    expect(screen.getByTestId("unattributed")).toHaveTextContent("main");
  });
});
