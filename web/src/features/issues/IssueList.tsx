import { PinMenu } from "../pins/index.ts";
import { QueueMenu } from "../queue/index.ts";
import type { IssueRef, IssueSummary, MapRef, Me, Overview } from "../../protocol.ts";
import { setIssueDrag } from "../../shared/dnd.ts";
import { closedUnmerged, finishedIssues, openIssues } from "../../shared/finished.ts";
import { STATE_LABEL, issueKey } from "../../shared/format.ts";
import type { IssueList as Which } from "../../shared/route.ts";
import { Backlog } from "./Backlog.tsx";
import { ownLanes } from "./own.ts";
import { SignalBadges } from "./PlanSignals.tsx";
import { ProgressRing } from "./visuals.tsx";

interface Props {
  overview: Overview;
  me: Me | null;
  /** Which issues to list: the open ones Rubato tracks, the open ones nobody pinned, the finished. */
  list: Which;
  onList: (list: Which) => void;
  /** The part or seam selected on the map; the list shows the issues pinned to exactly it. */
  selection: MapRef | null;
  onClearSelection: () => void;
  onOpen: (issue: IssueRef) => void;
  /** Adds a pin; an issue may have several. The keyboard twin of dropping a row on the map. */
  onPin: (issue: IssueRef, target: MapRef) => void;
  /** Queue an issue on one of the caller's own companions. Without it no "Queue on…" is offered. */
  onQueue?: ((issue: IssueRef, companionId: string) => void) | undefined;
}

const ORDER = ["blocked", "unknown", "building", "designing", "designed", "started", "in_review", "planned", "done"] as const;

/** The open issues of the map's repos that Rubato does not track: nobody pinned them, and they have no plan. */
export function notPinned(overview: Overview): number {
  const tracked = new Set(openIssues(overview).map((i) => issueKey(i.issue)));
  return (overview.backlog?.issues ?? []).filter((i) => !tracked.has(issueKey(i.issue))).length;
}

/**
 * The issue list (issue #357): find the issue to work on, and open it. Three lists, one at a
 * time: Open (tracked, open on GitHub: the default, #265), Not pinned (open on GitHub, not on the
 * map yet) and Done. A row says where the issue stands; what else can be done with it shows when
 * the row is pointed at or focused, and dragging a row onto the map or a lane still works.
 */
export function IssueList({ overview, me, list, onList, selection, onClearSelection, onOpen, onPin, onQueue }: Props) {
  // The name of this workspace, when there are several: what "counts in" is told apart from.
  const hereName = (overview.workspaces ?? []).find((w) => w.id === overview.workspace)?.name;
  const lanes = onQueue ? ownLanes(overview, me) : [];
  const pinnedHere = (i: IssueSummary) => !selection || i.pins.some((p) => p.kind === selection.kind && p.id === selection.id);
  const allOpen = openIssues(overview);
  const open = allOpen.filter(pinnedHere).sort((a, b) => ORDER.indexOf(a.progress.state) - ORDER.indexOf(b.progress.state) || a.issue.number - b.issue.number);
  const done = finishedIssues(overview).filter(pinnedHere);
  const issues = list === "done" ? done : open;
  const what = selection ? `${selection.kind === "seam" ? "seam" : "part"} ${selection.id}` : "";
  const segment = (which: Which, label: string, n: number) => (
    <button type="button" className={list === which ? "active" : ""} aria-pressed={list === which} data-testid={`issues-view-${which}`} onClick={() => onList(which)}>
      {label} ({n})
    </button>
  );

  return (
    <section data-testid="issue-list">
      {selection && list !== "not-pinned" && (
        <p data-testid="map-filter">
          <span className="muted">pinned to {selection.kind === "seam" ? "seam" : "part"}</span> <strong>{selection.id}</strong> <button onClick={onClearSelection}>show all</button>
        </p>
      )}
      <div className="lens view-switch" role="group" aria-label="Which issues to list">
        {segment("open", "Open", open.length)}
        {segment("not-pinned", "Not pinned", notPinned(overview))}
        {segment("done", "Done", done.length)}
      </div>
      {list === "not-pinned" ? (
        <Backlog backlog={overview.backlog} map={overview.map.map} onPin={onPin} lanes={lanes} queue={overview.queue ?? []} onQueue={onQueue} listedAbove={new Set(allOpen.map((i) => issueKey(i.issue)))} />
      ) : (
        <ul className="plain" data-testid={list === "done" ? "issues-done" : "issues-open"}>
          {issues.map((i) => (
            <li
              key={issueKey(i.issue)}
              className={i.pins.length === 0 ? "issue-row to-pin" : "issue-row"}
              onClick={() => onOpen(i.issue)}
              data-testid={`issue-${i.issue.number}`}
              draggable={list === "open"}
              onDragStart={list === "open" ? (e) => setIssueDrag(e, i.issue) : undefined}
              title={list === "open" ? "Drag onto a part or a seam of the map to add a pin" : undefined}
            >
              {/* `issue-row-N` names the whole row for the e2e; `issue-N` is the older id on the same row. */}
              <div className="row" data-testid={`issue-row-${i.issue.number}`}>
                <ProgressRing state={i.progress.state} />
                <span className="issue-main">
                  {/* A real button, so the row opens from the keyboard; its click reaches the row's handler. */}
                  <button type="button" className="link">
                    {issueKey(i.issue)}
                  </button>{" "}
                  <span className="muted">{i.title ?? ""}</span>
                  <div className="muted">{i.pins.map((p) => p.id).join(", ") || "not pinned"}</div>
                </span>
                <span className="row-end">
                  <span className={`badge ${i.progress.state}`}>{STATE_LABEL[i.progress.state]}</span>
                  {closedUnmerged(i) && (
                    <span className="badge closed" title="Closed on GitHub without a merged pull request">
                      closed
                    </span>
                  )}
                  <SignalBadges issue={i} here={hereName} lean />
                  {list === "open" && (
                    <span className="row-actions">
                      <PinMenu map={overview.map.map} issue={i.issue} pinned={i.pins} onPin={onPin} />
                      {onQueue && <QueueMenu issue={i.issue} lanes={lanes} queue={overview.queue ?? []} onQueue={onQueue} />}
                    </span>
                  )}
                </span>
              </div>
            </li>
          ))}
          {issues.length === 0 && (
            <li className="muted" data-testid="no-tracked-issues">
              {list === "done"
                ? selection
                  ? `No finished issue is pinned to ${what}.`
                  : "Nothing finished yet."
                : selection
                  ? `No open issue is pinned to ${what}.`
                  : done.length > 0
                    ? `Nothing open on GitHub is tracked here. ${done.length} finished: see Done. Pin an issue from Not pinned to track it.`
                    : "No tracked issues yet. Under Not pinned, drag one onto a part or a seam of the map, or use its Pin to… menu."}
            </li>
          )}
        </ul>
      )}
      {list === "open" && (overview.unpinned_sessions ?? []).length > 0 && (
        <p className="muted">
          {(overview.unpinned_sessions ?? []).length} session{(overview.unpinned_sessions ?? []).length === 1 ? "" : "s"} on branches outside the convention (unpinned).
        </p>
      )}
    </section>
  );
}
