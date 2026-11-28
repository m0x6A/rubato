import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Confirm } from "./Confirm.tsx";
import { Menu } from "./Menu.tsx";

function fruit(onPick: (name: string) => void) {
  return (
    <Menu button="Pick…" buttonLabel="Pick a fruit…" menuLabel="Fruit">
      {(close) =>
        ["apple", "pear", "plum"].map((name) => (
          <button
            key={name}
            type="button"
            role="menuitem"
            tabIndex={-1}
            onClick={() => {
              close();
              onPick(name);
            }}
          >
            {name}
          </button>
        ))
      }
    </Menu>
  );
}

describe("the one menu (issue #357)", () => {
  it("opens on its first entry; arrows, Home and End move and wrap; a pick closes it and returns focus", () => {
    const onPick = vi.fn();
    render(fruit(onPick));
    const button = screen.getByRole("button", { name: "Pick a fruit…" });
    expect(button).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(button);
    const menu = screen.getByRole("menu", { name: "Fruit" });
    const [apple, pear, plum] = screen.getAllByRole("menuitem");
    expect(apple).toHaveFocus();
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(pear).toHaveFocus();
    fireEvent.keyDown(menu, { key: "End" });
    expect(plum).toHaveFocus();
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(apple).toHaveFocus();
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(plum).toHaveFocus();
    fireEvent.keyDown(menu, { key: "Home" });
    expect(apple).toHaveFocus();

    fireEvent.click(pear!);
    expect(onPick).toHaveBeenCalledWith("pear");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(button).toHaveFocus();
  });

  it("Escape closes the menu and goes no further, so the view behind it stays", () => {
    const outside = vi.fn();
    render(<div onKeyDown={outside}>{fruit(() => {})}</div>);
    fireEvent.click(screen.getByRole("button", { name: "Pick a fruit…" }));
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(outside).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Pick a fruit…" })).toHaveFocus();
  });

  it("leaving the menu closes it", () => {
    render(
      <>
        {fruit(() => {})}
        <button type="button">elsewhere</button>
      </>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Pick a fruit…" }));
    fireEvent.blur(screen.getAllByRole("menuitem")[0]!, { relatedTarget: screen.getByRole("button", { name: "elsewhere" }) });
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("ask once, inline (issue #357)", () => {
  const ui = (onConfirm: () => void, disabled = false) => (
    <Confirm ask="Remove…" askLabel="Remove ada" confirm="Remove ada" confirmTestId="yes" keep="Keep" disabled={disabled} onConfirm={onConfirm} explain={<p>She loses access.</p>} wrap={(q) => <div data-testid="question">{q}</div>} />
  );

  it("the button gives way to what it means and two buttons, with focus on the way out", () => {
    const onConfirm = vi.fn();
    render(ui(onConfirm));
    expect(screen.queryByTestId("question")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Remove ada" }));
    expect(screen.getByTestId("question")).toHaveTextContent("She loses access.");
    expect(screen.getByRole("button", { name: "Keep" })).toHaveFocus();
    expect(onConfirm).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId("yes"));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("question")).toBeNull();
  });

  it("Keep and Escape leave things as they are, and focus returns to the button", () => {
    const onConfirm = vi.fn();
    const outside = vi.fn();
    render(<div onKeyDown={outside}>{ui(onConfirm)}</div>);
    fireEvent.click(screen.getByRole("button", { name: "Remove ada" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep" }));
    expect(screen.queryByTestId("question")).toBeNull();
    expect(screen.getByRole("button", { name: "Remove ada" })).toHaveFocus();

    fireEvent.click(screen.getByRole("button", { name: "Remove ada" }));
    fireEvent.keyDown(screen.getByRole("button", { name: "Keep" }), { key: "Escape" });
    expect(screen.queryByTestId("question")).toBeNull();
    expect(outside).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
