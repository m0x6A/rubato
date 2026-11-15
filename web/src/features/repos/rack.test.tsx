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

describe("what a repo provides, on its rack unit (issue #89)", () => {
  it("lists parts with their descriptions, seams owned and used, and journeys", () => {
    const provides = {
      parts: [{ id: "auth-api", kind: "context" as const, repo: "acme/auth", description: "Issues and checks tokens" }],
      owns: [{ id: "shop-auth", from: "shop-web", to: "auth-api", owner: "auth-api" }],
      uses: [{ id: "auth-db", from: "auth-api", to: "db", owner: "db" }],
      scenarios: [{ id: "buy", name: "Buy something", steps: [] }],
    };
    render(<RackUnit repo={repo("acme/auth", {})} max={rackScale([])} provides={provides} />);
    const unit = screen.getByTestId("provides-acme/auth");
    expect(unit).toHaveTextContent("1 part · owns 1 seam · uses 1 · in 1 journey");
    expect(unit).toHaveTextContent("auth-api — Issues and checks tokens");
    expect(unit).toHaveTextContent("Owns shop-auth");
    expect(unit).toHaveTextContent("Uses auth-db");
    expect(unit).toHaveTextContent("Buy something");
  });

  it("a repo outside the map says so instead of an empty list", () => {
    render(<RackUnit repo={repo("acme/loose", { in_map: false })} max={rackScale([])} provides={{ parts: [], owns: [], uses: [], scenarios: [] }} />);
    expect(screen.getByTestId("provides-acme/loose")).toHaveTextContent("No part of the map lives in this repo.");
  });
});
