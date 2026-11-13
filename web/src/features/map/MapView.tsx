import { Background, BaseEdge, Controls, type Edge, EdgeLabelRenderer, type EdgeProps, Handle, MarkerType, type Node, type NodeChange, type NodeProps, Position, ReactFlow, ReactFlowProvider, applyNodeChanges, getBezierPath, useReactFlow } from "@xyflow/react";
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { caption, playerReducer, resolveScenario } from "./scenario.ts";
import { mergeNodes } from "./nodes.ts";
import type { IssueRef, MapLayout, MapRef, Plan } from "../../protocol.ts";
import { issueKey } from "../../shared/format.ts";
import { planLens } from "./lens.ts";
import { type XY, placePart, positionsToSave } from "./layout.ts";
import { lit } from "./explain.ts";
import { type Arrow, LENSES, type Lens, arrowsFor, flowCounts } from "./flows.ts";
import type { OnPin } from "./drop.ts";
import { type DraftMarkLike, type Graph, type GraphEdge, type GraphNode, layout } from "./graph.ts";
import { PartCard } from "./PartCard.tsx";
import { SeamChip } from "./SeamChip.tsx";
import { FlowParticles, type ParticleMode, particleMode } from "./particles.tsx";
import { useReducedMotion } from "../../shared/motion.ts";

type MapNodeData = { node: GraphNode; selected: boolean; onPin: OnPin | undefined; dim: boolean; draft: DraftMarkLike | undefined; editing: boolean };
type MapFlowNode = Node<MapNodeData, "map">;
type SeamEdgeData = {
  seam: GraphEdge;
  arrow: Arrow;
  /** This arrow carries the seam's chip: one chip per seam in every lens. */
  chip: boolean;
  selected: boolean;
  onSelect: (id: string) => void;
  onPin: OnPin | undefined;
  draft: DraftMarkLike | undefined;
  /** What moves along this arrow: ambient particles, the scenario step's pulse, or nothing (issue #81). */
  particles: ParticleMode;
  /** Reduced motion: one still marker instead of anything that travels. */
  still: boolean;
};
type SeamFlowEdge = Edge<SeamEdgeData, "seam">;

