import { fireEvent } from "@testing-library/react";

/** `screen`, or what `render` and `within` return: anything that finds elements by role. */
type In = {
  getByRole: (role: string, options?: { name?: string | RegExp }) => HTMLElement;
  queryByRole: (role: string, options?: { name?: string | RegExp }) => HTMLElement | null;
};

/** Open the map's View menu (issue #357): how the map is laid out, and the two ways to change it. */
export function openViewMenu(view: In): void {
  fireEvent.click(view.getByRole("button", { name: /^View/ }));
}

/** The View menu's Auto-arrange entry, or null when the map offers none; opens the menu to look. */
export function autoArrangeItem(view: In): HTMLElement | null {
  if (!view.queryByRole("menu", { name: "How the map is laid out" })) {
    openViewMenu(view);
  }
  return view.queryByRole("menuitem", { name: /^Auto-arrange/ });
}

/** Whether a menu entry can be chosen: a menu marks what it cannot do with aria-disabled. */
export const offered = (item: HTMLElement | null): boolean => item !== null && item.getAttribute("aria-disabled") !== "true";
