import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { overview, realMap } from "../../../test/fixtures.ts";
import { buildGraph } from "./graph.ts";
import { MapView } from "./MapView.tsx";

describe("MapView", () => {
  it("renders a card for every node of rubato-map.yml", async () => {
    render(<MapView graph={buildGraph(overview())} selected={null} onSelect={() => {}} />);
    await waitFor(() => expect(screen.getByTestId("node-rubato-server")).toBeInTheDocument(), { timeout: 5000 });
    for (const n of realMap().nodes) {
      expect(screen.getByTestId(`node-${n.id}`)).toBeInTheDocument();
    }
  });
});
