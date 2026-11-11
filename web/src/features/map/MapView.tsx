import { Background, BaseEdge, Controls, type Edge, EdgeLabelRenderer, type EdgeProps, Handle, MarkerType, type Node, type NodeChange, type NodeProps, Position, ReactFlow, ReactFlowProvider, applyNodeChanges, getBezierPath, useReactFlow } from "@xyflow/react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { mergeNodes } from "./nodes.ts";
import type { MapRef } from "../../protocol.ts";
import type { OnPin } from "./drop.ts";
import { type Graph, type GraphEdge, type GraphNode, layout } from "./graph.ts";
import { PartCard } from "./PartCard.tsx";
import { SeamChip } from "./SeamChip.tsx";

type MapNodeData = { node: GraphNode; selected: boolean; onPin: OnPin | undefined };
type MapFlowNode = Node<MapNodeData, "map">;
type SeamEdgeData = { seam: GraphEdge; selected: boolean; onSelect: (id: string) => void; onPin: OnPin | undefined };
type SeamFlowEdge = Edge<SeamEdgeData, "seam">;

function MapNodeCard({ data }: NodeProps<MapFlowNode>) {
  return (
    <>
      <Handle type="target" position={Position.Left} style={{ opacity: 0 }} />
      <PartCard node={data.node} selected={data.selected} onPin={data.onPin} />
      <Handle type="source" position={Position.Right} style={{ opacity: 0 }} />
    </>
  );
}

/** A seam: the path with its arrowhead, and its chip as an HTML label, which can take a drop. */
function SeamEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, markerEnd, data }: EdgeProps<SeamFlowEdge>) {
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  return (
    <>
      <BaseEdge path={path} {...(markerEnd ? { markerEnd } : {})} />
      {data && (
        <EdgeLabelRenderer>
          <div className="seam-label" style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}>
            <SeamChip seam={data.seam} selected={data.selected} onSelect={data.onSelect} onPin={data.onPin} />
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

const nodeTypes = { map: MapNodeCard };
const edgeTypes = { seam: SeamEdge };

interface Props {
  graph: Graph;
  /** The selected part or seam. */
  selected: MapRef | null;
  onSelect: (ref: MapRef | null) => void;
  /** An issue row was dropped on a part or a seam chip. */
  onPin?: OnPin;
}

function Inner({ graph, selected, onSelect, onPin }: Props) {
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

  const selectedNode = selected?.kind === "node" ? selected.id : null;
  const selectedSeam = selected?.kind === "seam" ? selected.id : null;

  const wanted: MapFlowNode[] = useMemo(
    () =>
      graph.nodes.map((n) => ({
        id: n.id,
        type: "map",
        position: positions?.get(n.id) ?? { x: 0, y: 0 },
        data: { node: n, selected: selectedNode === n.id, onPin },
      })),
    [graph, positions, selectedNode, onPin],
  );
  // React Flow draws an edge only between parts it has measured, and it reports a measurement
  // once, through `onNodesChange`. So the nodes are kept in state and every refresh of the
  // overview updates them in place: a part that is rebuilt from scratch loses its measurement
  // and its seams (and their chips) disappear until something resizes it.
  const [nodes, setNodes] = useState<MapFlowNode[]>([]);
  useEffect(() => setNodes((current) => mergeNodes(current, wanted)), [wanted]);
  const onNodesChange = useCallback((changes: NodeChange<MapFlowNode>[]) => setNodes((current) => applyNodeChanges(changes, current)), []);
  const edges: SeamFlowEdge[] = useMemo(
    () =>
      graph.edges.map((e) => ({
        id: e.id,
        type: "seam",
        source: e.from,
        target: e.to,
        className: [e.collision ? "collision" : e.openPlans.length ? "busy" : "", selectedSeam === e.id ? "chosen" : ""].filter(Boolean).join(" "),
        markerEnd: { type: MarkerType.ArrowClosed, width: 18, height: 18 },
        data: { seam: e, selected: selectedSeam === e.id, onSelect: (id: string) => onSelect({ kind: "seam", id }), onPin },
      })),
    [graph, selectedSeam, onSelect, onPin],
  );

  if (!positions) {
    return <div className="empty">Laying out the map…</div>;
  }
  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      edgeTypes={edgeTypes}
      onNodesChange={onNodesChange}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable
      onNodeClick={(_e, n) => onSelect({ kind: "node", id: n.id })}
      onEdgeClick={(_e, edge) => onSelect({ kind: "seam", id: edge.id })}
      onPaneClick={() => onSelect(null)}
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
