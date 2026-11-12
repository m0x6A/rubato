import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { minutesAgo, openIssue, realMap } from "../../../test/fixtures.ts";
import type { Backlog as BacklogData } from "../../protocol.ts";
import { Backlog } from "./Backlog.tsx";

const map = realMap();
const REPO = "m0x6A/rubato";

function renderBacklog(backlog: BacklogData | undefined, onPin = vi.fn()) {
  render(<Backlog backlog={backlog} map={map} onPin={onPin} />);
  return onPin;
}

const rows = () => screen.queryAllByTestId(/^backlog-row-\d+$/).map((el) => el.getAttribute("data-testid"));

describe("the Not pinned section (S2)", () => {
  it("lists every not-pinned issue", () => {
    renderBacklog({
      issues: [openIssue(901, "Lens: a plan with flows", { labels: ["map", "web"] }), openIssue(902, "Backlog: an unpinned issue"), openIssue(903, "Backlog: another unpinned issue")],
      repos: [{ repo: REPO, listed_at: minutesAgo(5) }],
    });
    expect(rows()).toEqual(["backlog-row-901", "backlog-row-902", "backlog-row-903"]);
    const first = within(screen.getByTestId("backlog-row-901"));
    expect(first.getByRole("link", { name: "m0x6A/rubato#901" })).toHaveAttribute("href", "https://github.com/m0x6A/rubato/issues/901");
    expect(first.getByText("Lens: a plan with flows")).toBeInTheDocument();
    expect(first.getByText("map")).toBeInTheDocument();
    expect(first.getByText("web")).toBeInTheDocument();
    expect(screen.getByTestId("backlog-repo-m0x6A/rubato")).toHaveTextContent("m0x6A/rubato listed 5 min ago");
    expect(screen.getByTestId("backlog")).toHaveTextContent("Not pinned (3)");
  });

  it("filters by text", () => {
    renderBacklog({
      issues: [openIssue(901, "Lens: a plan with flows", { labels: ["map"] }), openIssue(902, "Backlog: an unpinned issue", { labels: ["server"] }), openIssue(913, "Something else")],
      repos: [{ repo: REPO, listed_at: minutesAgo(1) }],
    });
    const filter = screen.getByRole("searchbox", { name: "Filter not-pinned issues" });
    fireEvent.change(filter, { target: { value: "BACKLOG" } }); // title, any case
    expect(rows()).toEqual(["backlog-row-902"]);
    fireEvent.change(filter, { target: { value: "map" } }); // label
    expect(rows()).toEqual(["backlog-row-901"]);
    fireEvent.change(filter, { target: { value: "#913" } }); // key
    expect(rows()).toEqual(["backlog-row-913"]);
    fireEvent.change(filter, { target: { value: "no such thing" } });
    expect(rows()).toEqual([]);
    expect(screen.getByText(/No not-pinned issue matches/)).toBeInTheDocument();
    expect(screen.queryByText(/No open issues in/)).toBeNull();
  });

  it("shows the listing error with the last good time, not an empty list", () => {
    renderBacklog({ issues: [], repos: [{ repo: REPO, listed_at: minutesAgo(42), error: "GitHub answered 502" }] });
    const repo = screen.getByTestId("backlog-repo-m0x6A/rubato");
    expect(repo).toHaveTextContent("m0x6A/rubato could not be listed: GitHub answered 502");
    expect(repo).toHaveTextContent("last listed 42 min ago");
    expect(screen.queryByText(/No open issues in/)).toBeNull();
    expect(rows()).toEqual([]);
  });

  it("says not listed yet before the first listing", () => {
    renderBacklog({ repos: [{ repo: REPO }] });
    expect(screen.getByTestId("backlog-repo-m0x6A/rubato")).toHaveTextContent("m0x6A/rubato not listed yet");
    expect(screen.queryByText(/No open issues in/)).toBeNull();
  });

  it("a failed first listing says so and that nothing was ever listed", () => {
    renderBacklog({ repos: [{ repo: REPO, error: "not covered by the installation" }] });
    expect(screen.getByTestId("backlog-repo-m0x6A/rubato")).toHaveTextContent("m0x6A/rubato could not be listed: not covered by the installation · never listed");
    expect(screen.queryByText(/No open issues in/)).toBeNull();
  });

  it("says so when the map names no repository", () => {
    renderBacklog({ issues: [], repos: [] });
    expect(screen.getByText(/The map names no repository/)).toBeInTheDocument();
    expect(screen.queryByText(/No open issues in/)).toBeNull();
  });

  it("says there are no open issues only when every repo was listed", () => {
    renderBacklog({ issues: [], repos: [{ repo: REPO, listed_at: minutesAgo(2) }] });
    expect(screen.getByText("No open issues in m0x6A/rubato waiting to be pinned.")).toBeInTheDocument();
  });

  it("marks a truncated listing", () => {
    renderBacklog({ issues: [openIssue(1, "one")], repos: [{ repo: REPO, listed_at: minutesAgo(2), truncated: true }] });
    expect(screen.getByTestId("backlog-repo-m0x6A/rubato")).toHaveTextContent("first 500 shown");
  });
});

