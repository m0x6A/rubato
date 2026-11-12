import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { overview, realMap } from "../../../test/fixtures.ts";
import { buildGraph } from "./graph.ts";
import { MapView } from "./MapView.tsx";

/** Where React Flow put each part, read from the node wrapper's transform. */
function placed(container: HTMLElement): Map<string, string> {
  return new Map([...container.querySelectorAll<HTMLElement>(".react-flow__node")].map((n) => [n.dataset["id"] ?? "", n.style.transform]));
}

describe("MapView", () => {
  it("no stored layout uses ELK for every part", async () => {
    const graph = buildGraph(overview());
    const auto = render(<MapView graph={graph} selected={null} onSelect={() => {}} />);
    await waitFor(() => expect(auto.getByTestId("node-rubato-server")).toBeInTheDocument(), { timeout: 5000 });
    const byElk = placed(auto.container);
    expect(byElk.size).toBe(realMap().nodes.length);
    // ELK spreads the parts: no two share a place, and none is left at the origin by default.
    expect(new Set(byElk.values()).size).toBe(byElk.size);
    // Without a way to save a move, parts do not drag, and there is no Auto-arrange.
    expect(auto.container.querySelector(".react-flow__node.draggable")).toBeNull();
    expect(auto.queryByRole("button", { name: "Auto-arrange" })).toBeNull();
    auto.unmount();

    // One stored position: that part sits there, every other part stays where ELK put it.
    const stored = render(<MapView graph={graph} selected={null} onSelect={() => {}} layout={{ positions: { web: { x: 1234, y: 567 } } }} onMove={async () => {}} onAutoArrange={() => {}} />);
    await waitFor(() => expect(placed(stored.container).get("web")).toBe("translate(1234px,567px)"), { timeout: 5000 });
    for (const [id, where] of placed(stored.container)) {
      if (id !== "web") {
        expect(where, id).toBe(byElk.get(id));
      }
    }
    expect(stored.container.querySelectorAll(".react-flow__node.draggable").length).toBe(byElk.size);
    expect(stored.getByRole("button", { name: "Auto-arrange" })).toBeEnabled();
  });

  it("Auto-arrange is disabled when nothing is stored", async () => {
    const view = render(<MapView graph={buildGraph(overview())} selected={null} onSelect={() => {}} layout={{ positions: {} }} onMove={async () => {}} onAutoArrange={() => {}} />);
    expect(await view.findByRole("button", { name: "Auto-arrange" }, { timeout: 5000 })).toBeDisabled();
  });

  it("plays a scenario step by step, with a caption a screen reader hears (ADR 0024)", async () => {
    const view = render(<MapView graph={buildGraph(overview())} selected={null} onSelect={() => {}} />);
    await view.findByTestId("node-rubato-server", undefined, { timeout: 5000 });
    expect(view.queryByRole("group", { name: "Scenario player" })).toBeNull();
    fireEvent.click(view.getByRole("radio", { name: "Scenarios (3)" }));
    const player = view.getByRole("group", { name: "Scenario player" });
    expect(within(player).getByRole("combobox", { name: "Scenario" })).toHaveValue("start-a-session");
    const live = view.getByTestId("scenario-caption");
    expect(live).toHaveAttribute("aria-live", "polite");
    expect(live).toHaveTextContent("Step 1 of 11: web → rubato-server, control: LaunchRequest");
    expect(within(player).getByRole("button", { name: "Previous step" })).toBeDisabled();
    fireEvent.click(within(player).getByRole("button", { name: "Next step" }));
    expect(live).toHaveTextContent("Step 2 of 11: rubato-server → rubato-companion, control: LaunchCommand, for the paired user only — the server sends the launch to your own companion only");
    // The keyboard steps too.
    fireEvent.keyDown(player, { key: "ArrowRight" });
    expect(live).toHaveTextContent("Step 3 of 11");
    fireEvent.keyDown(player, { key: "ArrowLeft" });
    expect(live).toHaveTextContent("Step 2 of 11");
    // The step list jumps; the current step is marked.
    const steps = within(player).getByRole("list", { name: "Steps" });
    fireEvent.click(within(steps).getAllByRole("button")[6]!);
    expect(live).toHaveTextContent("Step 7 of 11: rubato-server → web, data:");
    expect(within(steps).getAllByRole("button")[6]).toHaveAttribute("aria-current", "step");
    // Data and control look different without colour: the legend says dot and ring.
    expect(view.getByTestId("legend")).toHaveTextContent("A filled dot is data moving; a ring is control");
    // Another scenario starts at its first step.
    fireEvent.change(within(player).getByRole("combobox", { name: "Scenario" }), { target: { value: "plan-comes-back" } });
    expect(live).toHaveTextContent("Step 1 of 7: system-map → skill-rubato, data:");
  });

  it("renders a card for every node of rubato-map.yml", async () => {
    render(<MapView graph={buildGraph(overview())} selected={null} onSelect={() => {}} />);
    await waitFor(() => expect(screen.getByTestId("node-rubato-server")).toBeInTheDocument(), { timeout: 5000 });
    for (const n of realMap().nodes) {
      expect(screen.getByTestId(`node-${n.id}`)).toBeInTheDocument();
    }
  });
});
