import { useEffect, useState } from "react";

/** How much of the window the right panel takes (#265): 400 px, up to 60% beside the map, or
 * all of it with the map hidden. A per-browser convenience; the page renders without it. */
export type PanelSize = "normal" | "wide" | "full";

const SIZES: PanelSize[] = ["normal", "wide", "full"];
const STORAGE_KEY = "rubato.panel";
/** Below this the page stacks the map over the panel, and the panel has one size. */
const SIDE_BY_SIDE = "(min-width: 901px)";

export function readPanelSize(): PanelSize {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return SIZES.find((s) => s === stored) ?? "normal";
  } catch {
    return "normal";
  }
}

export function storePanelSize(size: PanelSize): void {
  try {
    localStorage.setItem(STORAGE_KEY, size);
  } catch {
    // A private window: the size lasts until the page is closed.
  }
}

export function nextPanelSize(size: PanelSize): PanelSize {
  return SIZES[(SIZES.indexOf(size) + 1) % SIZES.length] ?? "normal";
}

/** What the button does next, as its accessible name, and the glyph it shows (the tab row has no room for a word). */
export const PANEL_ACTION: Record<PanelSize, { name: string; word: string }> = {
  normal: { name: "Widen panel", word: "⇤" },
  wide: { name: "Fill the window with the panel", word: "⇤⇤" },
  full: { name: "Back to normal panel width", word: "⇥" },
};

/** Whether the map and the panel sit side by side. True where the browser cannot say. */
export function useSideBySide(): boolean {
  const query = typeof window !== "undefined" && typeof window.matchMedia === "function" ? window.matchMedia(SIDE_BY_SIDE) : null;
  const [matches, setMatches] = useState(query ? query.matches : true);
  useEffect(() => {
    if (!query) {
      return;
    }
    const onChange = () => setMatches(query.matches);
    query.addEventListener("change", onChange);
    onChange();
    return () => query.removeEventListener("change", onChange);
    // The query object is the same media query for the page's life.
  }, []);
  return matches;
}
