import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { dataTransfer } from "../../../test/dnd.ts";
import { issue, overview } from "../../../test/fixtures.ts";
import type { IssueRef } from "../../protocol.ts";
import { setIssueDrag } from "../../shared/dnd.ts";
import { buildGraph } from "./graph.ts";
import { PartCard } from "./PartCard.tsx";
import { SeamChip } from "./SeamChip.tsx";

function dragOf(ref: IssueRef): DataTransfer {
  const dt = dataTransfer();
  setIssueDrag({ dataTransfer: dt }, ref);
  return dt;
}

const graph = (ov = overview()) => buildGraph(ov);
const part = (id: string, ov = overview()) => {
  const n = graph(ov).nodes.find((x) => x.id === id);
  if (!n) throw new Error(`no part ${id} in the map`);
  return n;
};
const seam = (id: string, ov = overview()) => {
  const e = graph(ov).edges.find((x) => x.id === id);
  if (!e) throw new Error(`no seam ${id} in the map`);
  return e;
};

describe("drop targets on the map (S3)", () => {
  it("drop on a part card posts a node pin", () => {
    const onPin = vi.fn();
    render(<PartCard node={part("rubato-server")} selected={false} onPin={onPin} />);
    const card = screen.getByTestId("node-rubato-server");
    const dt = dragOf({ repo: "m0x6A/rubato", number: 902 });

    expect(card).not.toHaveClass("drop-over");
    const over = fireEvent.dragOver(card, { dataTransfer: dt });
    expect(over).toBe(false); // default prevented: the card accepts the drop
    expect(card).toHaveClass("drop-over");
    fireEvent.dragLeave(card, { dataTransfer: dt });
    expect(card).not.toHaveClass("drop-over");

    fireEvent.dragOver(card, { dataTransfer: dt });
    fireEvent.drop(card, { dataTransfer: dt });
    expect(card).not.toHaveClass("drop-over");
    expect(onPin).toHaveBeenCalledTimes(1);
    expect(onPin).toHaveBeenCalledWith({ repo: "m0x6A/rubato", number: 902 }, { kind: "node", id: "rubato-server" });
  });

  it("drop on a seam chip posts a seam pin", () => {
    const onPin = vi.fn();
    const onSelect = vi.fn();
    render(<SeamChip seam={seam("web-server")} selected={false} onSelect={onSelect} onPin={onPin} />);
    const chip = screen.getByTestId("seam-web-server");
    expect(chip.tagName).toBe("BUTTON");
    expect(chip).toHaveTextContent("web-server");
    const dt = dragOf({ repo: "m0x6A/rubato", number: 903 });

    fireEvent.dragEnter(chip, { dataTransfer: dt });
    expect(chip).toHaveClass("drop-over");
    fireEvent.drop(chip, { dataTransfer: dt });
    expect(chip).not.toHaveClass("drop-over");
    expect(onPin).toHaveBeenCalledWith({ repo: "m0x6A/rubato", number: 903 }, { kind: "seam", id: "web-server" });

    // The chip is a button: a click (and so Enter) selects the seam.
    fireEvent.click(chip);
    expect(onSelect).toHaveBeenCalledWith("web-server");
    expect(onPin).toHaveBeenCalledTimes(1);
  });

  it("dropping a pinned issue adds a second pin", () => {
    const onPin = vi.fn();
    // #7 is already pinned to `web`; the card for another part still takes it, as an added pin.
    const ov = overview({ issues: [issue(7, "web")] });
    render(
      <>
        <PartCard node={part("rubato-server", ov)} selected={false} onPin={onPin} />
        <SeamChip seam={seam("web-server", ov)} selected={false} onSelect={() => {}} onPin={onPin} />
      </>,
    );
    const dt = dragOf({ repo: "m0x6A/rubato", number: 7 });
    fireEvent.dragOver(screen.getByTestId("node-rubato-server"), { dataTransfer: dt });
    fireEvent.drop(screen.getByTestId("node-rubato-server"), { dataTransfer: dt });
    fireEvent.drop(screen.getByTestId("seam-web-server"), { dataTransfer: dt });
    expect(onPin.mock.calls).toEqual([
      [{ repo: "m0x6A/rubato", number: 7 }, { kind: "node", id: "rubato-server" }],
      [{ repo: "m0x6A/rubato", number: 7 }, { kind: "seam", id: "web-server" }],
    ]);
  });

  it("ignores a drag that is not an issue", () => {
    const onPin = vi.fn();
    render(<PartCard node={part("rubato-server")} selected={false} onPin={onPin} />);
    const card = screen.getByTestId("node-rubato-server");
    const dt = dataTransfer();
    dt.setData("text/plain", "some text");
    expect(fireEvent.dragOver(card, { dataTransfer: dt })).toBe(true);
    expect(card).not.toHaveClass("drop-over");
    fireEvent.drop(card, { dataTransfer: dt });
    expect(onPin).not.toHaveBeenCalled();
  });
});
