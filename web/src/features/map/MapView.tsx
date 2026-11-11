import { Background, BaseEdge, Controls, type Edge, EdgeLabelRenderer, type EdgeProps, Handle, MarkerType, type Node, type NodeChange, type NodeProps, Panel, Position, ReactFlow, ReactFlowProvider, applyNodeChanges, getBezierPath, useReactFlow } from "@xyflow/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { mergeNodes } from "./nodes.ts";
import type { MapLayout, MapRef } from "../../protocol.ts";
import { type XY, placePart, positionsToSave } from "./layout.ts";
import { lit } from "./explain.ts";
import { type Arrow, LENSES, type Lens, arrowsFor, flowCounts } from "./flows.ts";
import type { OnPin } from "./drop.ts";
import { type Graph, type GraphEdge, type GraphNode, layout } from "./graph.ts";
import { PartCard } from "./PartCard.tsx";
import { SeamChip } from "./SeamChip.tsx";

type MapNodeData = { node: GraphNode; selected: boolean; onPin: OnPin | undefined; dim: boolean };
type MapFlowNode = Node<MapNodeData, "map">;
type SeamEdgeData = { seam: GraphEdge; arrow: Arrow; /** This arrow carries the seam's chip: one chip per seam in every lens. */ chip: boolean; selected: boolean; onSelect: (id: string) => void; onPin: OnPin | undefined };
type SeamFlowEdge = Edge<SeamEdgeData, "seam">;

function MapNodeCard({ data }: NodeProps<MapFlowNode>) {
  return (
    <>
      <Handle type="target" position={Position.Left} style={{ opacity: 0 }} />
      <PartCard node={data.node} selected={data.selected} onPin={data.onPin} dim={data.dim} />
      <Handle type="source" position={Position.Right} style={{ opacity: 0 }} />
    </>
  );
}

/** How far above a flow's own label the seam's chip sits. */
const CHIP_ABOVE_FLOW = 24;
// A label must fit between two columns of parts (200 px apart); the title carries the whole text.
const FLOW_LABEL_MAX = 28;

/**
 * One arrow of a seam: the seam itself, or one declared flow. The path keeps the seam's own
 * geometry, shifted sideways so parallel flows do not overlap; a flow that runs against the
 * seam's direction has its head at the start. The seam's chip is an HTML label, which can take
 * a drop; a flow's label says what it carries.
 */
function SeamEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, markerEnd, markerStart, data }: EdgeProps<SeamFlowEdge>) {
  const offset = data?.arrow.offset ?? 0;
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY: sourceY + offset, targetX, targetY: targetY + offset, sourcePosition, targetPosition });
  const flow = data && data.arrow.kind !== "structure" ? data.arrow : null;
  const label = flow?.label ?? "";
  return (
    <>
      <BaseEdge path={path} {...(markerEnd ? { markerEnd } : {})} {...(markerStart ? { markerStart } : {})} {...(data?.arrow.dashed ? { style: { strokeDasharray: "7 5" } } : {})} />
      {data && (
        <EdgeLabelRenderer>
          {flow && (
            <div className={`seam-label flow-label ${flow.kind}`} data-testid={`flow-${flow.id}`} title={`${flow.kind} flow over ${data.seam.id}: ${label}`} style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}>
              <span aria-hidden="true">{flow.reversed ? "← " : ""}</span>
              {label.length > FLOW_LABEL_MAX ? `${label.slice(0, FLOW_LABEL_MAX - 1)}…` : label}
              <span aria-hidden="true">{flow.reversed ? "" : " →"}</span>
            </div>
          )}
          {data.chip && (
            <div className="seam-label" style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY - (flow ? CHIP_ABOVE_FLOW : 0)}px)` }}>
              <SeamChip seam={data.seam} selected={data.selected} onSelect={data.onSelect} onPin={data.onPin} />
            </div>
          )}
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

  const [lens, setLens] = useState<Lens>("structure");
  const counts = useMemo(() => flowCounts(graph.edges), [graph]);
  // What stays lit while something is selected: it, its neighbours and the seams between them.
  const litNow = useMemo(() => lit(graph.edges, selected), [graph, selected]);
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
        data: { node: n, selected: selectedNode === n.id, onPin, dim: litNow !== null && !litNow.nodes.has(n.id) },
      })),
    // `heldRevision` stands for `held`, a ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [graph, settled, selectedNode, onPin, heldRevision, litNow],
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
      graph.edges.flatMap((e) =>
        arrowsFor(e, lens).map((arrow, i) => {
          const marker = { type: MarkerType.ArrowClosed, width: 18, height: 18 };
          return {
            id: arrow.id,
            type: "seam" as const,
            source: e.from,
            target: e.to,
            className: [
              e.collision ? "collision" : e.openPlans.length ? "busy" : "",
              selectedSeam === e.id ? "chosen" : "",
              arrow.kind === "structure" ? "" : `flow ${arrow.kind}`,
              arrow.faint ? "faint" : "",
              litNow !== null && !litNow.seams.has(e.id) ? "dim" : "",
            ]
              .filter(Boolean)
              .join(" "),
            ...(arrow.reversed ? { markerStart: marker } : { markerEnd: marker }),
            data: { seam: e, arrow, chip: i === 0, selected: selectedSeam === e.id, onSelect: (id: string) => onSelect({ kind: "seam", id }), onPin },
          };
        }),
      ),
    [graph, lens, selectedSeam, onSelect, onPin, litNow],
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
      onEdgeClick={(_e, edge) => onSelect({ kind: "seam", id: edge.data?.seam.id ?? edge.id })}
      onPaneClick={() => onSelect(null)}
      proOptions={{ hideAttribution: true }}
      minZoom={0.2}
    >
      <Background gap={24} />
      <Controls showInteractive={false} />
      <Panel position="top-left">
        <div className="lens" role="radiogroup" aria-label="What the arrows show">
          {LENSES.map((l) => (
            <button key={l.id} type="button" role="radio" aria-checked={lens === l.id} className={lens === l.id ? "active" : ""} onClick={() => setLens(l.id)}>
              {l.label}
              {l.id !== "structure" && ` (${counts[l.id]})`}
            </button>
          ))}
        </div>
        <p className="legend" data-testid="legend">
          {lens === "structure" && "Each arrow points from the part that depends to the part it depends on."}
          {lens === "data" && (
            <>
              <svg width="34" height="8" aria-hidden="true">
                <line x1="0" y1="4" x2="34" y2="4" className="legend-line" />
              </svg>{" "}
              Data moves along a solid arrow, in the arrow's direction. A faint seam declares no data flow.
            </>
          )}
          {lens === "control" && (
            <>
              <svg width="34" height="8" aria-hidden="true">
                <line x1="0" y1="4" x2="34" y2="4" className="legend-line" strokeDasharray="7 5" />
              </svg>{" "}
              Control runs along a dashed arrow: one side tells the other to act. A faint seam declares no control flow.
            </>
          )}
        </p>
      </Panel>
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
