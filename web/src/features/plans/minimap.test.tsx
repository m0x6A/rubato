import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { realMap } from "../../../test/fixtures.ts";
import type { Plan } from "../../protocol.ts";
import { MiniMap, layered, miniPositions } from "./MiniMap.tsx";

const map = realMap();
const plan: Plan = {
  issue: { repo: "m0x6A/rubato", number: 82 },
  path: "docs/plans/82.md",
  affected_parts: [
    { node: "rubato-server", status: "affected", change: "lists pulls", seams: ["server-github", "web-server"] },
    { node: "web", status: "affected", change: "shows them", seams: ["web-server"] },
    { node: "github", status: "quiet", change: "", seams: [] },
  ],
};

describe("a plan's mini map (issue #81)", () => {
  it("lights exactly the plan's affected parts and seams", () => {
    render(<MiniMap map={map} plan={plan} />);
    const svg = screen.getByTestId("mini-map");
    const lit = [...svg.querySelectorAll(".mini-part.affected, .mini-part.new")].map((c) => c.getAttribute("data-part"));
    expect(lit.sort()).toEqual(["rubato-server", "web"]);
    expect(svg.querySelectorAll(".mini-part")).toHaveLength(map.nodes.length);
    const seams = [...svg.querySelectorAll(".mini-seam.lit")].map((l) => l.getAttribute("data-seam"));
    expect(seams.sort()).toEqual(["server-github", "web-server"]);
    expect(svg).toHaveAttribute("aria-label", "Where it lands: rubato-server, web");
    console.log(`checked ${map.nodes.length} parts and ${(map.seams ?? []).length} seams on the mini map`);
  });

  it("uses the team's layout when it places every part, and fits the box either way", () => {
    const positions = Object.fromEntries(map.nodes.map((n, i) => [n.id, { x: i * 100, y: 0 }]));
    const placed = miniPositions(map, { positions });
    const first = map.nodes[0]!.id;
    const last = map.nodes[map.nodes.length - 1]!.id;
    expect(placed.get(first)!.x).toBeLessThan(placed.get(last)!.x);
    for (const p of [...placed.values(), ...miniPositions(map).values()]) {
      expect(p.x).toBeGreaterThanOrEqual(0);
      expect(p.x).toBeLessThanOrEqual(120);
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.y).toBeLessThanOrEqual(68);
    }
  });

  it("layers parts by their seams: a seam's target sits right of its source", () => {
    const at = layered(map);
    for (const s of map.seams ?? []) {
      expect(at.get(s.to)!.x, s.id).toBeGreaterThan(at.get(s.from)!.x);
    }
  });
});
