import { Background, BaseEdge, Controls, type Edge, EdgeLabelRenderer, type EdgeProps, Handle, MarkerType, type Node, type NodeChange, type NodeProps, Panel, Position, ReactFlow, ReactFlowProvider, applyNodeChanges, getBezierPath, useReactFlow } from "@xyflow/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { mergeNodes } from "./nodes.ts";
import type { MapLayout, MapRef } from "../../protocol.ts";
import { type XY, placePart, positionsToSave } from "./layout.ts";
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
  /** Where the team has put each part (ADR 0016). A part not in it is placed by ELK. */
  layout?: MapLayout | undefined;
  /** Parts came to rest somewhere new. Resolves once the write is answered and the overview is fresh. Without it parts do not move. */
  onMove?: ((moved: MapLayout) => Promise<void>) | undefined;
  /** Remove every stored position for the team. */
  onAutoArrange?: (() => void) | undefined;
}

/** A burst of arrow-key moves, or the end of a drag, becomes one write after this pause. */
const SAVE_AFTER_MS = 250;

function Inner({ graph, selected, onSelect, onPin, layout: stored, onMove, onAutoArrange }: Props) {
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

  // Where this browser holds a part while it drags it or waits for its own write to be answered.
  const held = useRef(new Map<string, XY>());
  const [heldRevision, setHeldRevision] = useState(0);
  // Where each part sits for the team (stored, else ELK): what a move is compared with.
  const settled = useMemo(() => new Map(graph.nodes.map((n) => [n.id, placePart({ auto: positions?.get(n.id), stored: stored?.positions?.[n.id], held: undefined })])), [graph, positions, stored]);
  const wanted: MapFlowNode[] = useMemo(
    () =>
      graph.nodes.map((n) => ({
        id: n.id,
        type: "map",
        position: held.current.get(n.id) ?? settled.get(n.id) ?? { x: 0, y: 0 },
        data: { node: n, selected: selectedNode === n.id, onPin },
      })),
    // `heldRevision` stands for `held`, a ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [graph, settled, selectedNode, onPin, heldRevision],
  );
  // React Flow draws an edge only between parts it has measured, and it reports a measurement
  // once, through `onNodesChange`. So the nodes are kept in state and every refresh of the
  // overview updates them in place: a part that is rebuilt from scratch loses its measurement
  // and its seams (and their chips) disappear until something resizes it.
  const [nodes, setNodes] = useState<MapFlowNode[]>([]);
  useEffect(() => setNodes((current) => mergeNodes(current, wanted)), [wanted]);
  const toSave = useRef<Record<string, XY>>({});
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const settledRef = useRef(settled);
  settledRef.current = settled;
  const onMoveRef = useRef(onMove);
  onMoveRef.current = onMove;
  const save = useCallback(() => {
    saveTimer.current = null;
    const positions = toSave.current;
    toSave.current = {};
    const move = onMoveRef.current;
    if (!move || Object.keys(positions).length === 0) {
      return;
    }
    // Answered (saved and the overview fresh, or refused): let go of the parts, unless they
    // moved again meanwhile. A refused part goes back to where the team has it.
    void move({ positions }).finally(() => {
      for (const [id, p] of Object.entries(positions)) {
        const now = held.current.get(id);
        if (now && now.x === p.x && now.y === p.y) {
          held.current.delete(id);
        }
      }
      setHeldRevision((r) => r + 1);
    });
  }, []);
  useEffect(
    () => () => {
      if (saveTimer.current) {
        clearTimeout(saveTimer.current);
      }
    },
    [],
  );
  const onNodesChange = useCallback(
    (changes: NodeChange<MapFlowNode>[]) => {
      setNodes((current) => applyNodeChanges(changes, current));
      for (const c of changes) {
        if (c.type === "position" && c.position && c.dragging) {
          held.current.set(c.id, c.position);
        }
      }
      const moved = positionsToSave(changes, settledRef.current);
      if (moved?.positions) {
        for (const [id, p] of Object.entries(moved.positions)) {
          if (p) {
            held.current.set(id, p);
            toSave.current[id] = p;
          }
        }
        if (saveTimer.current) {
          clearTimeout(saveTimer.current);
        }
        saveTimer.current = setTimeout(save, SAVE_AFTER_MS);
      }
    },
    [save],
  );
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
      nodesDraggable={Boolean(onMove)}
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
      {onAutoArrange && (
        <Panel position="top-right">
          <button type="button" className="map-action" onClick={onAutoArrange} disabled={Object.keys(stored?.positions ?? {}).length === 0} title="Remove every saved position, for the whole team, and lay the map out automatically">
            Auto-arrange
          </button>
        </Panel>
      )}
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
