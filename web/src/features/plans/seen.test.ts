import { afterEach, describe, expect, it, vi } from "vitest";
import { readSeen, seenKey, writeSeen } from "./seen.ts";

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("the revision last looked at, per draft and per device (ADR 0053)", () => {
  it("is remembered per draft", () => {
    expect(readSeen("d1")).toBeNull();
    writeSeen("d1", 3);
    writeSeen("d2", 1);
    expect(readSeen("d1")).toBe(3);
    expect(readSeen("d2")).toBe(1);
    expect(localStorage.getItem(seenKey("d1"))).toBe("3");
  });

  it("reads anything that is not a revision number as nothing seen", () => {
    for (const v of ["", "abc", "0", "-2", "1.5"]) {
      localStorage.setItem(seenKey("d1"), v);
      expect(readSeen("d1"), v).toBeNull();
    }
  });

  it("works without storage: a read is nothing seen, a write does nothing, neither throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    expect(readSeen("d1")).toBeNull();
    expect(() => writeSeen("d1", 2)).not.toThrow();
  });
});
