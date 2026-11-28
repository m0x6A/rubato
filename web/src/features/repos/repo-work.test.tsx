import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { issue, openIssue, overview } from "../../../test/fixtures.ts";
import type { OpenPull } from "../../protocol.ts";
import { ConnectedRepos, RepoView } from "./ConnectedRepos.tsx";
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
    const pulls: number[] = [];
    render(<RepoWork overview={ov} repo="m0x6A/rubato" onOpenIssue={(i) => opened.push(i.number)} onOpenPull={(n) => pulls.push(n)} onPin={() => {}} />);
    expect(screen.getByTestId("repo-work-pulls")).toHaveTextContent("1 open pull request");
    expect(screen.getByTestId("repo-work-issues")).toHaveTextContent("2 open issues");
    // A pull request opens its own view: the list hands its number up.
    fireEvent.click(within(screen.getByTestId("pull-row-5")).getByRole("button"));
    expect(pulls).toEqual([5]);
    expect(screen.queryByTestId("pull-view")).toBeNull();
    const tracked = screen.getByTestId("repo-issue-82");
    const backlog = screen.getByTestId("repo-issue-90");
    expect(backlog).toHaveTextContent("bug");
    expect(within(backlog).getByRole("button", { name: "Pin m0x6A/rubato#90 to…" })).toBeInTheDocument();
    expect(within(tracked).queryByRole("button", { name: "Pin m0x6A/rubato#82 to…" })).toBeNull();
    fireEvent.click(within(tracked).getByRole("button", { name: "#82 Pulls in Rubato" }));
    expect(opened).toEqual([82]);
  });
});

describe("the repos, and one repo (issue #357)", () => {
  const ov = () =>
    overview({
      repos: [
        { repo: "m0x6A/rubato", in_map: true, covered: true, open_issues: 2, plans: 1, sessions: 0, unattributed: 0 },
        { repo: "acme/other", in_map: false, covered: false, open_issues: 0, plans: 0, sessions: 0, unattributed: 0 },
      ],
      issues: [issue(82, "rubato-server", { title: "Pulls in Rubato" })],
      pulls: [{ repo: "m0x6A/rubato", pulls: [pull(5)], truncated: false }],
    });

  it("the list shows each repo's standing and opens a repo; its pull requests and issues are not on the list", () => {
    const opened: string[] = [];
    render(<ConnectedRepos overview={ov()} onOpenRepo={(r) => opened.push(r)} />);
    expect(screen.getByTestId("repo-m0x6A/rubato")).toHaveTextContent("covered");
    expect(screen.queryByTestId("pull-list")).toBeNull();
    expect(screen.queryByTestId("repo-issues")).toBeNull();
    expect(screen.queryByTestId("provides-m0x6A/rubato")).toBeNull();
    fireEvent.click(screen.getByTestId("repo-open-m0x6A/rubato"));
    expect(opened).toEqual(["m0x6A/rubato"]);
  });

  it("a repo's view holds its pull requests, its issues and what it provides, open to read", () => {
    render(<RepoView overview={ov()} repo="M0X6A/rubato" onBack={() => {}} onOpenPull={() => {}} />);
    const view = screen.getByTestId("repo-view");
    expect(within(view).getByTestId("pull-row-5")).toBeInTheDocument();
    expect(within(view).getByTestId("repo-issue-82")).toBeInTheDocument();
    expect(within(view).getByTestId("provides-m0x6A/rubato")).toHaveAttribute("open");
    // In its own view the name is a name, not a way to where the person already is.
    expect(within(view).queryByTestId("repo-open-m0x6A/rubato")).toBeNull();
  });

  it("a repo the map does not name says so; one outside the map lists nothing", () => {
    const { unmount } = render(<RepoView overview={ov()} repo="nobody/nothing" onBack={() => {}} onOpenPull={() => {}} />);
    expect(screen.getByTestId("repo-missing")).toHaveTextContent("nobody/nothing is not a repo this workspace's map names.");
    unmount();
    render(<RepoView overview={ov()} repo="acme/other" onBack={() => {}} onOpenPull={() => {}} />);
    expect(screen.getByTestId("repo-view")).toHaveTextContent("The map names no part in this repo");
    expect(screen.queryByTestId("repo-work-acme/other")).toBeNull();
  });
});
