import { describe, expect, it } from "vitest";
import { type XY, placePart, positionsToSave } from "./layout.ts";

const at = (x: number, y: number): XY => ({ x, y });

describe("the shared layout (S6, S7)", () => {
  it("a stored position wins over ELK", () => {
    expect(placePart({ auto: at(0, 0), stored: at(300, 40), held: undefined })).toEqual(at(300, 40));
    // No stored position: the automatic layout places the part, as before this feature.
    expect(placePart({ auto: at(12, 34), stored: undefined, held: undefined })).toEqual(at(12, 34));
    // Neither yet (ELK still running): the origin, never undefined.
    expect(placePart({ auto: undefined, stored: undefined, held: undefined })).toEqual(at(0, 0));
  });

  it("an incoming position does not move the part being dragged", () => {
    // `held` is where this browser has the part while it drags it or waits for its own write.
    expect(placePart({ auto: at(0, 0), stored: at(300, 40), held: at(555, 66) })).toEqual(at(555, 66));
  });

  it("the drag-stop handler sends only the moved part", () => {
    const shown = new Map([
      ["web", at(10, 10)],
      ["rubato-server", at(200, 10)],
    ]);
    const changes = [
      // Still under the pointer: nothing is sent until the drop.
      { type: "position" as const, id: "web", position: at(50, 60), dragging: true },
      // Dropped at a new place.
      { type: "position" as const, id: "rubato-server", position: at(260.4, 80.6), dragging: false },
      // Not a move at all.
      { type: "dimensions" as const, id: "web" },
      { type: "select" as const, id: "web", selected: true },
    ];
    expect(positionsToSave(changes, shown)).toEqual({ positions: { "rubato-server": at(260, 81) } });
  });

  it("a drop where the part already is sends nothing", () => {
    const shown = new Map([["web", at(10, 10)]]);
    expect(positionsToSave([{ type: "position", id: "web", position: at(10.2, 9.9), dragging: false }], shown)).toBeNull();
    expect(positionsToSave([], shown)).toBeNull();
    // A position change with no position (React Flow sends these) is not a move.
    expect(positionsToSave([{ type: "position", id: "web", dragging: false }], shown)).toBeNull();
  });
});
