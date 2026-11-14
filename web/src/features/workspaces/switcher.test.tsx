import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import { WorkspaceSwitcher } from "./Switcher.tsx";

const two = [
  { id: "default", name: "Admin", map_repo: "acme/admin-map" },
  { id: "shop", name: "Shop", map_repo: "acme/shop-map" },
];

describe("the workspace switcher (issue #89)", () => {
  it("is not drawn for one workspace", () => {
    const { container } = render(<WorkspaceSwitcher overview={overview({ workspace: "default", workspaces: [] })} onSwitch={() => {}} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("lists every workspace, shows the current one and switches", () => {
    const onSwitch = vi.fn();
    render(<WorkspaceSwitcher overview={overview({ workspace: "shop", workspaces: two })} onSwitch={onSwitch} />);
    const select = screen.getByRole("combobox", { name: "Workspace" }) as HTMLSelectElement;
    expect(select.value).toBe("shop");
    expect([...select.options].map((o) => o.text)).toEqual(["Admin", "Shop"]);
    fireEvent.change(select, { target: { value: "default" } });
    expect(onSwitch).toHaveBeenCalledWith("default");
  });
});
