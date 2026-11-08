import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MapView } from "../src/components/MapView.tsx";
import { buildGraph } from "../src/graph.ts";
import { overview, realMap } from "./fixtures.ts";

describe("MapView", () => {
  it("renders a card for every node of rubato-map.yml", async () => {
    render(<MapView graph={buildGraph(overview())} selectedNode={null} onSelectNode={() => {}} />);
    await waitFor(() => expect(screen.getByTestId("node-rubato-server")).toBeInTheDocument(), { timeout: 5000 });
    for (const n of realMap().nodes) {
      expect(screen.getByTestId(`node-${n.id}`)).toBeInTheDocument();
    }
  });
});
