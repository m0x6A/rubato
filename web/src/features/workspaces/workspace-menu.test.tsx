import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import { WorkspaceMenu } from "./WorkspaceMenu.tsx";

const two = [
  { id: "default", name: "Admin", map_repo: "acme/admin-map" },
  { id: "shop", name: "Shop", map_repo: "acme/shop-map" },
];

const handlers = () => ({ onSwitch: vi.fn(), onConfigure: vi.fn(), onWorkspaces: vi.fn() });

describe("the workspace menu (issue #89, issue #357)", () => {
  it("with one workspace it still leads to its settings, names the map's commit, and offers nothing to switch to (ADR 0075)", () => {
    const h = handlers();
    const ov = overview({ workspace: "default", workspaces: [] });
    render(<WorkspaceMenu overview={ov} {...h} />);
    fireEvent.click(screen.getByRole("button", { name: `Workspace ${ov.map.map.name}` }));
    expect(screen.getByTestId("map-version")).toHaveTextContent(`map ${ov.map.map.name} @ ${ov.map.version.slice(0, 7)}`);
    expect(screen.queryByRole("menuitemradio")).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: "Configure workspace…" }));
    expect(h.onConfigure).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("names the workspace shown, lists every workspace and switches", () => {
    const h = handlers();
    render(<WorkspaceMenu overview={overview({ workspace: "shop", workspaces: two })} {...h} />);
    fireEvent.click(screen.getByRole("button", { name: "Workspace Shop" }));
    const choices = screen.getAllByRole("menuitemradio");
    expect(choices.map((c) => c.textContent)).toEqual(["Admin", "Shopshowing"]);
    expect(choices[1]).toHaveAttribute("aria-checked", "true");
    // The first entry has focus: the keyboard reaches every one.
    expect(choices[0]).toHaveFocus();
    fireEvent.click(choices[0]!);
    expect(h.onSwitch).toHaveBeenCalledWith("default");
  });

  it("choosing the workspace already shown switches nothing; Workspaces… leads to the list", () => {
    const h = handlers();
    render(<WorkspaceMenu overview={overview({ workspace: "shop", workspaces: two })} {...h} />);
    fireEvent.click(screen.getByRole("button", { name: "Workspace Shop" }));
    fireEvent.click(screen.getByTestId("switch-workspace-shop"));
    expect(h.onSwitch).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Workspace Shop" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Workspaces…" }));
    expect(h.onWorkspaces).toHaveBeenCalledTimes(1);
  });
});
