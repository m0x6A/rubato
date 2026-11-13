import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { issue } from "../../../test/fixtures.ts";
import type { ProgressState } from "../../protocol.ts";
import { ProgressRing, RAIL, SpendGauge, StageRail, TeamTrack } from "./visuals.tsx";

// Issue #81, design §6: each surface opens with a picture of its state.

const ALL: ProgressState[] = ["planned", "started", "designing", "designed", "building", "blocked", "in_review", "done", "unknown"];

describe("the progress ring", () => {
  it("maps all nine states: a share of the rail, blocked a red gap, unknown dashed", () => {
    const seen: Record<string, string> = {};
    for (const state of ALL) {
      const { container, unmount } = render(<ProgressRing state={state} />);
      const svg = container.querySelector("svg.ring")!;
      expect(svg.getAttribute("aria-label")).toBeTruthy();
      seen[state] = `${svg.getAttribute("data-share")}:${[...svg.classList].filter((c) => c !== "ring").join(".")}`;
      unmount();
    }
    console.log(`checked ${ALL.length} progress states`);
    expect(seen).toEqual({
      planned: "1/7:planned",
      started: "2/7:started",
      designing: "3/7:designing",
      designed: "4/7:designed",
      building: "5/7:building",
      blocked: "gap:blocked",
      in_review: "6/7:in_review",
      done: "7/7:done",
      unknown: "dashed:unknown",
    });
  });
});

describe("the stage rail", () => {
  it("lights the current station of seven and fills the ones before it", () => {
    render(<StageRail state="building" />);
    const rail = screen.getByTestId("stage-rail");
    const stations = [...rail.querySelectorAll(".station")];
    expect(stations.map((s) => s.textContent)).toEqual(RAIL.map((r) => r.label));
    expect(stations.map((s) => s.className.replace("station", "").trim())).toEqual(["past", "past", "past", "past", "now", "", ""]);
    expect(rail).toHaveAttribute("aria-label", "Progress: Building, station 5 of 7");
  });

  it("puts blocked as a notch, not a station, and unknown lights nothing", () => {
    const { unmount } = render(<StageRail state="blocked" />);
    expect(screen.getByTestId("stage-rail").querySelectorAll(".station.now")).toHaveLength(0);
    expect(screen.getByTestId("stage-rail").querySelector(".notch")).not.toBeNull();
    unmount();
    render(<StageRail state="unknown" />);
    expect(screen.getByTestId("stage-rail").querySelectorAll(".station.now, .station.past")).toHaveLength(0);
  });
});

describe("the spend gauge", () => {
  it("is calm below 80 %, amber from 80 %, red at the cap", () => {
    const tone = (spend: number) => {
      const { container, unmount } = render(<SpendGauge spend={spend} cap={60} />);
      const t = container.querySelector(".gauge")!.getAttribute("data-tone");
      unmount();
      return t;
    };
    expect([tone(18.4), tone(47.99), tone(48), tone(59), tone(60), tone(75)]).toEqual(["calm", "calm", "near", "near", "over", "over"]);
  });

  it("is absent without a cap, and says the numbers in words", () => {
    const { container, unmount } = render(<SpendGauge spend={4} cap={null} />);
    expect(container).toBeEmptyDOMElement();
    unmount();
    render(<SpendGauge spend={18.4} cap={60} />);
    expect(screen.getByRole("img")).toHaveAttribute("aria-label", "spend $18.40 of a $60.00 cap");
  });
});

describe("the team track", () => {
  it("has one segment per state present, adding up to the issue count", () => {
    const issues = [issue(1, "web"), issue(2, "web"), issue(3, "web", { progress: { issue: { repo: "m0x6A/rubato", number: 3 }, state: "done", last_seen: new Date().toISOString() } })];
    render(<TeamTrack issues={issues} />);
    const track = screen.getByTestId("team-track");
    const segments = [...track.querySelectorAll(".segment")] as HTMLElement[];
    expect(segments.map((s) => [s.dataset["state"], s.style.flexGrow])).toEqual([
      ["planned", "2"],
      ["done", "1"],
    ]);
    expect(segments.reduce((n, s) => n + Number(s.style.flexGrow), 0)).toBe(issues.length);
    expect(track).toHaveTextContent("3 issues");
  });
});
