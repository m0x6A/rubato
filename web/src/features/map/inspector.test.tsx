import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { MapRef, Overview } from "../../protocol.ts";
import { Inspector, PlanInspector } from "./Inspector.tsx";

const REPO = "m0x6A/rubato";
const ref = { repo: REPO, number: 48 };
const plan = {
  issue: ref,
  path: "docs/plans/48.md",
  affected_parts: [{ node: "rubato-server", status: "affected" as const, change: "adds the layout slice", seams: ["web-server"] }],
  o_rows: [{ seam: "web-server", signal: "rubato.layout.writes", panel: "Layout writes", owner: "rubato-server" }],
};
const decision = { issue: ref, comment_id: 9, title: "Positions live on the server", author: "m0x6A", created_at: new Date(Date.now() - 3_600_000).toISOString(), url: "https://example.invalid/9" };

function show(selection: MapRef, ov: Overview = overview()) {
  const onSelect = vi.fn();
  const onSelectIssue = vi.fn();
  render(<Inspector overview={ov} selection={selection} onSelect={onSelect} onSelectIssue={onSelectIssue} />);
  return { onSelect, onSelectIssue, box: screen.getByTestId("inspector") };
}

describe("the inspector explains what is selected (S9, S26)", () => {
  it("a part shows what it is, what goes in and out, and what touches it now", () => {
    const ov = overview({
      issues: [issue(48, "rubato-server", { title: "Interactive map", plan, decisions: [decision], progress: { issue: ref, state: "designed", last_seen: new Date().toISOString() } })],
    });
    const { box, onSelect, onSelectIssue } = show({ kind: "node", id: "rubato-server" }, ov);
    expect(within(box).getByRole("heading", { name: "rubato-server" })).toBeInTheDocument();
    expect(box).toHaveTextContent("One per team.");
    expect(within(box).getByRole("link", { name: /m0x6A\/rubato · crates\/rubato-server/ })).toHaveAttribute("href", "https://github.com/m0x6A/rubato/tree/HEAD/crates/rubato-server");
    // In and out, from the team's own map: the overview goes out to web, pins come in.
    const receives = within(box).getByRole("heading", { name: "Receives" }).nextElementSibling as HTMLElement;
    const sends = within(box).getByRole("heading", { name: "Sends" }).nextElementSibling as HTMLElement;
    expect(receives).toHaveTextContent("PinRequest, MapLayout, PairApproveRequest, map draft operations from web over web-server");
    expect(sends).toHaveTextContent(/Overview: map, issues.* to web over web-server/);
    expect(sends).toHaveTextContent("LaunchCommand, for the paired user only to rubato-companion over companion-server");
    // Dependencies from seam direction.
    expect(within(box).getByRole("heading", { name: "Depends on" }).nextElementSibling).toHaveTextContent("github over server-github");
    expect(within(box).getByRole("heading", { name: "Depended on by" }).nextElementSibling).toHaveTextContent("web over web-server");
    // What touches it now.
    expect(box).toHaveTextContent("m0x6A/rubato#48 Interactive map Designed");
    expect(box).toHaveTextContent("m0x6A/rubato#48 (affected): adds the layout slice");
    expect(within(box).getByRole("link", { name: "Positions live on the server" })).toHaveAttribute("href", "https://example.invalid/9");
    expect(box).toHaveTextContent(/m0x6A, .*ago/);

    // Every other part and seam named is a way to walk the map.
    fireEvent.click(within(receives).getAllByRole("button", { name: "web" })[0]!);
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "node", id: "web" });
    fireEvent.click(within(receives).getAllByRole("button", { name: "web-server" })[0]!);
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "seam", id: "web-server" });
    fireEvent.click(within(box).getAllByRole("button", { name: "m0x6A/rubato#48" })[0]!);
    expect(onSelectIssue).toHaveBeenCalledWith(ref);
    fireEvent.click(within(box).getByRole("button", { name: "Close" }));
    expect(onSelect).toHaveBeenLastCalledWith(null);
  });

  it("a seam shows carries, guard, flows, pins and plans", () => {
    const ov = overview({
      issues: [
        issue(48, "rubato-server", { title: "Interactive map", plan, progress: { issue: ref, state: "designed", last_seen: new Date().toISOString() } }),
        issue(51, "", { pins: [{ kind: "seam", id: "web-server" }], title: "Pin by drag" }),
      ],
    });
    const { box } = show({ kind: "seam", id: "web-server" }, ov);
    expect(within(box).getByRole("heading", { name: "web-server" })).toBeInTheDocument();
    expect(box).toHaveTextContent("The browser reads the whole picture in one payload");
    expect(box).toHaveTextContent("web depends on rubato-server");
    expect(box).toHaveTextContent("rubato-server is woken when it breaks");
    expect(box).toHaveTextContent("REST and websocket; TS types generated from the protocol schema");
    expect(box).toHaveTextContent("generated web/src/protocol.ts diffed in CI");
    const flows = within(box).getByRole("heading", { name: "Flows" }).nextElementSibling as HTMLElement;
    expect(within(flows).getAllByRole("listitem")).toHaveLength(4);
    expect(flows).toHaveTextContent("control WebEvent Changed over /ws/web: fetch the overview again: rubato-server → web");
    expect(box).toHaveTextContent("m0x6A/rubato#48: rubato.layout.writes on panel Layout writes, wakes rubato-server");
    expect(box).toHaveTextContent("m0x6A/rubato#51 Pin by drag Planned");
    expect(within(box).getByRole("heading", { name: "Open plans" }).nextElementSibling).toHaveTextContent("m0x6A/rubato#48");
  });

  it("a missing description or flow says not described yet", () => {
    const bare = overview();
    bare.map.map = {
      version: 1,
      name: "bare",
      nodes: [
        { id: "a", kind: "context" },
        { id: "b", kind: "external" },
      ],
      seams: [{ id: "a-b", from: "a", to: "b", owner: "a" }],
    };
    const part = show({ kind: "node", id: "a" }, bare);
    expect(part.box).toHaveTextContent("What this part is for is not described yet.");
    expect(part.box).toHaveTextContent("What crosses a-b is not described yet.");
    expect(part.box).toHaveTextContent("No issue is pinned here and no open plan names it.");
    expect(within(part.box).queryByRole("link")).toBeNull();
    cleanup();
    const seam = show({ kind: "seam", id: "a-b" }, bare);
    expect(seam.box).toHaveTextContent("What this seam is for is not described yet.");
    expect(seam.box).toHaveTextContent("What crosses this seam is not described yet.");
    expect(seam.box).toHaveTextContent("Carriesnot described yet");
    expect(seam.box).toHaveTextContent("No open plan has written an O-row for this seam.");
  });

  it("shows the selected plan's O-rows, seam assumptions and decisions with author and time", () => {
    const withFlows = {
      ...plan,
      flows: [{ from: "rubato-server", to: "web", seam: "web-server", kind: "data" as const, carries: "Overview with layout" }],
      assumed: [
        { id: "A1", decision: "Who writes the layout", chosen: "any team member", alternatives: "only the map owner", scope: "seam" as const },
        { id: "A2", decision: "Rounding", chosen: "whole pixels", scope: "node" as const },
      ],
    };
    const onSelect = vi.fn();
    render(<PlanInspector issue={issue(48, "rubato-server", { title: "Interactive map", plan: withFlows, decisions: [{ ...decision, chosen: "SQLite on the server" }] })} onSelect={onSelect} />);
    const box = screen.getByTestId("plan-inspector");
    expect(box).toHaveTextContent("docs/plans/48.md");
    expect(within(box).getByRole("heading", { name: "Changes" }).nextElementSibling).toHaveTextContent("rubato-server affected adds the layout slice");
    expect(within(box).getByRole("heading", { name: "Sends" }).nextElementSibling).toHaveTextContent("data Overview with layout: rubato-server → web over web-server");
    expect(within(box).getByRole("heading", { name: "If it breaks at 3 am" }).nextElementSibling).toHaveTextContent("web-server: rubato.layout.writes on panel Layout writes, wakes rubato-server");
    // Only the seam-scoped assumption waits for a human; the node-scoped one is the agent's to make.
    const waiting = within(box).getByRole("heading", { name: "Waiting for a human" }).nextElementSibling as HTMLElement;
    expect(waiting).toHaveTextContent("A1: Who writes the layout assumed any team member (instead of only the map owner)");
    expect(waiting).not.toHaveTextContent("Rounding");
    const decisions = within(box).getByRole("heading", { name: "Decisions" }).nextElementSibling as HTMLElement;
    expect(within(decisions).getByRole("link", { name: "Positions live on the server" })).toHaveAttribute("href", "https://example.invalid/9");
    expect(decisions).toHaveTextContent(/m0x6A, 1h ago|m0x6A, 60m ago|m0x6A, .*ago/);
    expect(decisions).toHaveTextContent("SQLite on the server");
    fireEvent.click(within(box).getAllByRole("button", { name: "web-server" })[0]!);
    expect(onSelect).toHaveBeenCalledWith({ kind: "seam", id: "web-server" });
  });

  it("a plan without flows, O-rows or decisions says what is missing", () => {
    render(<PlanInspector issue={issue(7, "web", { plan: { issue: { repo: REPO, number: 7 }, path: "docs/plans/7.md", affected_parts: [{ node: "web", status: "new", change: "a page" }] } })} onSelect={() => {}} />);
    const box = screen.getByTestId("plan-inspector");
    expect(box).toHaveTextContent("The plan has no Flows table: what it sends across seams is not described yet.");
    expect(box).toHaveTextContent("The plan has no O-rows.");
    expect(box).toHaveTextContent("No seam-level question is open.");
    expect(box).toHaveTextContent("No Decision: comment on the issue yet.");
  });

  it("a selection the map no longer has says so", () => {
    const { box } = show({ kind: "node", id: "gone" });
    expect(box).toHaveTextContent("gone is not in map abc1234.");
  });
});
