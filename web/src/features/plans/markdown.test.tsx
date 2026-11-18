import { fireEvent, render, screen } from "@testing-library/react";
import { createElement } from "react";
import ReactMarkdown from "react-markdown";
import { describe, expect, it } from "vitest";
import Markdown from "./Markdown.tsx";
import { PlanText } from "./PlanText.tsx";

const PLAN = `# A plan

## Affected parts

| Node | Status |
| --- | --- |
| web | affected |

- [ ] a task

\`\`\`mermaid
flowchart LR
  a --> b
\`\`\`

See [the issue](https://github.com/m0x6A/rubato/issues/265).
`;

/** What a hostile plan might carry, and the element it must not become. */
const HOSTILE: { name: string; text: string; selector: string }[] = [
  { name: "a script", text: "before\n\n<script>window.pwned = 1</script>\n\nafter", selector: "script" },
  { name: "an image with a handler", text: '<img src="x" onerror="window.pwned = 1">', selector: "img" },
  { name: "a javascript: link", text: "[press](javascript:window.pwned=1)", selector: 'a[href^="javascript"]' },
  { name: "a Markdown image", text: "![tracker](https://evil.example/pixel.png)", selector: "img" },
];

describe("plan text as Markdown (ADR 0079)", () => {
  it("renders headings, a table, a task list and a link; a Mermaid block stays code", () => {
    render(<Markdown text={PLAN} />);
    const body = screen.getByTestId("md-body");
    expect(screen.getByRole("heading", { level: 1, name: "A plan" })).toBeInTheDocument();
    expect(body.querySelector(".md-table table")).not.toBeNull();
    expect(screen.getByRole("cell", { name: "affected" })).toBeInTheDocument();
    expect(body.querySelector('input[type="checkbox"]')).toBeDisabled();
    expect(body.querySelector("pre code")).toHaveTextContent("flowchart LR");
    expect(body.querySelector("svg")).toBeNull();
    const link = screen.getByRole("link", { name: "the issue" });
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  it("a hostile plan puts nothing on the page but text and safe links", () => {
    for (const h of HOSTILE) {
      const { container, unmount } = render(<Markdown text={h.text} />);
      expect(container.querySelector(h.selector), h.name).toBeNull();
      unmount();
    }
    // The image is a link with its alt text, so nothing is fetched from the host it names.
    render(<Markdown text={HOSTILE[3]!.text} />);
    expect(screen.getByRole("link", { name: "tracker" })).toHaveAttribute("href", "https://evil.example/pixel.png");
    expect((window as { pwned?: number }).pwned).toBeUndefined();
    console.log(`checked ${HOSTILE.length} hostile inputs`);
  });

  it("must fail: the same inputs through a renderer with no rules do reach the page", () => {
    // An image left as an image, and a URL filter that lets everything through.
    const open = (text: string) => render(createElement(ReactMarkdown, { urlTransform: (u: string) => u }, text)).container;
    expect(open(HOSTILE[3]!.text).querySelector("img")).not.toBeNull();
    expect(open(HOSTILE[2]!.text).querySelector('a[href^="javascript"]')).not.toBeNull();
  });
});

describe("a file of a plan draft", () => {
  it("a Markdown file is rendered, with the raw text one click away and back", async () => {
    render(<PlanText path="docs/plans/265.md" text={PLAN} />);
    expect(await screen.findByTestId("md-body")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("plan-text-raw"));
    expect(screen.queryByTestId("md-body")).toBeNull();
    expect(document.querySelector("pre.draft-text")).toHaveTextContent("| Node | Status |");
    fireEvent.click(screen.getByTestId("plan-text-raw"));
    expect(await screen.findByTestId("md-body")).toBeInTheDocument();
  });

  it("any other file is shown as written", () => {
    render(<PlanText path="schema/x.json" text={'{"a": "# not a heading"}'} />);
    expect(screen.queryByTestId("plan-text-raw")).toBeNull();
    expect(document.querySelector("pre.draft-text")).toHaveTextContent("# not a heading");
  });
});
