import { useMemo, useRef, useState } from "react";
import type { IssueRef, MapRef, NewIssueRequest, Overview, SystemMap } from "../../protocol.ts";
import { failureText } from "../../shared/api.ts";
import { issueKey } from "../../shared/format.ts";
import { newIssueApi } from "./api.ts";

interface Props {
  overview: Overview;
  /** Where it was started: preselected, and where the issue is pinned. */
  target: MapRef;
  onCreated: (issue: IssueRef) => void;
  onCancel: () => void;
  onNotice: (text: string) => void;
}

const lines = (text: string) =>
  text
    .split("\n")
    .map((l) => l.replace(/^\s*[-*]\s+(\[[ xX]\]\s+)?/, "").trim())
    .filter(Boolean);

/** Every repo the map names: `repos[]` and the nodes' own. */
function reposOf(map: SystemMap): string[] {
  return [...new Set([...(map.repos ?? []).map((r) => r.name), ...map.nodes.flatMap((n) => (n.repo ? [n.repo] : []))])].sort();
}

/** The repo an issue started on `target` goes to: the part's, or the seam's owner's (design A5). */
function repoFor(map: SystemMap, target: MapRef): string | undefined {
  const nodeId = target.kind === "node" ? target.id : (map.seams ?? []).find((s) => s.id === target.id)?.owner;
  return map.nodes.find((n) => n.id === nodeId)?.repo ?? undefined;
}

const encode = (t: MapRef) => `${t.kind}:${t.id}`;
const decode = (v: string): MapRef => {
  const [kind, ...rest] = v.split(":");
  return { kind: kind === "seam" ? "seam" : "node", id: rest.join(":") };
};

/**
 * "New issue here…" (ADR 0025): the issue under #71's headings, created by the GitHub App and
 * pinned where it was started. Each list is one line per item. When this Rubato cannot create
 * issues the form says why and sends nothing.
 */
export function NewIssueForm({ overview, target: start, onCreated, onCancel, onNotice }: Props) {
  const map = overview.map.map;
  const repos = useMemo(() => reposOf(map), [map]);
  // One id per draft: a second press of Create cannot make a second issue.
  const requestId = useRef(crypto.randomUUID());
  const [target, setTarget] = useState<MapRef>(start);
  const [repo, setRepo] = useState(repoFor(map, start) ?? repos[0] ?? "");
  const [title, setTitle] = useState("");
  const [summary, setSummary] = useState("");
  const [does, setDoes] = useState("");
  const [know, setKnow] = useState("");
  const [mustNotBreak, setMustNotBreak] = useState("");
  const [context, setContext] = useState("");
  const [questions, setQuestions] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const missing = [
    !title.trim() && "a title",
    !summary.trim() && "a summary",
    lines(does).length === 0 && "a line under What the change does",
    lines(know).length === 0 && "a line under How we'll know it works",
    lines(mustNotBreak).length === 0 && "a line under What it must not break",
  ].filter(Boolean) as string[];
  const can = overview.can_create_issues;

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!can || missing.length > 0 || busy) {
      return;
    }
    const req: NewIssueRequest = {
      request_id: requestId.current,
      repo,
      target,
      title: title.trim(),
      summary: summary.trim(),
      does: lines(does),
      know: lines(know),
      must_not_break: lines(mustNotBreak),
      ...(context.trim() ? { context: context.trim() } : {}),
      open_questions: lines(questions),
    };
    setBusy(true);
    setMessage(null);
    try {
      const r = await newIssueApi.create(req);
      if (r.state === "created" && r.issue) {
        onNotice(`Created ${issueKey(r.issue)}, pinned to ${target.kind === "seam" ? "seam" : "part"} ${target.id}.`);
        onCreated(r.issue);
      } else {
        setMessage(r.message ?? (r.state === "opening" ? "Still opening." : "It failed."));
      }
    } catch (err) {
      setMessage(failureText(err));
    } finally {
      setBusy(false);
    }
  };

  const area = (label: string, value: string, set: (v: string) => void, hint: string, rows = 3) => (
    <label>
      {label}
      <textarea rows={rows} value={value} onChange={(e) => set(e.target.value)} placeholder={hint} />
    </label>
  );

  return (
    <section className="inspector new-issue" data-testid="new-issue" aria-label="New issue">
      <header>
        <h2>New issue</h2>
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </header>
      {!can && (
        <p className="warn" data-testid="new-issue-unavailable">
          This Rubato cannot create issues: it needs a GitHub App installation with Issues: write. Until then, open the issue on GitHub and end its body with <code className="mono">Map: {target.kind} {target.id}</code>.
        </p>
      )}
      <form onSubmit={(e) => void submit(e)} aria-label="New issue">
        <label>
          Where it fits
          <select value={encode(target)} onChange={(e) => {
            const t = decode(e.target.value);
            setTarget(t);
            setRepo(repoFor(map, t) ?? repo);
          }}>
            <optgroup label="Parts">
              {map.nodes.map((n) => (
                <option key={n.id} value={encode({ kind: "node", id: n.id })}>
                  {n.id}
                </option>
              ))}
            </optgroup>
            <optgroup label="Seams">
              {(map.seams ?? []).map((s) => (
                <option key={s.id} value={encode({ kind: "seam", id: s.id })}>
                  {s.id}
                </option>
              ))}
            </optgroup>
          </select>
        </label>
        <label>
          Repository
          <select value={repo} onChange={(e) => setRepo(e.target.value)}>
            {repos.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        </label>
        <label>
          Title
          <input value={title} onChange={(e) => setTitle(e.target.value)} />
        </label>
        {area("Summary", summary, setSummary, "One or two sentences.")}
        {area("What the change does", does, setDoes, "One line per thing it does.")}
        {area("How we'll know it works", know, setKnow, "One line per check; each becomes a checkbox.")}
        {area("What it must not break", mustNotBreak, setMustNotBreak, "One line per thing that must keep working.")}
        {area("Context (optional)", context, setContext, "Why now, where it came from.", 2)}
        {area("Open questions (optional)", questions, setQuestions, "One line per question.", 2)}
        {can && missing.length > 0 && <p className="muted">Still needed: {missing.join(", ")}.</p>}
        {message && (
          <p className="warn" role="status" data-testid="new-issue-message">
            {message}
          </p>
        )}
        <button type="submit" disabled={!can || missing.length > 0 || busy}>
          {busy ? "Creating…" : `Create in ${repo}`}
        </button>
      </form>
    </section>
  );
}
