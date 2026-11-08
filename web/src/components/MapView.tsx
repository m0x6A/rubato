import { Background, Controls, type Edge, Handle, type Node, type NodeProps, Position, ReactFlow, ReactFlowProvider, useReactFlow } from "@xyflow/react";
import { useEffect, useMemo, useState } from "react";
import { type Graph, type GraphNode, STATE_LABEL, issueCounts, layout, sessionCounts } from "../graph.ts";

type MapNodeData = { node: GraphNode; selected: boolean };
type MapFlowNode = Node<MapNodeData, "map">;

function MapNodeCard({ data }: NodeProps<MapFlowNode>) {
  const { node, selected } = data;
  const issues = issueCounts(node.issues);
  const sessions = sessionCounts(node.sessions);
  const classes = ["map-node", node.kind, node.paint ? `paint-${node.paint}` : "", selected ? "selected" : ""].filter(Boolean).join(" ");
  return (
    <div className={classes} data-testid={`node-${node.id}`} title={node.description}>
      <Handle type="target" position={Position.Left} style={{ opacity: 0 }} />
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
      <Handle type="source" position={Position.Right} style={{ opacity: 0 }} />
    </div>
  );
}

const nodeTypes = { map: MapNodeCard };

interface Props {
  graph: Graph;
  selectedNode: string | null;
  onSelectNode: (id: string | null) => void;
}

function Inner({ graph, selectedNode, onSelectNode }: Props) {
  const [positions, setPositions] = useState<Map<string, { x: number; y: number }> | null>(null);
  const { fitView } = useReactFlow();
  const shape = graph.nodes.map((n) => n.id).join(",") + "|" + graph.edges.map((e) => `${e.from}>${e.to}`).join(",");

  useEffect(() => {
    let cancelled = false;
    layout(graph)
      .then((pos) => {
        if (!cancelled) {
          setPositions(pos);
          setTimeout(() => fitView({ padding: 0.15 }), 0);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setPositions(new Map(graph.nodes.map((n, i) => [n.id, { x: (i % 4) * 260, y: Math.floor(i / 4) * 140 }])));
        }
      });
    return () => {
      cancelled = true;
    };
    // Re-layout only when the set of nodes or edges changes, not on every refresh.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shape]);

  const nodes: MapFlowNode[] = useMemo(
    () =>
      graph.nodes.map((n) => ({
        id: n.id,
        type: "map",
        position: positions?.get(n.id) ?? { x: 0, y: 0 },
        data: { node: n, selected: selectedNode === n.id },
      })),
    [graph, positions, selectedNode],
  );
  const edges: Edge[] = useMemo(
    () =>
      graph.edges.map((e) => ({
        id: e.id,
        source: e.from,
        target: e.to,
        className: e.collision ? "collision" : e.openPlans.length ? "busy" : "",
        label: e.openPlans.length ? `${e.id} · ${e.openPlans.length} plan${e.openPlans.length === 1 ? "" : "s"}${e.collision ? " ⚠" : ""}` : e.id,
        labelBgPadding: [4, 2] as [number, number],
        labelStyle: { fontSize: 10 },
        data: { seam: e },
      })),
    [graph],
  );

  if (!positions) {
    return <div className="empty">Laying out the map…</div>;
  }
  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable
      onNodeClick={(_e, n) => onSelectNode(n.id)}
      onPaneClick={() => onSelectNode(null)}
      proOptions={{ hideAttribution: true }}
      minZoom={0.2}
    >
      <Background gap={24} />
      <Controls showInteractive={false} />
    </ReactFlow>
  );
}

export function MapView(props: Props) {
  return (
    <ReactFlowProvider>
      <Inner {...props} />
    </ReactFlowProvider>
  );
}
