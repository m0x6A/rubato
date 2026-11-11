import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import type { DraftOp, MapDraft, MapRef, Overview, SystemMap } from "../../protocol.ts";
import { buildGraph, type DraftMarkLike } from "../map/index.ts";
import { DraftPanel } from "./DraftPanel.tsx";
import { changedWithContext, lineDiff } from "./diff.ts";
import { draftMarks, flowsFromText, flowsToText, seamOpFor, withDraft } from "./overlay.ts";

/** Every request the panel made: method, url and parsed body. */
let calls: { method: string; url: string; body: Record<string, unknown> | null }[];
/** What the next write answers; reads answer from `yaml`. */
let answer: { status: number; body?: string };
const yaml = { base: "version: 1\nname: rubato\nnodes:\n- id: web\n  kind: context\n", proposed: "version: 1\nname: rubato\nnodes:\n- id: web\n  kind: context\n- id: billing\n  kind: context\n" };

beforeEach(() => {
  calls = [];
  answer = { status: 201 };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      calls.push({ method: init?.method ?? "GET", url, body: init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : null });
      if (url.startsWith("/api/map/draft/yaml")) {
        return new Response(url.includes("base=true") ? yaml.base : yaml.proposed, { status: 200 });
      }
      return new Response(answer.body ?? null, { status: answer.status });
    }),
  );
});
afterEach(() => vi.unstubAllGlobals());

const op = (seq: number, author: string, body: Record<string, unknown>, stale?: string): DraftOp => ({ seq, op_id: `00000000-0000-0000-0000-00000000000${seq}`, author, created_at: new Date().toISOString(), ...(stale ? { stale } : {}), ...body }) as DraftOp;
const billing = { id: "billing", kind: "context" as const };
const addBilling = op(1, "m0x6A", { op: "add_node", node: billing });
const addSeam = op(2, "alice", { op: "add_seam", seam: { id: "web-billing", from: "web", to: "billing", owner: "billing" } });

function withOps(ops: DraftOp[], extra: Partial<MapDraft> = {}): Overview {
  const ov = overview();
  const merged = ov.map.map;
  const preview: SystemMap = {
    ...merged,
    nodes: [...merged.nodes, ...ops.flatMap((o) => (o.op === "add_node" && !o.stale ? [o.node] : []))],
    seams: [...(merged.seams ?? []), ...ops.flatMap((o) => (o.op === "add_seam" && !o.stale ? [o.seam] : []))],
  };
  return { ...ov, draft: { ops, ...(ops.length ? { preview } : {}), ...extra } };
}

function show(ov: Overview = overview(), selection: MapRef | null = null) {
  const onChanged = vi.fn();
  const onNotice = vi.fn();
  const onPartAdded = vi.fn();
  render(<DraftPanel overview={ov} selection={selection} onChanged={onChanged} onNotice={onNotice} onPartAdded={onPartAdded} />);
  return { onChanged, onNotice, onPartAdded, panel: screen.getByTestId("draft-panel") };
}
const writes = () => calls.filter((c) => c.method !== "GET");

