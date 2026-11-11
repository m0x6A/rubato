import { STATE_LABEL } from "../../shared/format.ts";
import { type OnPin, useIssueDrop } from "./drop.ts";
import { type GraphNode, issueCounts, sessionCounts } from "./graph.ts";

interface Props {
  node: GraphNode;
  selected: boolean;
  /** Called when an issue row is dropped on the card. Without it the card takes no drops. */
  onPin?: OnPin | undefined;
  /** Something else is selected and this part is not next to it. */
  dim?: boolean | undefined;
}

/** One part of the map. Renders without React Flow, so it is tested on its own. */
export function PartCard({ node, selected, onPin, dim }: Props) {
  const { over, handlers } = useIssueDrop<HTMLDivElement>({ kind: "node", id: node.id }, onPin);
  const issues = issueCounts(node.issues);
  const sessions = sessionCounts(node.sessions);
  const classes = ["map-node", node.kind, node.paint ? `paint-${node.paint}` : "", selected ? "selected" : "", dim ? "dim" : "", over ? "drop-over" : ""].filter(Boolean).join(" ");
  return (
    <div className={classes} data-testid={`node-${node.id}`} title={node.description} {...handlers}>
      <div className="title">
        <span className="id">{node.id}</span>
        <span className="kind">{node.kind}</span>
      </div>
      <div className="badges">
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
      </div>
      {node.findings.length > 0 && <div className="warn">⚠ {node.findings.length} finding{node.findings.length === 1 ? "" : "s"}</div>}
      {over && <div className="drop-hint">Drop to pin here</div>}
    </div>
  );
}
