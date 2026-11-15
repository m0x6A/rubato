import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue, realMap } from "../../../test/fixtures.ts";
import type { ImportRequest, ImportResponse, IssueSummary, Me } from "../../protocol.ts";
import { WhereItFits } from "./index.ts";

afterEach(() => vi.restoreAllMocks());

const bob: Me = { login: "bob" };
const BRANCH = "issue/95-hand";
const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

const outside = (fits: boolean): IssueSummary =>
  issue(95, "web", {
    outside_rubato: true,
    plan_sha: "abc1234",
    fit: { fits, reasons: [] } as NonNullable<IssueSummary["fit"]>,
    plan: { path: "docs/plans/95.md", git_ref: BRANCH, affected_parts: [] } as unknown as NonNullable<IssueSummary["plan"]>,
  });

function serve(answer: (req: ImportRequest) => Response) {
  const posted: ImportRequest[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    if (String(input) === "/api/plans/import" && init?.method === "POST") {
      const req = JSON.parse(String(init.body)) as ImportRequest;
      posted.push(req);
      return answer(req);
    }
    return json({ error: "unknown api path" }, 404);
  });
  return posted;
}

describe("a plan written outside Rubato (ADR 0058)", () => {
  it("offers Import instead of approval, and imports a plan that fits as your draft", async () => {
    const posted = serve(() => json({ outcome: "imported", draft_id: "d1", rev: 1, original_author: "carol" } satisfies ImportResponse, 201));
    const onDone = vi.fn();
    render(<WhereItFits issue={outside(true)} map={realMap()} me={bob} onNotice={onDone} approvalsInRubato />);
    expect(screen.queryByRole("button", { name: "Accept plan" })).not.toBeInTheDocument();
    expect(screen.queryByTestId("approve-in-rubato")).not.toBeInTheDocument();
    expect(screen.getByTestId("import-plan")).toHaveTextContent("it fits: it becomes your draft");
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await screen.findByText("Imported as your draft, revision 1; written by carol.");
    expect(posted).toEqual([{ repo: "m0x6A/rubato", number: 95, branch: BRANCH }]);
    expect(onDone).toHaveBeenCalledWith("Imported as your draft, revision 1; written by carol.");
  });

  it("says a session of yours rewrites a plan that does not fit", async () => {
    serve(() => json({ outcome: "rewriting", launch_id: "l1", branch: BRANCH } satisfies ImportResponse, 202));
    render(<WhereItFits issue={outside(false)} map={realMap()} me={bob} onNotice={() => {}} approvalsInRubato />);
    expect(screen.getByTestId("import-plan")).toHaveTextContent("it does not fit: a session of yours rewrites it");
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await screen.findByText(`A session of yours is rewriting the plan into Rubato's form on ${BRANCH}.`);
  });

  it("puts a refusal in words with its code", async () => {
    serve(() => json({ reason: "launch_refused", detail: "none of your companions is connected" }, 409));
    render(<WhereItFits issue={outside(false)} map={realMap()} me={bob} onNotice={() => {}} approvalsInRubato />);
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await screen.findByText("No session could be started to rewrite it; connect a companion of yours (launch_refused)");
  });
});
