import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ConnectedRepo } from "../../protocol.ts";
import { RackUnit, rackScale } from "./ConnectedRepos.tsx";

const repo = (name: string, extra: Partial<ConnectedRepo>): ConnectedRepo => ({ repo: name, in_map: true, covered: true, open_issues: 0, plans: 0, sessions: 0, unattributed: 0, ...extra });

describe("a repo as a rack unit (issue #81)", () => {
  const repos = [repo("a/one", { open_issues: 12, plans: 5, sessions: 3, open_pulls: 4 }), repo("a/two", { open_issues: 3, plans: 0, sessions: 1, covered: false, unattributed: 1 })];
  const max = rackScale(repos);

  it("scales every bar to the team's largest count of its kind", () => {
    render(<RackUnit repo={repos[1]!} max={max} />);
    const unit = screen.getByTestId("repo-a/two");
    const shares = [...unit.querySelectorAll(".rack-bar b")].map((b) => b.getAttribute("data-share"));
    expect(shares).toEqual(["0.25", "0.00", "0.33", "0.00"]);
    expect(unit).toHaveTextContent("app: not covered");
    expect(unit).toHaveTextContent("1 not tied to an issue");
  });

  it("the largest repo fills its bars, and a team of zeros draws empty bars, not NaN", () => {
    render(<RackUnit repo={repos[0]!} max={max} />);
    expect([...screen.getByTestId("repo-a/one").querySelectorAll(".rack-bar b")].map((b) => b.getAttribute("data-share"))).toEqual(["1.00", "1.00", "1.00", "1.00"]);
    const empty = rackScale([repo("a/zero", {})]);
    expect(Object.values(empty)).toEqual([1, 1, 1, 1]);
  });
});
