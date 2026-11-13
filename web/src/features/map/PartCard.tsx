import { STATE_LABEL } from "../../shared/format.ts";
import { type OnPin, useIssueDrop } from "./drop.ts";
import { type DraftMarkLike, type GraphNode, approvalCounts, issueCounts, sessionCounts } from "./graph.ts";

interface Props {
  node: GraphNode;
  selected: boolean;
  /** Called when an issue row is dropped on the card. Without it the card takes no drops. */
  onPin?: OnPin | undefined;
  /** Something else is selected and this part is not next to it. */
  dim?: boolean | undefined;
  /** The shared draft adds, changes or removes this part: drawn dashed, with a badge. */
  draft?: DraftMarkLike | undefined;
}

/** The badge on a part or seam the draft touches. A draft id is not a map id: it takes no pins. */
export function DraftBadge({ draft }: { draft: DraftMarkLike }) {
  const what = draft.state === "added" ? "draft" : draft.state === "changed" ? "changed in draft" : "removed in draft";
  return (
    <span className="badge draft-badge" title={draft.state === "added" ? "Not on the map yet: it cannot be pinned until the draft is proposed and merged" : undefined}>
      {what} · {draft.author}
    </span>
  );
}

/** The order the strip shows issue states in: the progress model's, blocked and unknown last. */
const STRIP_ORDER = ["planned", "started", "designing", "designed", "building", "in_review", "done", "blocked", "unknown"] as const;

/** A small mark for the part's kind (issue #81): a context is a block, a contract two sheets, an external a dashed ring. */
function KindGlyph({ kind }: { kind: GraphNode["kind"] }) {
  return (
    <svg className="kind-glyph" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
      {kind === "contract" ? (
        <>
          <rect x="1.5" y="3.5" width="8" height="9" rx="1.5" />
          <rect x="4.5" y="1.5" width="8" height="9" rx="1.5" />
        </>
      ) : kind === "external" ? (
        <circle cx="7" cy="7" r="5.5" strokeDasharray="2.4 2" />
      ) : (
        <>
          <rect x="1.5" y="1.5" width="11" height="11" rx="2.5" />
          <rect x="4.5" y="4.5" width="5" height="5" rx="1" />
        </>
      )}
    </svg>
  );
}

/** One part of the map. Renders without React Flow, so it is tested on its own. */
export function PartCard({ node, selected, onPin, dim, draft }: Props) {
  // A part that exists only in the draft takes no pins (the server would refuse them).
  const { over, handlers } = useIssueDrop<HTMLDivElement>({ kind: "node", id: node.id }, draft?.state === "added" ? undefined : onPin);
  const issues = issueCounts(node.issues);
  const sessions = sessionCounts(node.sessions);
  const approvals = approvalCounts(node.issues);
  // Breathing means a session is working here now; unknown, idle or ended never breathe (AGENTS.md).
  const working = (sessions.working ?? 0) > 0;
  const classes = ["map-node", node.kind, node.paint ? `paint-${node.paint}` : "", working ? "work" : "", selected ? "selected" : "", dim ? "dim" : "", draft ? `draft draft-${draft.state}` : "", over ? "drop-over" : ""].filter(Boolean).join(" ");
  return (
    <div className={classes} data-testid={`node-${node.id}`} title={node.description} {...handlers}>
      <div className="title">
        <KindGlyph kind={node.kind} />
        <span className="id">{node.id}</span>
        <span className="kind">{node.kind}</span>
      </div>
      <div className="badges">
        {draft && <DraftBadge draft={draft} />}
        {Object.entries(issues).map(([state, n]) => (
          <span key={state} className={`badge ${state}`}>
            {n} {STATE_LABEL[state as keyof typeof STATE_LABEL].toLowerCase()}
          </span>
        ))}
        {Object.entries(sessions).map(([status, n]) => (
          <span key={status} className={`badge ${status}`} title="sessions">
            {n} session{n === 1 ? "" : "s"} {status}
          </span>
        ))}
        {node.paint && <span className={`badge ${node.paint === "new" ? "designing" : ""}`}>{node.paint} by a plan</span>}
        {node.planned.length > 0 && (
          <span className="badge planned-here" title={node.planned.map((i) => `#${i.issue.number}`).join(", ")}>
            {node.planned.length} planned here
          </span>
        )}
        {node.models > 0 && (
          <span className="badge models" title="data models the open plans introduce or change here">
            +{node.models} model{node.models === 1 ? "" : "s"}
          </span>
        )}
        {approvals.approved > 0 && <span className="badge approved">{approvals.approved} approved</span>}
        {approvals.stale > 0 && (
          <span className="badge approval-stale" title="the plan changed after it was approved">
            {approvals.stale} approval{approvals.stale === 1 ? "" : "s"} stale
          </span>
        )}
      </div>
      {node.findings.length > 0 && <div className="warn">⚠ {node.findings.length} finding{node.findings.length === 1 ? "" : "s"}</div>}
      {over && <div className="drop-hint">Drop to pin here</div>}
      {node.issues.length > 0 && (
        <div className="activity" aria-hidden="true">
          {STRIP_ORDER.filter((st) => issues[st]).map((st) => (
            <span key={st} data-state={st} className={st} style={{ flexGrow: issues[st] }} />
          ))}
        </div>
      )}
    </div>
  );
}
