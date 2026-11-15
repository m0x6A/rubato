import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { issue, openIssue, overview } from "../../../test/fixtures.ts";
import type { OpenPull } from "../../protocol.ts";
import { RepoWork } from "./RepoWork.tsx";

const pull = (n: number): OpenPull => ({ repo: "m0x6A/rubato", number: n, title: `Pull ${n}`, author: "ann", branch: `issue/${n}-x`, base: "main", draft: false, head_sha: `h${n}`, updated_at: new Date().toISOString(), url: `https://github.com/m0x6A/rubato/pull/${n}` });

describe("a repo's open pull requests and issues (issue #82)", () => {
  it("lists tracked and backlog issues without bodies, Pin to… only on backlog ones", () => {
    const ov = overview({
      issues: [issue(82, "rubato-server", { title: "Pulls in Rubato" })],
      backlog: { issues: [openIssue(90, "Waiting", { labels: ["bug"] })] },
      pulls: [{ repo: "m0x6A/rubato", pulls: [pull(5)], truncated: false }],
    });
    const opened: number[] = [];
    render(<RepoWork overview={ov} repo="m0x6A/rubato" onOpenIssue={(i) => opened.push(i.number)} onPin={() => {}} />);
    expect(screen.getByTestId("repo-work-m0x6A/rubato")).toHaveTextContent("1 open pull request · 2 open issues");
    const tracked = screen.getByTestId("repo-issue-82");
    const backlog = screen.getByTestId("repo-issue-90");
    expect(backlog).toHaveTextContent("bug");
    expect(within(backlog).getByRole("button", { name: "Pin m0x6A/rubato#90 to…" })).toBeInTheDocument();
    expect(within(tracked).queryByRole("button", { name: "Pin m0x6A/rubato#82 to…" })).toBeNull();
    fireEvent.click(within(tracked).getByRole("button", { name: "#82 Pulls in Rubato" }));
    expect(opened).toEqual([82]);
  });
});