describe("suggested places (ADR 0026)", () => {
  it("Pin here pins at the suggested place with one click", () => {
    const scored = { ...openIssue(904, "rubato-server is slow"), suggestion: { target: { kind: "node" as const, id: "rubato-server" }, score: 100, why: ["id rubato-server in the issue"] } };
    const plain = openIssue(905, "Nothing to go on");
    const onPin = renderBacklog({ issues: [scored, plain], repos: [{ repo: REPO, listed_at: minutesAgo(1) }] });
    const s = screen.getByTestId("suggestion-904");
    expect(s).toHaveTextContent("Suggested: part rubato-server");
    expect(s).toHaveTextContent("id rubato-server in the issue");
    expect(screen.queryByTestId("suggestion-905")).toBeNull();
    expect(onPin).not.toHaveBeenCalled();
    fireEvent.click(within(s).getByRole("button", { name: "Pin m0x6A/rubato#904 to part rubato-server" }));
    expect(onPin).toHaveBeenCalledWith({ repo: REPO, number: 904 }, { kind: "node", id: "rubato-server" });
  });

  it("says when the place comes from a non-member's Map: line", () => {
    const seam = { kind: "seam" as const, id: "web-server" };
    renderBacklog({ issues: [{ ...openIssue(906, "From outside"), map_line: seam, suggestion: { target: seam, score: 100 } }], repos: [{ repo: REPO, listed_at: minutesAgo(1) }] });
    expect(screen.getByTestId("suggestion-906")).toHaveTextContent("Suggested: seam web-server (its Map: line, from someone outside the team)");
  });
});

describe("the row menu (S3, N16)", () => {
  it("Pin to… pins by keyboard", () => {
    const onPin = renderBacklog({ issues: [openIssue(902, "Backlog: an unpinned issue")], repos: [{ repo: REPO, listed_at: minutesAgo(1) }] });
    const trigger = within(screen.getByTestId("backlog-row-902")).getByRole("button", { name: "Pin m0x6A/rubato#902 to…" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("menu")).toBeNull();

    // Enter and Space activate a real button; the menu opens with focus on the first part.
    fireEvent.click(trigger);
    const menu = screen.getByRole("menu");
    const items = within(menu).getAllByRole("menuitem");
    expect(items).toHaveLength(map.nodes.length + (map.seams ?? []).length);
    expect(items[0]).toHaveFocus();

    // Escape closes and hands focus back.
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(trigger).toHaveFocus();
    expect(onPin).not.toHaveBeenCalled();

    // Arrow keys walk the items; End jumps to the last seam; Enter on a real button is a click.
    fireEvent.click(trigger);
    fireEvent.keyDown(document.activeElement as Element, { key: "ArrowDown" });
    expect(within(screen.getByRole("menu")).getAllByRole("menuitem")[1]).toHaveFocus();
    fireEvent.keyDown(document.activeElement as Element, { key: "End" });
    const last = within(screen.getByRole("menu")).getAllByRole("menuitem").at(-1) as HTMLElement;
    expect(last).toHaveFocus();
    const lastSeam = (map.seams ?? []).at(-1)?.id as string;
    expect(last).toHaveTextContent(lastSeam);
    fireEvent.click(last);

    expect(onPin).toHaveBeenCalledTimes(1);
    expect(onPin).toHaveBeenCalledWith({ repo: REPO, number: 902 }, { kind: "seam", id: lastSeam });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("a row carries its issue when dragged", async () => {
    const { dataTransfer } = await import("../../../test/dnd.ts");
    const { getIssueDrag } = await import("../../shared/dnd.ts");
    renderBacklog({ issues: [openIssue(902, "Backlog: an unpinned issue")], repos: [{ repo: REPO, listed_at: minutesAgo(1) }] });
    const row = screen.getByTestId("backlog-row-902");
    expect(row).toHaveAttribute("draggable", "true");
    const dt = dataTransfer();
    fireEvent.dragStart(row, { dataTransfer: dt });
    expect(getIssueDrag({ dataTransfer: dt })).toEqual({ repo: REPO, number: 902 });
  });

  it("a drag that starts on the issue link is the row's drag", async () => {
    const { dataTransfer } = await import("../../../test/dnd.ts");
    const { ISSUE_MIME } = await import("../../shared/dnd.ts");
    renderBacklog({ issues: [openIssue(902, "Backlog: an unpinned issue")], repos: [{ repo: REPO, listed_at: minutesAgo(1) }] });
    const link = screen.getByRole("link", { name: "m0x6A/rubato#902" });
    // Not draggable on its own, so the browser drags the row and not a URL.
    expect(link).toHaveAttribute("draggable", "false");
    const dt = dataTransfer();
    fireEvent.dragStart(link, { dataTransfer: dt });
    expect(dt.types).toContain(ISSUE_MIME);
  });

  it("says how to pin under the heading", () => {
    renderBacklog({ issues: [openIssue(902, "Backlog: an unpinned issue")], repos: [{ repo: REPO, listed_at: minutesAgo(1) }] });
    expect(screen.getByText("Drag an issue onto a part or a seam, or use Pin to…")).toBeInTheDocument();
  });
});