function MapNodeCard({ data }: NodeProps<MapFlowNode>) {
  return (
    <>
      {/* In edit mode the handles show: drag from the right handle of one part to the left of another to draw a seam. */}
      <Handle type="target" position={Position.Left} className={data.editing ? "seam-handle" : "seam-handle hidden"} isConnectable={data.editing} />
      <PartCard node={data.node} selected={data.selected} onPin={data.onPin} dim={data.dim} draft={data.draft} />
      <Handle type="source" position={Position.Right} className={data.editing ? "seam-handle" : "seam-handle hidden"} isConnectable={data.editing} />
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
          {flow && flow.kind !== "structure" && (
            // Data is a filled dot, control a ring: the kind reads without colour. Particles travel
            // in the flow's own direction; with reduced motion one marker sits still (particles.tsx).
            <FlowParticles id={flow.id} path={path} kind={flow.kind} reversed={flow.reversed} mode={data.particles} reduced={data.still} length={Math.hypot(targetX - sourceX, targetY - sourceY) * 1.15} />
          )}
          {flow && (
            <div className={`seam-label flow-label ${flow.kind}`} data-testid={`flow-${flow.id}`} title={`${flow.kind} flow over ${data.seam.id}: ${label}`} style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}>
              <span aria-hidden="true">{flow.reversed ? "← " : ""}</span>
              {flow.moving && <span className="proposed-tag">proposed</span>}
              {label.length > FLOW_LABEL_MAX ? `${label.slice(0, FLOW_LABEL_MAX - 1)}…` : label}
              <span aria-hidden="true">{flow.reversed ? "" : " →"}</span>
            </div>
          )}
          {data.chip && (
            <div className="seam-label" style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY - (flow ? CHIP_ABOVE_FLOW : 0)}px)` }}>
              <SeamChip seam={data.seam} selected={data.selected} onSelect={data.onSelect} onPin={data.onPin} draft={data.draft} />
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
  /** The selected issue's plan: the map shows it and nothing else (the plan lens, design S11). */
  plan?: { issue: IssueRef; plan: Plan } | null | undefined;
  /** What the shared draft adds, changes or removes, by id (ADR 0018). */
  draftMarks?: ReadonlyMap<string, DraftMarkLike> | undefined;
  /** Edit mode: handles show, parts connect, a double-click on the canvas asks for a new part. */
  editing?: boolean | undefined;
  onToggleEdit?: (() => void) | undefined;
  /** A seam was drawn from one part to another. Nothing is added locally: the seam appears when the server has it. */
  onConnectParts?: ((from: string, to: string) => void) | undefined;
  /** The canvas was double-clicked in edit mode, at this canvas position. */
  onAddPartAt?: ((at: XY) => void) | undefined;
}

/** A burst of arrow-key moves, or the end of a drag, becomes one write after this pause. */
const SAVE_AFTER_MS = 250;
/** How long Play shows each step; one trip of the pulse is `PULSE_MS` in particles.tsx. */
const STEP_MS = 2400;

function Inner({ graph, selected, onSelect, onPin, layout: stored, onMove, onAutoArrange, plan, draftMarks, editing = false, onToggleEdit, onConnectParts, onAddPartAt }: Props) {
  const [positions, setPositions] = useState<Map<string, { x: number; y: number }> | null>(null);
  const { fitView, screenToFlowPosition, getNode, getViewport, setCenter } = useReactFlow();
  const canvasRef = useRef<HTMLDivElement>(null);
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

  const [chosenLens, setLens] = useState<Exclude<Lens, "plan">>("structure");
  const planView = useMemo(() => (plan ? planLens(plan.plan, graph.edges) : null), [plan, graph]);
  // While a plan is shown, the switch waits: the arrows are the plan's.
  const lens: Lens = planView ? "plan" : chosenLens;
  const counts = useMemo(() => flowCounts(graph.edges), [graph]);
  // The scenario lens (ADR 0024): one scenario, one step at a time.
  const reducedMotion = useReducedMotion();
  const [scenarioId, setScenarioId] = useState<string | null>(null);
  const scenario = useMemo(() => {
    const sc = graph.scenarios.find((s) => s.id === scenarioId) ?? graph.scenarios[0];
    return sc ? resolveScenario(sc, graph.edges) : null;
  }, [graph, scenarioId]);
  const [player, dispatch] = useReducer(playerReducer, { step: 0, playing: false });
  const total = scenario?.steps.length ?? 0;
  useEffect(() => dispatch({ type: "reset", total }), [scenario?.id, total]);
  useEffect(() => {
    if (!player.playing || lens !== "scenario") {
      return;
    }
    const t = setInterval(() => dispatch({ type: "tick", total }), STEP_MS);
    return () => clearInterval(t);
  }, [player.playing, lens, total]);
  const step = lens === "scenario" ? (scenario?.steps[player.step] ?? null) : null;
  // What stays lit while something is selected: it, its neighbours and the seams between them.
  // In the plan lens it is what the plan names; in the scenario lens, the current step.
  const litNow = useMemo(
    () => (planView ? { nodes: planView.litParts, seams: planView.litSeams } : step ? { nodes: new Set([step.from, step.to]), seams: new Set([step.seam]) } : lit(graph.edges, selected)),
    [graph, selected, planView, step],
  );
  // A new step whose parts are off screen brings them into view: the same zoom if both fit,
  // otherwise just enough to fit them. No animated pan under reduced motion.
  useEffect(() => {
    const el = canvasRef.current;
    const a = step ? getNode(step.from) : undefined;
    const b = step ? getNode(step.to) : undefined;
    if (!el || !a || !b) {
      return;
    }
    const boxes = [a, b].map((n) => ({ x: n.position.x, y: n.position.y, w: n.measured?.width ?? 0, h: n.measured?.height ?? 0 }));
    const { x, y, zoom } = getViewport();
    const width = el.clientWidth;
    const height = el.clientHeight;
    const margin = 24;
    const visible = boxes.every((r) => r.x * zoom + x >= margin && r.y * zoom + y >= margin && (r.x + r.w) * zoom + x <= width - margin && (r.y + r.h) * zoom + y <= height - margin);
    if (visible) {
      return;
    }
    const minX = Math.min(...boxes.map((r) => r.x));
    const minY = Math.min(...boxes.map((r) => r.y));
    const maxX = Math.max(...boxes.map((r) => r.x + r.w));
    const maxY = Math.max(...boxes.map((r) => r.y + r.h));
    const duration = reducedMotion ? 0 : 300;
    if ((maxX - minX) * zoom <= width - 2 * margin && (maxY - minY) * zoom <= height - 2 * margin) {
      void setCenter((minX + maxX) / 2, (minY + maxY) / 2, { zoom, duration });
    } else {
      void fitView({ nodes: [{ id: a.id }, { id: b.id }], padding: 0.2, duration });
    }
    // Only when the step changes: a pan the person makes afterwards is theirs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step?.arrow, step?.index]);
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
        // In the plan lens a part is painted by this plan only, not by every open plan.
        data: { node: planView ? { ...n, paint: planView.parts.get(n.id) ?? null } : n, selected: selectedNode === n.id, onPin, dim: litNow !== null && !litNow.nodes.has(n.id), draft: draftMarks?.get(n.id), editing },
      })),
    // `heldRevision` stands for `held`, a ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [graph, settled, selectedNode, onPin, heldRevision, litNow, planView, draftMarks, editing],
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
        arrowsFor(planView ? { ...e, flows: planView.flows.get(e.id) } : e, lens).map((arrow, i) => {
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
              arrow.moving ? "moving" : "",
              step?.arrow === arrow.id ? "step-active" : "",
              arrow.reversed ? "reversed" : "",
              draftMarks?.has(e.id) ? `draft draft-${draftMarks.get(e.id)?.state}` : "",
              litNow !== null && !litNow.seams.has(e.id) ? "dim" : "",
            ]
              .filter(Boolean)
              .join(" "),
            ...(arrow.reversed ? { markerStart: marker } : { markerEnd: marker }),
            data: { seam: e, arrow, chip: i === 0, selected: selectedSeam === e.id, onSelect: (id: string) => onSelect({ kind: "seam", id }), onPin, draft: draftMarks?.get(e.id), particles: particleMode(arrow, lens, step?.arrow === arrow.id), still: reducedMotion },
          };
        }),
      ),
    [graph, lens, selectedSeam, onSelect, onPin, litNow, planView, draftMarks, step, reducedMotion],
  );

  // The lens switch, the scenario player, the legend and the map actions sit in a band above the
  // canvas, never over it: no part is hidden under them.
  const band = (
    <div className="map-band" data-testid="map-band">
      <div className="map-band-row">
        <div className="lens" role="radiogroup" aria-label="What the arrows show">
          {LENSES.map((l) => (
            <button key={l.id} type="button" role="radio" aria-checked={lens === l.id} className={lens === l.id ? "active" : ""} onClick={() => setLens(l.id)} disabled={planView !== null} title={planView ? "Close the issue to choose what the arrows show" : undefined}>
              {l.label}
              {l.id !== "structure" && ` (${counts[l.id]})`}
            </button>
          ))}
          {graph.scenarios.length > 0 && (
            <button type="button" role="radio" aria-checked={lens === "scenario"} className={lens === "scenario" ? "active" : ""} onClick={() => setLens("scenario")} disabled={planView !== null} title={planView ? "Close the issue to choose what the arrows show" : "Play a journey through the system, step by step"}>
              Scenarios ({graph.scenarios.length})
            </button>
          )}
        </div>
        {(onAutoArrange || onToggleEdit) && (
          <div className="map-actions">
            {onToggleEdit && (
              <button type="button" className={editing ? "map-action active" : "map-action"} aria-pressed={editing} onClick={onToggleEdit} title="Change the map as one shared draft: add, change and remove parts and seams, then propose the draft as a pull request">
                Edit map
              </button>
            )}
            {onAutoArrange && (
              <button type="button" className="map-action" onClick={onAutoArrange} disabled={Object.keys(stored?.positions ?? {}).length === 0} title="Remove every saved position, for the whole team, and lay the map out automatically">
                Auto-arrange
              </button>
            )}
          </div>
        )}
      </div>
      {lens === "scenario" && scenario && (
        <div
          className="scenario-player"
          role="group"
          aria-label="Scenario player"
          tabIndex={0}
          onKeyDown={(e) => {
            if (e.key === "ArrowRight") {
              dispatch({ type: "next", total });
              e.preventDefault();
            } else if (e.key === "ArrowLeft") {
              dispatch({ type: "prev", total });
              e.preventDefault();
            }
          }}
        >
          <div className="map-band-row">
            <select aria-label="Scenario" value={scenario.id} onChange={(e) => setScenarioId(e.target.value)}>
              {graph.scenarios.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
            <span className="scenario-controls">
              <button type="button" aria-label="Previous step" onClick={() => dispatch({ type: "prev", total })} disabled={player.step === 0}>
                ‹
              </button>
              <button type="button" aria-label={player.playing ? "Pause" : "Play"} onClick={() => dispatch({ type: player.playing ? "pause" : "play", total })} disabled={total === 0}>
                {player.playing ? "❚❚" : "▶"}
              </button>
              <button type="button" aria-label="Next step" onClick={() => dispatch({ type: "next", total })} disabled={player.step >= total - 1}>
                ›
              </button>
            </span>
            <span className="scenario-count" aria-hidden="true">
              {total ? `${player.step + 1}/${total}` : "0/0"}
            </span>
            <span className="scenario-key" data-testid="legend">
              <i className="key-mark data" aria-hidden="true" /> data moves <i className="key-mark control" aria-hidden="true" />{" "}
              control: one side tells the other to act
            </span>
            <details className="scenario-steps">
              <summary>Steps</summary>
              <ol aria-label="Steps">
                {scenario.steps.map((s, i) => (
                  <li key={s.index}>
                    <button type="button" aria-current={i === player.step ? "step" : undefined} className={i === player.step ? "current" : ""} onClick={() => dispatch({ type: "goto", total, step: i })}>
                      <span className={`flow-kind ${s.kind}`}>{s.kind}</span> {s.from} → {s.to}: {s.carries}
                    </button>
                  </li>
                ))}
              </ol>
            </details>
          </div>
          <p className="scenario-caption" data-testid="scenario-caption" aria-live="polite">
            {caption(scenario, player.step)}
          </p>
          {scenario.unplaced > 0 && <p className="scenario-note">{scenario.unplaced} of its steps cannot be drawn: the map no longer declares their flow.</p>}
        </div>
      )}
      {lens !== "scenario" && (
        <p className="legend" data-testid="legend">
          {lens === "plan" && plan && planView && (
            <>
              <strong>The plan of {issueKey(plan.issue)}.</strong> Its parts are painted, the rest steps back
              {planView.flows.size > 0 ? (
                <>
                  , and what it will send is proposed:{" "}
                  <svg width="34" height="8" aria-hidden="true">
                    <line x1="0" y1="4" x2="34" y2="4" className="legend-line proposed" />
                  </svg>{" "}
                  its dashes move along dashed arrows; nothing travels on them yet.
                </>
              ) : (
                ". It declares no flows."
              )}
              {planView.unplaced > 0 && ` ${planView.unplaced} of its flows cannot be drawn: the seam does not join the two parts.`}
            </>
          )}
          {lens === "structure" && "Each arrow points from the part that depends to the part it depends on."}
          {lens === "data" && (
            <>
              <svg width="34" height="8" aria-hidden="true">
                <line x1="0" y1="4" x2="34" y2="4" className="legend-line" />
              </svg>{" "}
              <i className="key-mark data" aria-hidden="true" /> Data moves along a solid arrow as dots, in the arrow's direction. A faint seam declares no data flow.
            </>
          )}
          {lens === "control" && (
            <>
              <svg width="34" height="8" aria-hidden="true">
                <line x1="0" y1="4" x2="34" y2="4" className="legend-line" strokeDasharray="7 5" />
              </svg>{" "}
              <i className="key-mark control" aria-hidden="true" /> Control runs along a dashed arrow as rings: one side tells the other to act. A faint seam declares no control flow.
            </>
          )}
        </p>
      )}
    </div>
  );

  return (
    <div className="map-frame">
      {band}
      <div className="map-canvas" ref={canvasRef}>
        {!positions ? (
          <div className="empty">Laying out the map…</div>
        ) : (
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            onNodesChange={onNodesChange}
            nodesDraggable={Boolean(onMove)}
            nodesConnectable={editing}
            onConnect={(c) => onConnectParts?.(c.source, c.target)}
            zoomOnDoubleClick={!editing}
            onDoubleClick={(e) => {
              // Only the empty canvas: a double-click on a part or a label is not a new part.
              if (editing && onAddPartAt && e.target instanceof Element && e.target.classList.contains("react-flow__pane")) {
                const at = screenToFlowPosition({ x: e.clientX, y: e.clientY });
                onAddPartAt({ x: Math.round(at.x), y: Math.round(at.y) });
              }
            }}
            elementsSelectable
            onNodeClick={(_e, n) => onSelect({ kind: "node", id: n.id })}
            onEdgeClick={(_e, edge) => onSelect({ kind: "seam", id: edge.data?.seam.id ?? edge.id })}
            onPaneClick={() => onSelect(null)}
            proOptions={{ hideAttribution: true }}
            minZoom={0.2}
          >
            <Background gap={24} />
            <Controls showInteractive={false} />
          </ReactFlow>
        )}
      </div>
    </div>
  );
}

export function MapView(props: Props) {
  return (
    <ReactFlowProvider>
      <Inner {...props} />
    </ReactFlowProvider>
  );
}
