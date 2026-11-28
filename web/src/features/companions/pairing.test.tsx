import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { companion, overview } from "../../../test/fixtures.ts";
import { CompanionsPanel } from "./CompanionsPanel.tsx";

const me = { login: "m0x6A" };
const noQueue = { onQueue: () => {}, onReorder: () => {}, onUnqueue: () => {}, onStart: () => {} };

function stubFetch(status: number, body: unknown) {
  const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => new Response(body === undefined ? null : JSON.stringify(body), { status }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

afterEach(() => vi.unstubAllGlobals());

describe("pairing a companion (N17)", () => {
  it("someone with no companion sees how to pair one, in place of nothing; someone with one sees their lanes and a quiet way to pair another (issue #357)", () => {
    const { unmount } = render(<CompanionsPanel overview={overview({ companions: [companion("alice", true)] })} me={me} onChanged={() => {}} onPair={() => {}} {...noQueue} />);
    expect(screen.getByTestId("pair-companion")).toHaveTextContent("rubato-companion pair --server");
    expect(screen.queryByTestId("pair-another")).toBeNull();
    unmount();
    const onPair = vi.fn();
    render(<CompanionsPanel overview={overview({ companions: [companion("m0x6A", true)] })} me={me} onChanged={() => {}} onPair={onPair} {...noQueue} />);
    expect(screen.queryByTestId("pair-companion")).toBeNull();
    expect(screen.queryByTestId("pair-code")).toBeNull();
    fireEvent.click(screen.getByTestId("pair-another"));
    expect(onPair).toHaveBeenCalledTimes(1);
  });

  it("the pairing view says what was paired and goes back to the lanes", async () => {
    stubFetch(200, companion("m0x6A", false));
    const onPairDone = vi.fn();
    render(<CompanionsPanel overview={overview({ companions: [companion("m0x6A", true)] })} me={me} onChanged={() => {}} pairing onPairDone={onPairDone} {...noQueue} />);
    expect(screen.queryByTestId("lanes")).toBeNull();
    fireEvent.change(screen.getByTestId("pair-code"), { target: { value: "abcd-efgh" } });
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(onPairDone).toHaveBeenCalledWith("Paired m0x6A-laptop as m0x6A."));
    fireEvent.click(screen.getByRole("button", { name: "← all companions" }));
    expect(onPairDone).toHaveBeenLastCalledWith(null);
  });

  it("approving a code posts it", async () => {
    const fetchMock = stubFetch(200, companion("m0x6A", false));
    const onChanged = vi.fn();
    render(<CompanionsPanel overview={overview()} me={me} onChanged={onChanged} {...noQueue} />);

    const approve = screen.getByRole("button", { name: "Approve" });
    expect(approve).toBeDisabled();
    fireEvent.change(screen.getByTestId("pair-code"), { target: { value: " abcd-efgh " } });
    expect(approve).toBeEnabled();
    fireEvent.click(approve);

    await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/pair/approve");
    expect(init.method).toBe("POST");
    expect(JSON.parse(String(init.body))).toEqual({ user_code: "ABCD-EFGH" });
    expect(screen.getByText("Paired m0x6A-laptop as m0x6A.")).toBeInTheDocument();
  });

  it("revoke is offered only on your own", async () => {
    const fetchMock = stubFetch(204, undefined);
    const onChanged = vi.fn();
    const mine = companion("m0x6A", true, { companion_id: "11111111-1111-1111-1111-111111111111" });
    const theirs = companion("alice", true, { companion_id: "22222222-2222-2222-2222-222222222222" });
    const revoked = companion("m0x6A", false, { companion_id: "33333333-3333-3333-3333-333333333333", revoked_at: new Date().toISOString() });
    render(<CompanionsPanel overview={overview({ companions: [mine, theirs, revoked] })} me={me} onChanged={onChanged} {...noQueue} />);

    expect(within(screen.getByTestId("companion-alice")).queryByRole("button", { name: /revoke/i })).toBeNull();
    // One button in total: my live pairing has it, my revoked one and alice's do not.
    const buttons = screen.getAllByRole("button", { name: "Revoke m0x6A-laptop" });
    expect(buttons).toHaveLength(1);
    fireEvent.click(buttons[0] as HTMLElement);
    // Asked once before it is done (issue #357): nothing is sent on the first press.
    expect(fetchMock).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Keep it" })).toHaveFocus();
    fireEvent.click(screen.getByTestId(`confirm-revoke-${mine.companion_id}`));

    await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(`/api/pairings/${mine.companion_id}`);
    expect(init.method).toBe("DELETE");
  });
});
