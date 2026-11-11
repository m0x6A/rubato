import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, failureText, onNeedsLogin, request, resetNeedsLogin } from "./api.ts";

beforeEach(() => resetNeedsLogin());
afterEach(() => vi.unstubAllGlobals());

describe("request (S24)", () => {
  it("a 401 raises needs-login once", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("session needs a fresh team record", { status: 401 })));
    const raised = vi.fn();
    const off = onNeedsLogin(raised);

    await expect(request("GET", "/api/overview")).rejects.toMatchObject({ status: 401 });
    await expect(request("POST", "/api/pins", { any: "body" })).rejects.toBeInstanceOf(ApiError);
    await expect(request("GET", "/api/me")).rejects.toMatchObject({ status: 401 });
    expect(raised).toHaveBeenCalledTimes(1);

    // After a reset (a login that worked) the next 401 raises it again.
    resetNeedsLogin();
    await expect(request("GET", "/api/overview")).rejects.toMatchObject({ status: 401 });
    expect(raised).toHaveBeenCalledTimes(2);
    off();
  });

  it("a success without a body is not an error", async () => {
    // `POST /api/pins` answers 201 with nothing in it (pins/handlers.rs).
    vi.stubGlobal("fetch", vi.fn(async () => new Response(null, { status: 201 })));
    await expect(request<void>("POST", "/api/pins", {})).resolves.toBeUndefined();
    vi.stubGlobal("fetch", vi.fn(async () => new Response(null, { status: 204 })));
    await expect(request<void>("DELETE", "/api/pins", {})).resolves.toBeUndefined();
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ login: "m0x6A" }), { status: 200 })));
    await expect(request("GET", "/api/me")).resolves.toEqual({ login: "m0x6A" });
  });

  it("other failures do not raise needs-login", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("unknown node", { status: 422 })));
    const raised = vi.fn();
    const off = onNeedsLogin(raised);
    await expect(request("POST", "/api/pins", {})).rejects.toMatchObject({ status: 422, message: "unknown node" });
    expect(raised).not.toHaveBeenCalled();
    off();
  });
});

describe("failureText", () => {
  it("keeps the server's reason and names a network failure", () => {
    expect(failureText(new ApiError(422, "unknown node"))).toBe("unknown node");
    expect(failureText(new TypeError("Failed to fetch"))).toBe("Could not reach the server");
    expect(failureText("anything else")).toBe("Could not reach the server");
  });
});
