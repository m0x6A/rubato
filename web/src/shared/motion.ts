/**
 * The one motion switch (issue #81, design §5). `data-motion` on `<html>` is `full` unless the
 * person asked for less motion; every CSS animation and transition is written under
 * `:root[data-motion="full"]`, so nothing moves until this has run, and nothing moves after it
 * says `reduced`. JS that decides whether to move (a panned view, particles) reads `useMotion`.
 */
import { useEffect, useState } from "react";

const QUERY = "(prefers-reduced-motion: reduce)";

function query(): MediaQueryList | null {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" ? window.matchMedia(QUERY) : null;
}

function apply(reduced: boolean): void {
  if (typeof document !== "undefined") {
    document.documentElement.dataset["motion"] = reduced ? "reduced" : "full";
  }
}

/** Set `data-motion` now and keep it in step with the preference. Returns the unsubscribe. */
export function startMotionSwitch(): () => void {
  const q = query();
  apply(q?.matches ?? false);
  if (!q) {
    return () => {};
  }
  const on = () => apply(q.matches);
  q.addEventListener("change", on);
  return () => q.removeEventListener("change", on);
}

/** Whether the person asked for less motion. jsdom and old browsers have no matchMedia: full motion, as CSS without the switch would still show nothing moving. */
export function useReducedMotion(): boolean {
  const [q] = useState(query);
  const [reduced, setReduced] = useState(q?.matches ?? false);
  useEffect(() => {
    if (!q) {
      return;
    }
    const on = () => setReduced(q.matches);
    q.addEventListener("change", on);
    return () => q.removeEventListener("change", on);
  }, [q]);
  return reduced;
}