describe("editing the map as a draft (S18, S19)", () => {
  it("Add part form posts AddNode", async () => {
    const { onChanged, onPartAdded } = show();
    const form = screen.getByRole("form", { name: "Add part" });
    fireEvent.change(within(form).getByLabelText(/^Id/), { target: { value: "billing" } });
    fireEvent.change(within(form).getByLabelText("Kind"), { target: { value: "external" } });
    fireEvent.change(within(form).getByLabelText("What it is"), { target: { value: "Invoices and payments." } });
    fireEvent.click(within(form).getByRole("button", { name: "Add part" }));
    await waitFor(() => expect(onPartAdded).toHaveBeenCalledWith("billing"));
    expect(writes()).toHaveLength(1);
    const { method, url, body } = writes()[0]!;
    expect([method, url]).toEqual(["POST", "/api/map/draft/ops"]);
    // Empty fields are left out, and the browser made the id that lets a retry add it once.
    expect(body).toEqual({ op: "add_node", node: { id: "billing", kind: "external", description: "Invoices and payments." }, op_id: expect.stringMatching(/^[0-9a-f-]{36}$/) });
    expect(onChanged).toHaveBeenCalled();
    expect(within(form).getByLabelText(/^Id/)).toHaveValue("");
  });

  it("a refused operation shows the server's reason and stores nothing here", async () => {
    answer = { status: 422, body: "id web already exists" };
    const { onPartAdded } = show();
    const form = screen.getByRole("form", { name: "Add part" });
    fireEvent.change(within(form).getByLabelText(/^Id/), { target: { value: "web" } });
    fireEvent.click(within(form).getByRole("button", { name: "Add part" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Part web was not added: id web already exists");
    expect(onPartAdded).not.toHaveBeenCalled();
    // What was typed is still there to correct.
    expect(within(form).getByLabelText(/^Id/)).toHaveValue("web");
  });

  it("Add seam form posts AddSeam", async () => {
    show();
    const form = screen.getByRole("form", { name: "Add seam" });
    fireEvent.change(within(form).getByLabelText(/^From/), { target: { value: "web" } });
    fireEvent.change(within(form).getByLabelText(/^To/), { target: { value: "github" } });
    fireEvent.change(within(form).getByLabelText("Carries"), { target: { value: "issue links" } });
    fireEvent.click(within(form).getByRole("button", { name: "Add seam" }));
    await waitFor(() => expect(writes()).toHaveLength(1));
    // No id given: from-to. The To side owns it unless the form says otherwise.
    expect(writes()[0]!.body).toMatchObject({ op: "add_seam", seam: { id: "web-github", from: "web", to: "github", owner: "github", carries: "issue links" } });
  });

  it("the connect handler posts AddSeam and adds no local edge", () => {
    // Dragging from one part's handle to another's asks for this operation; the map draws the
    // seam only when the server has it and the overview brings it back.
    expect(seamOpFor("web", "billing")).toEqual({ op: "add_seam", seam: { id: "web-billing", from: "web", to: "billing", owner: "billing" } });
    expect(seamOpFor("web", "web")).toBeNull();
    expect(seamOpFor("web", null)).toBeNull();
    // The graph the map draws comes from the overview alone: a seam nobody stored is not in it.
    expect(buildGraph(withDraft(overview())).edges.some((e) => e.id === "web-billing")).toBe(false);
    expect(buildGraph(withDraft(withOps([addBilling, addSeam]))).edges.some((e) => e.id === "web-billing")).toBe(true);
  });

  it("editing a field posts an update", async () => {
    show(overview(), { kind: "seam", id: "web-server" });
    const form = screen.getByRole("form", { name: "Change seam web-server" });
    // The form starts from what the map has, flows included.
    const flows = within(form).getByLabelText(/^Flows/) as HTMLTextAreaElement;
    expect(flows.value.split("\n")).toHaveLength(4);
    fireEvent.change(within(form).getByLabelText("Carries"), { target: { value: "REST and one websocket" } });
    fireEvent.change(flows, { target: { value: "data: rubato-server -> web: Overview\ncontrol: web -> rubato-server: LaunchRequest" } });
    fireEvent.click(within(form).getByRole("button", { name: "Save seam" }));
    await waitFor(() => expect(writes()).toHaveLength(1));
    const seam = (writes()[0]!.body as { op: string; seam: Record<string, unknown> }).seam;
    expect(writes()[0]!.body).toMatchObject({ op: "update_seam" });
    // The whole seam is sent: what was not touched is kept, the id and ends cannot change.
    expect(seam).toMatchObject({ id: "web-server", from: "web", to: "rubato-server", owner: "rubato-server", carries: "REST and one websocket", guard: "generated web/src/protocol.ts diffed in CI" });
    expect(seam["flows"]).toEqual([
      { kind: "data", from: "rubato-server", to: "web", carries: "Overview" },
      { kind: "control", from: "web", to: "rubato-server", carries: "LaunchRequest" },
    ]);

    // A flow line that does not read is said here; nothing is sent.
    fireEvent.change(flows, { target: { value: "web to server" } });
    fireEvent.click(within(form).getByRole("button", { name: "Save seam" }));
    expect(await screen.findByRole("alert")).toHaveTextContent('"web to server" is not a flow');
    expect(writes()).toHaveLength(1);
  });

  it("remove posts a removal", async () => {
    show(overview(), { kind: "node", id: "plan-check" });
    fireEvent.click(screen.getByRole("button", { name: "Remove part plan-check" }));
    await waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toMatchObject({ op: "remove_node", id: "plan-check" });

    // And one operation can be taken out of the draft, anyone's.
    answer = { status: 204 };
    cleanup();
    const { panel } = show(withOps([addBilling, addSeam]));
    fireEvent.click(within(panel).getByRole("button", { name: 'Remove "add part billing" from the draft' }));
    await waitFor(() => expect(writes()).toHaveLength(2));
    expect([writes()[1]!.method, writes()[1]!.url]).toEqual(["DELETE", "/api/map/draft/ops/1"]);
  });

  it("draft elements are dashed and badged", () => {
    const ov = withOps([addBilling, addSeam, op(3, "alice", { op: "remove_seam", id: "skill-plan-file" }), op(4, "m0x6A", { op: "update_node", node: { id: "web", kind: "context" } })]);
    const marks = draftMarks(ov.map.map, ov.draft);
    expect(Object.fromEntries(marks)).toEqual<Record<string, DraftMarkLike>>({
      billing: { state: "added", author: "m0x6A" },
      "web-billing": { state: "added", author: "alice" },
      "skill-plan-file": { state: "removed", author: "alice" },
      web: { state: "changed", author: "m0x6A" },
    });
    // The map drawn has the new part and seam; a real removal would still be drawn, marked.
    const graph = buildGraph(withDraft(ov));
    expect(graph.nodes.some((n) => n.id === "billing")).toBe(true);
    expect(graph.edges.some((e) => e.id === "web-billing")).toBe(true);
    // A stale operation marks nothing, and shows why in the list.
    const stale = withOps([op(5, "alice", { op: "add_seam", seam: { id: "web-ledger", from: "web", to: "ledger", owner: "web" } }, 'seam "web-ledger" references unknown node "ledger"')]);
    expect(draftMarks(stale.map.map, stale.draft).size).toBe(0);
    const { panel } = show(stale);
    expect(within(panel).getByTestId("draft-op-5")).toHaveTextContent('no longer applies: seam "web-ledger" references unknown node "ledger"');
    expect(within(panel).getByRole("button", { name: "Propose map change…" })).toBeDisabled();
  });

  it("flows read and write as one line each", () => {
    const flows = [
      { kind: "data" as const, from: "a", to: "b", carries: "Overview: map, issues" },
      { kind: "control" as const, from: "b", to: "a", carries: "reload" },
    ];
    expect(flowsToText(flows)).toBe("data: a -> b: Overview: map, issues\ncontrol: b -> a: reload");
    expect(flowsFromText(flowsToText(flows))).toEqual(flows);
    expect(flowsFromText("\n  \n")).toEqual([]);
    expect(() => flowsFromText("magic: a -> b: x")).toThrow(/is not a flow/);
  });
});

describe("proposing, withdrawing and discarding the draft (S21 to S23)", () => {
  it("Propose shows what changes in the file, then opens the pull request", async () => {
    answer = { status: 200, body: JSON.stringify({ proposal: { state: "proposed", state_since: new Date().toISOString(), branch: "map/draft-1", proposed_by: "m0x6A", pr_number: 7, pr_url: "https://example.invalid/pull/7" } }) };
    const { onNotice, panel } = show(withOps([addBilling]));
    fireEvent.click(within(panel).getByRole("button", { name: "Propose map change…" }));
    const diff = await screen.findByTestId("draft-diff");
    expect(diff).toHaveTextContent("+ - id: billing");
    expect(diff.querySelectorAll(".add")).toHaveLength(2);
    expect(diff.querySelectorAll(".del")).toHaveLength(0);
    // Nothing is written until the person confirms.
    expect(writes()).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Open the pull request" }));
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith(expect.stringContaining("proposed as pull request #7")));
    expect([writes()[0]!.method, writes()[0]!.url]).toEqual(["POST", "/api/map/draft/propose"]);
    expect(screen.queryByTestId("draft-diff")).toBeNull();
  });

  it("a proposed draft is read-only, with the pull request and Withdraw", async () => {
    answer = { status: 200 };
    const proposal = { state: "proposed" as const, state_since: new Date().toISOString(), branch: "map/draft-1", proposed_by: "alice", pr_number: 7, pr_url: "https://example.invalid/pull/7" };
    const { panel, onNotice } = show(withOps([addBilling], { proposal }), { kind: "node", id: "web" });
    expect(within(panel).getByTestId("proposal")).toHaveTextContent("Proposed · pull request #7 by alice");
    expect(within(panel).getByRole("link", { name: "pull request #7" })).toHaveAttribute("href", "https://example.invalid/pull/7");
    // No forms, no removal, no discard: only Withdraw and Copy YAML.
    expect(screen.queryByRole("form")).toBeNull();
    expect(within(panel).queryByRole("button", { name: /^Remove/ })).toBeNull();
    expect(within(panel).queryByRole("button", { name: "Discard draft" })).toBeNull();
    fireEvent.click(within(panel).getByRole("button", { name: "Withdraw proposal" }));
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith(expect.stringContaining("editable again")));
    expect(writes()[0]!.url).toBe("/api/map/draft/withdraw");
  });

  it("a refused proposal says why and keeps the review open", async () => {
    const { panel } = show(withOps([addBilling]));
    fireEvent.click(within(panel).getByRole("button", { name: "Propose map change…" }));
    await screen.findByTestId("draft-diff");
    answer = { status: 409, body: "The map file on main is not the one Rubato has loaded (version abc). Reload the map first, then propose." };
    fireEvent.click(screen.getByRole("button", { name: "Open the pull request" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The map change was not proposed: The map file on main is not the one Rubato has loaded");
    expect(screen.getByTestId("draft-diff")).toBeInTheDocument();
  });

  it("Copy YAML offers the proposed file", async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    const { panel, onNotice } = show(withOps([addBilling]));
    fireEvent.click(within(panel).getByRole("button", { name: "Copy YAML" }));
    const text = (await screen.findByTestId("draft-yaml")) as HTMLTextAreaElement;
    // The whole proposed file, shown to copy by hand too (the clipboard may be refused).
    expect(text.value).toBe(yaml.proposed);
    expect(writeText).toHaveBeenCalledWith(yaml.proposed);
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith("The proposed map file is on the clipboard."));
    expect(calls.map((c) => c.url)).toEqual(["/api/map/draft/yaml"]);
  });

  it("Discard shows Undo", async () => {
    answer = { status: 200 };
    const { panel, onNotice } = show(withOps([addBilling, addSeam]));
    fireEvent.click(within(panel).getByRole("button", { name: "Discard draft" }));
    await waitFor(() => expect(onNotice).toHaveBeenCalled());
    expect(writes()[0]!.url).toBe("/api/map/draft/discard");
    const [text, undo] = onNotice.mock.calls[0] as [string, () => void];
    expect(text).toBe("Draft discarded for everyone: 2 changes.");
    // The notice's Undo restores it.
    undo();
    await waitFor(() => expect(writes().map((c) => c.url)).toContain("/api/map/draft/restore"));

    // Everyone else is offered the same way back, in the panel, while it is still possible.
    cleanup();
    const other = show({ ...overview(), draft: { can_restore: true } });
    fireEvent.click(within(other.panel).getByRole("button", { name: "Undo discard" }));
    await waitFor(() => expect(other.onNotice).toHaveBeenCalledWith("The draft is back, with every change and its author."));
    // With an empty draft there is nothing to propose, discard or copy.
    expect(within(other.panel).getByRole("button", { name: "Propose map change…" })).toBeDisabled();
    expect(within(other.panel).queryByRole("button", { name: "Discard draft" })).toBeNull();
  });
});

describe("the line diff of a proposal", () => {
  it("marks added and removed lines and keeps a little context", () => {
    const before = ["a", "b", "c", "d", "e", "f", "g", "h"].join("\n");
    const after = ["a", "b", "c", "d", "X", "f", "g", "h", "i"].join("\n");
    const diff = lineDiff(before, after);
    expect(diff.filter((l) => l.sign !== " ")).toEqual([
      { sign: "-", text: "e" },
      { sign: "+", text: "X" },
      { sign: "+", text: "i" },
    ]);
    expect(changedWithContext(diff, 1).map((l) => `${l.sign}${l.text}`)).toEqual([" d", "-e", "+X", " f", " …", " h", "+i"]);
    expect(lineDiff("same", "same")).toEqual([{ sign: " ", text: "same" }]);
    expect(changedWithContext(lineDiff("same", "same"))).toEqual([]);
  });
});
