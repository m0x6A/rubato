import { Background, BaseEdge, Controls, type Edge, EdgeLabelRenderer, type EdgeProps, Handle, MarkerType, type Node, type NodeChange, type NodeProps, Position, ReactFlow, ReactFlowProvider, applyNodeChanges, getBezierPath, useReactFlow } from "@xyflow/react";
import { type DragEvent, type ReactNode, useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { caption, playerReducer, resolveScenario } from "./scenario.ts";
import type { IssueRef, MapLayout, MapRef, Plan } from "../../protocol.ts";
import { issueKey } from "../../shared/format.ts";
import { planLens } from "./lens.ts";
import { type XY, placePart, positionsToSave } from "./layout.ts";
import { lit } from "./explain.ts";
import { type Arrow, LENSES, type Lens, arrowsFor, flowCounts } from "./flows.ts";
import { type OnPin, useIssueDrop } from "./drop.ts";
import { hasIssueDrag } from "../../shared/dnd.ts";
import { type Grouping, boxRepo, clampInBox, groupedToSave, placeGrouped, refitBoxes, refreshNodes, savedIds, settle, settledGrouped } from "./grouped.ts";
import { type Bundle, badgeState, bundleKey, bundlesOf, isOpen as bundleIsOpen } from "./bundles.ts";
import { type LabelStack, type Rect, bezierAt, placeLabels, textWidth } from "./labels.ts";
import { type DraftMarkLike, type Graph, type GraphEdge, type GraphNode, NODE_HEIGHT, NODE_WIDTH, layout, layoutByRepo } from "./graph.ts";
import { PartCard } from "./PartCard.tsx";
import { SeamChip } from "./SeamChip.tsx";
import { FlowParticles, type ParticleMode, particleBudget, particleMode } from "./particles.tsx";
import { type PartTraffic, partTrafficOf, quietClass, quietState, seamChip, TerritoryMark, TrafficChevrons, TrafficVitals, chevronCount, chevronsOn, chipTraffic, compactRate, health, liveTier, partRates, perMinute, trafficLegend, trafficVitals, unknownSentence } from "./traffic.tsx";
import { windowText } from "../findings/index.ts";
import type { GhostPart, UndeclaredEdge } from "../../protocol.ts";
import { endId, endText, ghostBand, ghostId, ghostsToDraw, isGhostId, serviceText } from "./territory.ts";
import { useReducedMotion } from "../../shared/motion.ts";

type MapNodeData = { node: GraphNode; selected: boolean; onPin: OnPin | undefined; dim: boolean; draft: DraftMarkLike | undefined; editing: boolean; traffic: PartTraffic | undefined };
type MapFlowNode = Node<MapNodeData, "map">;
/** A repo's box in the By repo grouping (issue #89): behind its parts, never selected; moved by its name bar (issue #144). */
type RepoFlowNode = Node<{ repo: string; movable: boolean }, "repo">;
/** A service seen in telemetry that no part names (issue #140): drawn in the Traffic lens, never a part. */
type GhostFlowNode = Node<{ ghost: GhostPart; windowSecs: number }, "ghost">;
type AnyFlowNode = MapFlowNode | RepoFlowNode | GhostFlowNode;
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
  /** In the Traffic lens on a fresh report: chevrons each way (issue #104). */
  chevrons: { forward: number; reverse: number } | null;
  /** In the Traffic lens: what the chip adds, the rate or "no calls in 1 h". */
  chipTraffic: string | undefined;
  /** In the By repo grouping, the bundle this seam belongs to while it is open (issue #144): pointing at it keeps it open. */
  bundle?: string | undefined;
  /** In the By repo grouping: where along the curve the labels sit, clear of cards and other labels (labels.ts). */
  labelT?: number | undefined;
};
type SeamFlowEdge = Edge<SeamEdgeData, "seam">;
type BundleEdgeData = { labelT: number | undefined; onPin: OnPin | undefined; bundle: Bundle; inDraft: (id: string) => boolean; dim: boolean; onToggle: (key: string, fromKeyboard: boolean) => void; onDragOpen: (key: string) => void };
type BundleFlowEdge = Edge<BundleEdgeData, "bundle">;
type TerritoryEdgeData = { edge: UndeclaredEdge; windowSecs: number; reduced: boolean; still: boolean };
type TerritoryFlowEdge = Edge<TerritoryEdgeData, "territory">;

function MapNodeCard({ data }: NodeProps<MapFlowNode>) {
  return (
    <>
      {/* In edit mode the handles show: drag from the right handle of one part to the left of another to draw a seam. */}
      <Handle type="target" position={Position.Left} className={data.editing ? "seam-handle" : "seam-handle hidden"} isConnectable={data.editing} />
      <PartCard node={data.node} selected={data.selected} onPin={data.onPin} dim={data.dim} draft={data.draft} traffic={data.traffic} />
      <Handle type="source" position={Position.Right} className={data.editing ? "seam-handle" : "seam-handle hidden"} isConnectable={data.editing} />
    </>
  );
}

/** How far above a flow's own label the seam's chip sits. */
const CHIP_ABOVE_FLOW = 24;

/**
 * One arrow of a seam: the seam itself, or one declared flow. The path keeps the seam's own
 * geometry, shifted sideways so parallel flows do not overlap; a flow that runs against the
 * seam's direction has its head at the start. The seam's chip is an HTML label, which can take
 * a drop; a flow's label says what it carries.
 */
/** About how wide a flow's label is drawn: its text clipped at 27 ch, the arrow, a proposed tag. */
function flowLabelWidth(a: { label?: string | null | undefined; moving?: boolean | undefined }): number {
  return textWidth(Math.min((a.label ?? "").length, 27) + 4 + (a.moving ? 9 : 0), 10.5, 16);
}

function SeamEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, markerEnd, markerStart, data }: EdgeProps<SeamFlowEdge>) {
  const offset = data?.arrow.offset ?? 0;
  const [path, midX, midY] = getBezierPath({ sourceX, sourceY: sourceY + offset, targetX, targetY: targetY + offset, sourcePosition, targetPosition });
  const at = data?.labelT !== undefined ? bezierAt({ x: sourceX, y: sourceY + offset }, { x: targetX, y: targetY + offset }, data.labelT) : { x: midX, y: midY };
  const [labelX, labelY] = [at.x, at.y];
  const flow = data && data.arrow.kind !== "structure" ? data.arrow : null;
  const label = flow?.label ?? "";
  // An open bundle stays open while the pointer is on one of its labels (issue #144): the
  // pointer tracker in the map reads `data-bundle` under the pointer.
  const keepOpen = data?.bundle ? { "data-bundle": data.bundle } : {};
  return (
    <>
      <path d={path} className="conduit" />
      <BaseEdge path={path} {...(markerEnd ? { markerEnd } : {})} {...(markerStart ? { markerStart } : {})} {...(data?.arrow.dashed ? { style: { strokeDasharray: "7 5" } } : {})} />
      {data && (
        <EdgeLabelRenderer>
          {data.chevrons && (
            // Observed calls, as chevrons pointing the way they go (issue #104, design §5.2).
            <TrafficChevrons id={data.seam.id} path={path} forward={data.chevrons.forward} reverse={data.chevrons.reverse} reduced={data.still} length={Math.hypot(targetX - sourceX, targetY - sourceY) * 1.15} />
          )}
          {flow && flow.kind !== "structure" && (
            // Data is a filled dot, control a ring: the kind reads without colour. Particles travel
            // in the flow's own direction; with reduced motion one marker sits still (particles.tsx).
            <FlowParticles id={flow.id} path={path} kind={flow.kind} reversed={flow.reversed} mode={data.particles} reduced={data.still} length={Math.hypot(targetX - sourceX, targetY - sourceY) * 1.15} label={{ at: data.labelT ?? 0.5, half: flowLabelWidth(flow) / 2 }} />
          )}
          {flow && (
            // The whole text is in the label; CSS clips it to a layer's gap and lifts the clip while
            // it is hovered or focused, so every label can be read in place (issue #144).
            <div className={`seam-label flow-label ${flow.kind}`} data-testid={`flow-${flow.id}`} tabIndex={0} title={`${flow.kind} flow over ${data.seam.id}: ${label}`} style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }} {...keepOpen}>
              <span aria-hidden="true">{flow.reversed ? "← " : ""}</span>
              {flow.moving && <span className="proposed-tag">proposed</span>}
              <span className="flow-text">{label}</span>
              <span aria-hidden="true">{flow.reversed ? "" : " →"}</span>
            </div>
          )}
          {data.chip && (
            <div className="seam-label" style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY - (flow ? CHIP_ABOVE_FLOW : 0)}px)` }} {...keepOpen}>
              <SeamChip seam={data.seam} selected={data.selected} onSelect={data.onSelect} onPin={data.onPin} draft={data.draft} traffic={data.chipTraffic} />
            </div>
          )}
        </EdgeLabelRenderer>
      )}
    </>
  );
}

/**
 * Calls between two parts no seam joins (issue #104, design §5.4): a static dotted amber line
 * with the calls' chevrons and a label. Not a seam: it cannot be selected, pinned or dropped on.
 */
function TerritoryEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps<TerritoryFlowEdge>) {
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  return (
    <>
      <BaseEdge path={path} interactionWidth={0} />
      {data && (
        <EdgeLabelRenderer>
          <TerritoryMark edge={data.edge} windowSecs={data.windowSecs} path={path} labelX={labelX} labelY={labelY} reduced={data.reduced} still={data.still} length={Math.hypot(targetX - sourceX, targetY - sourceY) * 1.15} />
        </EdgeLabelRenderer>
      )}
    </>
  );
}

/**
 * A ghost part (issue #140): dashed, amber like the territory it belongs to, with the identity
 * it emits and its calls. No handles to draw a seam from, no drop, no pin, no plan.
 */
function GhostCard({ data }: NodeProps<GhostFlowNode>) {
  const g = data.ghost;
  const calls = g.calls_in + g.calls_out;
  const perMin = perMinute(calls, data.windowSecs);
  return (
    <>
      <Handle type="target" position={Position.Left} className="seam-handle hidden" isConnectable={false} />
      <div className="ghost-card" data-testid={`ghost-${serviceText(g.service)}`} title={`${serviceText(g.service)} is running and calling, but no part on the map says it is this service`}>
        <span className="ghost-tag">not on the map</span>
        <strong>{g.service.name}</strong>
        {g.service.namespace && <span className="muted">{g.service.namespace}</span>}
        <span className="muted">
          {calls} calls · {perMin < 0.1 ? "<0.1" : Math.round(perMin * 10) / 10}/min
        </span>
      </div>
      <Handle type="source" position={Position.Right} className="seam-handle hidden" isConnectable={false} />
    </>
  );
}

/**
 * The seams between two parts of one repo, folded (issue #144, decision 3): one plain line and a
 * badge that says how many and keeps the strongest warning. Pointing at it opens it; a click (or
 * Enter) keeps it open; an issue dragged over it opens it until the drop.
 */
function BundleEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps<BundleFlowEdge>) {
  const [path, midX, midY] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  // A bundle of one seam takes an issue dropped on its badge for that seam, as the seam's chip
  // would: nothing has to open under the dragged issue. A bundle of several opens instead, and
  // stays open until the drop, so the issue can be taken to the right chip.
  const only = data && data.bundle.seams.length === 1 ? data.bundle.seams[0] : undefined;
  const drop = useIssueDrop<HTMLButtonElement>({ kind: "seam", id: only?.id ?? "" }, only && !data?.inDraft(only.id) ? data?.onPin : undefined);
  if (!data) {
    return null;
  }
  const { x: labelX, y: labelY } = data.labelT !== undefined ? bezierAt({ x: sourceX, y: sourceY }, { x: targetX, y: targetY }, data.labelT) : { x: midX, y: midY };
  const b = data.bundle;
  const st = badgeState(b, data.inDraft);
  const classes = ["seam-chip", "bundle-badge", "nodrag", "nopan", st.tone, st.draft ? "draft" : "", drop.over ? "drop-over" : ""].filter(Boolean).join(" ");
  return (
    <>
      <BaseEdge path={path} className="bundle-line" />
      <EdgeLabelRenderer>
        <div className={data.dim ? "seam-label bundle-label dim" : "seam-label bundle-label"} data-bundle={b.key} style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}>
          <button
            type="button"
            className={classes}
            data-testid={`bundle-${b.key}`}
            title={st.title}
            aria-label={`${st.text} between ${b.from} and ${b.to}${st.warn ? ", with a warning" : ""}: show`}
            onClick={(e) => {
              e.stopPropagation();
              // A click from the keyboard has no pointer position: detail is 0.
              data.onToggle(b.key, e.detail === 0);
            }}
            {...(only
              ? drop.handlers
              : {
                  onDragEnter: (e: DragEvent<HTMLButtonElement>) => {
                    if (hasIssueDrag(e)) {
                      data.onDragOpen(b.key);
                    }
                  },
                })}
          >
            {st.text}
            {st.warn && <span aria-hidden="true"> ⚠</span>}
          </button>
        </div>
      </EdgeLabelRenderer>
    </>
  );
}

function RepoBox({ data }: NodeProps<RepoFlowNode>) {
  return (
    <div className="repo-box" data-testid={`repo-box-${data.repo}`}>
      {/* The name bar is the box's handle: the box's body lets clicks and pans through to the canvas. */}
      <span className={data.movable ? "repo-box-name repo-box-frame" : "repo-box-name"} title={data.movable ? "Drag to move this repo with its parts, for everyone" : undefined}>
        {data.repo}
      </span>
    </div>
  );
}

const nodeTypes = { map: MapNodeCard, repo: RepoBox, ghost: GhostCard };
const edgeTypes = { seam: SeamEdge, territory: TerritoryEdge, bundle: BundleEdge };

/** How long a bundle stays open after the pointer leaves it, so the pointer can reach a chip (A2). */
const BUNDLE_CLOSE_MS = 400;

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
  /** Remove every stored position for the team, of the flat map or of the By repo grouping (ADR 0068). */
  onAutoArrange?: ((arrangement: "flat" | "grouped") => void) | undefined;
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
  /** Shown under the Traffic lens's legend: what to do about each discrepancy (issue #140). */
  trafficActions?: ReactNode | undefined;
  /** Opens Configure workspace: offered where the map says something is set there. */
  onConfigure?: (() => void) | undefined;
}

/** A burst of arrow-key moves, or the end of a drag, becomes one write after this pause. */
const SAVE_AFTER_MS = 250;
/** How long Play shows each step; one trip of the pulse is `PULSE_MS` in particles.tsx. */
const STEP_MS = 2400;

function Inner({ graph, selected, onSelect, onPin, layout: stored, onMove, onAutoArrange, plan, draftMarks, editing = false, onToggleEdit, onConnectParts, onAddPartAt, trafficActions, onConfigure }: Props) {
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

  // The By repo grouping (issue #89): each repo's parts in one box. ELK places it; the team's own
  // arrangement of it (ADR 0068) is stored apart from the flat map's, which it never touches.
  const [byRepo, setByRepo] = useState(false);
  const [grouped, setGrouped] = useState<Grouping | null>(null);
  useEffect(() => {
    if (!byRepo) {
      setGrouped(null);
      return;
    }
    let cancelled = false;
    layoutByRepo(graph)
      .then((g) => {
        if (!cancelled) {
          setGrouped(g);
          // Fit once the boxes are on the canvas: two frames, so React has committed them and
          // React Flow has measured them. A timeout of 0 could fit the flat map's bounds, and a
          // tall box then sat with its name bar above the visible canvas (issue #146).
          requestAnimationFrame(() => requestAnimationFrame(() => void fitView({ padding: 0.15 })));
        }
      })
      .catch(() => {
        if (!cancelled) {
          setByRepo(false);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shape, byRepo]);
  const grouping = byRepo && grouped !== null && !editing;
  // Where this browser holds a grouped part or box (by node id) while it drags it or waits for its write.
  const heldGrouped = useRef(new Map<string, XY>());
  const [groupedRevision, setGroupedRevision] = useState(0);
  const placed = useMemo(
    () => (grouped ? placeGrouped({ elk: grouped, layout: stored, held: heldGrouped.current }) : null),
    // `groupedRevision` stands for `heldGrouped`, a ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [grouped, stored, groupedRevision],
  );
  const placedRef = useRef(placed);
  placedRef.current = placed;
  const shownGrouped = useMemo(() => (grouped ? settledGrouped(grouped, stored) : new Map<string, XY>()), [grouped, stored]);
  const shownGroupedRef = useRef(shownGrouped);
  shownGroupedRef.current = shownGrouped;
  // Bundles of seams inside one repo (issue #144): which are open, and why.
  const [hoveredBundles, setHoveredBundles] = useState<ReadonlySet<string>>(new Set());
  const [pinnedBundles, setPinnedBundles] = useState<ReadonlySet<string>>(new Set());
  const [dragBundle, setDragBundle] = useState<string | null>(null);
  const closeTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const onBundleHover = useCallback((key: string, on: boolean) => {
    const t = closeTimers.current.get(key);
    if (on) {
      if (t) {
        clearTimeout(t);
        closeTimers.current.delete(key);
      }
      setHoveredBundles((s) => (s.has(key) ? s : new Set([...s, key])));
      return;
    }
    if (t) {
      return;
    }
    closeTimers.current.set(
      key,
      setTimeout(() => {
        closeTimers.current.delete(key);
        setHoveredBundles((s) => new Set([...s].filter((k) => k !== key)));
      }, BUNDLE_CLOSE_MS),
    );
  }, []);
  const focusAfterOpen = useRef<string | null>(null);
  const onBundleToggle = useCallback((key: string, fromKeyboard: boolean) => {
    setPinnedBundles((s) => (s.has(key) ? new Set([...s].filter((k) => k !== key)) : new Set([...s, key])));
    // From the keyboard, the badge gives way to the chips: focus moves to the first one.
    focusAfterOpen.current = fromKeyboard ? key : null;
  }, []);
  // Which bundles the pointer is on, read from what lies under it on every move: a badge that
  // gives way to its chips is gone before it could hear the pointer leave, and a chip that
  // appears under a still pointer never hears it enter.
  const hoveredRef = useRef(hoveredBundles);
  hoveredRef.current = hoveredBundles;
  useEffect(() => {
    if (!grouping) {
      return;
    }
    let frame = 0;
    let at: { x: number; y: number } | null = null;
    const check = () => {
      frame = 0;
      if (!at) {
        return;
      }
      const under = new Set(
        document
          .elementsFromPoint(at.x, at.y)
          .map((e) => e.closest("[data-bundle]")?.getAttribute("data-bundle"))
          .filter((k): k is string => Boolean(k)),
      );
      for (const k of under) {
        onBundleHover(k, true);
      }
      for (const k of hoveredRef.current) {
        if (!under.has(k)) {
          onBundleHover(k, false);
        }
      }
    };
    const move = (e: PointerEvent) => {
      at = { x: e.clientX, y: e.clientY };
      if (!frame) {
        frame = requestAnimationFrame(check);
      }
    };
    window.addEventListener("pointermove", move);
    return () => {
      window.removeEventListener("pointermove", move);
      if (frame) {
        cancelAnimationFrame(frame);
      }
    };
  }, [grouping, onBundleHover]);
  // Pointer events do not fire during a native drag: a bundle an issue was dragged onto stays open until the drop or the end of the drag.
  useEffect(() => {
    const end = () => setDragBundle(null);
    document.addEventListener("drop", end);
    document.addEventListener("dragend", end);
    return () => {
      document.removeEventListener("drop", end);
      document.removeEventListener("dragend", end);
    };
  }, []);
  useEffect(
    () => () => {
      for (const t of closeTimers.current.values()) {
        clearTimeout(t);
      }
    },
    [],
  );

  // Null until someone picks a lens: the map opens on Traffic while a fresh report is in, else
  // on Structure. Observed traffic is the first thing to see when there is any.
  const [pickedLens, setLens] = useState<Exclude<Lens, "plan"> | null>(null);
  const trafficFresh = graph.traffic?.state === "fresh";
  const chosenLens: Exclude<Lens, "plan"> = pickedLens ?? (trafficFresh ? "traffic" : "structure");
  const planView = useMemo(() => (plan ? planLens(plan.plan, graph.edges) : null), [plan, graph]);
  // While a plan is shown, the switch waits: the arrows are the plan's.
  const lens: Lens = planView ? "plan" : chosenLens;
  const counts = useMemo(() => flowCounts(graph.edges), [graph]);
  // In the Traffic lens on a fresh report: each part's calls a minute, in and out, and how they fare.
  const partTraffic = useMemo(() => (chevronsOn(lens, graph.traffic?.state ?? "not_configured") && graph.traffic ? partRates(graph.traffic, graph.edges) : null), [lens, graph]);
  const vitalsAt = useMemo(() => (trafficFresh && graph.traffic ? trafficVitals(graph.traffic, graph.edges.filter((e) => e.quietFor !== undefined).length) : null), [trafficFresh, graph]);
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
  // The observed system (issue #140): ghosts and their edges, in the Traffic lens on a fresh
  // report, on the flat map. They sit in a band right of the declared parts, which never move
  // for them, and never reach the shared layout.
  const ghostView = useMemo(
    () => (chevronsOn(lens, graph.traffic?.state ?? "not_configured") && !grouping ? ghostsToDraw(graph.traffic, new Set(graph.nodes.map((n) => n.id))) : { ghosts: [], edges: [] }),
    [graph, lens, grouping],
  );
  const ghostAt = useMemo(() => ghostBand(settled, ghostView.ghosts), [settled, ghostView]);
  // The band sits outside the view fitted to the declared map: bring it in when ghosts appear,
  // and back to the map when they go. The parts themselves do not move.
  const ghostCount = ghostView.ghosts.length;
  const hadGhosts = useRef(false);
  useEffect(() => {
    if (ghostCount > 0 !== hadGhosts.current) {
      hadGhosts.current = ghostCount > 0;
      setTimeout(() => fitView({ padding: 0.15 }), 0);
    }
  }, [ghostCount, fitView]);
  const wanted: AnyFlowNode[] = useMemo(
    () => [
      // Boxes first: React Flow draws a parent before its children.
      ...(grouping && placed ? placed.groups : []).map(
        (g): RepoFlowNode => ({
          id: g.id,
          type: "repo",
          position: { x: g.x, y: g.y },
          data: { repo: g.repo, movable: Boolean(onMove) },
          style: { width: g.width, height: g.height },
          selectable: false,
          draggable: Boolean(onMove),
          dragHandle: ".repo-box-frame",
          focusable: false,
          zIndex: -1,
        }),
      ),
      ...graph.nodes.map((n): MapFlowNode => {
        const box = grouping && placed ? placed.parts.get(n.id) : undefined;
        return {
        id: n.id,
        type: "map",
        position: box ? { x: box.x, y: box.y } : (held.current.get(n.id) ?? settled.get(n.id) ?? { x: 0, y: 0 }),
        ...(box?.parent ? { parentId: box.parent } : {}),
        // In the plan lens a part is painted by this plan only, not by every open plan.
        data: { node: planView ? { ...n, paint: planView.parts.get(n.id) ?? null } : n, selected: selectedNode === n.id, onPin, dim: litNow !== null && !litNow.nodes.has(n.id), draft: draftMarks?.get(n.id), editing, traffic: partTraffic && graph.traffic ? partTrafficOf(n.id, graph.traffic, partTraffic) : undefined },
        };
      }),
      ...ghostView.ghosts.map(
        (g): GhostFlowNode => ({
          id: ghostId(g.service),
          type: "ghost",
          position: ghostAt.get(ghostId(g.service)) ?? { x: 0, y: 0 },
          data: { ghost: g, windowSecs: graph.traffic?.window_secs ?? 3600 },
          draggable: false,
          selectable: false,
          connectable: false,
        }),
      ),
    ],
    // `heldRevision` stands for `held`, a ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [graph, settled, selectedNode, onPin, heldRevision, litNow, planView, draftMarks, editing, grouping, placed, onMove, ghostView, ghostAt, partTraffic],
  );
  // React Flow draws an edge only between parts it has measured, and it reports a measurement
  // once, through `onNodesChange`. So the nodes are kept in state and every refresh of the
  // overview updates them in place: a part that is rebuilt from scratch loses its measurement
  // and its seams (and their chips) disappear until something resizes it.
  const [nodes, setNodes] = useState<AnyFlowNode[]>([]);
  const groupingRef = useRef(grouping);
  groupingRef.current = grouping;
  useEffect(() => setNodes((current) => refreshNodes(current, wanted)), [wanted]);
  const toSave = useRef<{ positions: Record<string, XY>; grouped: Record<string, XY>; boxes: Record<string, XY> }>({ positions: {}, grouped: {}, boxes: {} });
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const settledRef = useRef(settled);
  settledRef.current = settled;
  const onMoveRef = useRef(onMove);
  onMoveRef.current = onMove;
  const save = useCallback(() => {
    saveTimer.current = null;
    const { positions, grouped: groupedMoves, boxes } = toSave.current;
    toSave.current = { positions: {}, grouped: {}, boxes: {} };
    const move = onMoveRef.current;
    const some = (r: Record<string, XY>) => Object.keys(r).length > 0;
    if (!move || (!some(positions) && !some(groupedMoves) && !some(boxes))) {
      return;
    }
    const body: MapLayout = { ...(some(positions) ? { positions } : {}), ...(some(groupedMoves) ? { grouped: groupedMoves } : {}), ...(some(boxes) ? { boxes } : {}) };
    // Answered (saved and the overview fresh, or refused): let go of the parts, unless they
    // moved again meanwhile. A refused part goes back to where the team has it.
    void move(body).finally(() => {
      const release = (from: Map<string, XY>, id: string, p: XY) => {
        const now = from.get(id);
        if (now && now.x === p.x && now.y === p.y) {
          from.delete(id);
        }
      };
      for (const [id, p] of Object.entries(positions)) {
        release(held.current, id, p);
      }
      for (const id of savedIds({ grouped: groupedMoves, boxes })) {
        const repo = boxRepo(id);
        const p = repo !== undefined ? boxes[repo] : groupedMoves[id];
        if (p) {
          release(heldGrouped.current, id, p);
        }
      }
      setHeldRevision((r) => r + 1);
      setGroupedRevision((r) => r + 1);
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
  // Where each grouped node was when its drag began, for the snap-back.
  const dragFrom = useRef(new Map<string, XY>());
  const sizeOf = useCallback((id: string) => {
    const n = getNode(id);
    return { width: n?.measured?.width ?? NODE_WIDTH, height: n?.measured?.height ?? NODE_HEIGHT };
  }, [getNode]);
  const onNodesChange = useCallback(
    (changes: NodeChange<AnyFlowNode>[]) => {
      if (groupingRef.current) {
        // The By repo grouping (ADR 0068): a part stays inside its box's left and top edge; a
        // node that comes to rest on a box it does not belong to goes back; the boxes refit to
        // their parts; moves are written as the grouping, never as the flat map.
        const now = placedRef.current;
        const shown = shownGroupedRef.current;
        const fixed = changes.map((c) => {
          if (c.type !== "position" || !c.position || isGhostId(c.id)) {
            return c;
          }
          const parent = now?.parts.get(c.id)?.parent;
          let position = parent ? clampInBox(c.position) : c.position;
          if (c.dragging) {
            if (!dragFrom.current.has(c.id)) {
              dragFrom.current.set(c.id, heldGrouped.current.get(c.id) ?? shown.get(c.id) ?? position);
            }
          } else if (now) {
            const from = dragFrom.current.get(c.id) ?? heldGrouped.current.get(c.id) ?? shown.get(c.id) ?? position;
            dragFrom.current.delete(c.id);
            position = settle(c.id, position, from, now, sizeOf);
          }
          return { ...c, position };
        });
        setNodes((current) => refitBoxes(applyNodeChanges(fixed, current)));
        for (const c of fixed) {
          if (c.type === "position" && c.position && c.dragging) {
            heldGrouped.current.set(c.id, c.position);
          }
        }
        const moved = groupedToSave(fixed.filter((c) => !("id" in c && isGhostId(c.id))), shown);
        // Every node that came to rest is held where it is saved, or let go if it rests where the
        // team has it (it did not move, or it snapped back).
        for (const c of fixed) {
          if (c.type !== "position" || c.dragging || isGhostId(c.id)) {
            continue;
          }
          const repo = boxRepo(c.id);
          const p = moved ? (repo !== undefined ? moved.boxes?.[repo] : moved.grouped?.[c.id]) : undefined;
          if (!p) {
            heldGrouped.current.delete(c.id);
          } else if (repo !== undefined) {
            heldGrouped.current.set(c.id, p);
            toSave.current.boxes[repo] = p;
          } else {
            heldGrouped.current.set(c.id, p);
            toSave.current.grouped[c.id] = p;
          }
        }
        if (fixed.some((c) => c.type === "position" && !c.dragging)) {
          setGroupedRevision((r) => r + 1);
        }
        if (moved) {
          if (saveTimer.current) {
            clearTimeout(saveTimer.current);
          }
          saveTimer.current = setTimeout(save, SAVE_AFTER_MS);
        }
        return;
      }
      setNodes((current) => applyNodeChanges(changes, current));
      // A ghost is not a part: nothing about it is held or saved (ADR 0016 stores map ids only).
      const parts = changes.filter((c) => !("id" in c && isGhostId(c.id)));
      for (const c of parts) {
        if (c.type === "position" && c.position && c.dragging) {
          held.current.set(c.id, c.position);
        }
      }
      const moved = positionsToSave(parts, settledRef.current);
      if (moved?.positions) {
        for (const [id, p] of Object.entries(moved.positions)) {
          if (p) {
            held.current.set(id, p);
            toSave.current.positions[id] = p;
          }
        }
        if (saveTimer.current) {
          clearTimeout(saveTimer.current);
        }
        saveTimer.current = setTimeout(save, SAVE_AFTER_MS);
      }
    },
    [save, sizeOf],
  );
  const traffic = graph.traffic ?? { state: "not_configured" as const };
  const windowSecs = traffic.window_secs ?? 3600;
  const showTraffic = chevronsOn(lens, traffic.state);
  // In the grouping, the seams inside one repo fold into bundles (issue #144); an open bundle
  // draws its seams as the flat map does.
  const { bundles, closed, folded } = useMemo(() => {
    const bundles = grouping ? bundlesOf(graph.nodes, graph.edges) : new Map<string, Bundle>();
    const openBy = { lit: litNow?.seams ?? null, hovered: dragBundle ? new Set([...hoveredBundles, dragBundle]) : hoveredBundles, pinned: pinnedBundles };
    const closed = [...bundles.values()].filter((b) => !bundleIsOpen(b, openBy));
    return { bundles, closed, folded: new Set(closed.flatMap((b) => b.seams.map((sm) => sm.id))) };
  }, [grouping, graph, litNow, hoveredBundles, pinnedBundles, dragBundle]);
  // Where along its curve each seam's labels and each bundle's badge sit in the grouping: clear of
  // the part cards and of each other (labels.ts). The flat map keeps every label at its middle.
  const labelTs = useMemo(() => {
    if (!grouping) {
      return new Map<string, number>();
    }
    const byId = new Map(nodes.map((n) => [n.id, n]));
    const cards = new Map<string, Rect>();
    for (const n of nodes) {
      if (n.type !== "map") {
        continue;
      }
      const parent = n.parentId ? byId.get(n.parentId) : undefined;
      cards.set(n.id, { x: n.position.x + (parent?.position.x ?? 0), y: n.position.y + (parent?.position.y ?? 0), width: n.measured?.width ?? NODE_WIDTH, height: n.measured?.height ?? NODE_HEIGHT });
    }
    // Every bundle has one place along its curve, open or closed: an opened bundle's chips sit
    // where its badge was, so opening it moves nothing under the pointer or a dragged issue.
    const inBundle = new Map<string, string>();
    for (const b of bundles.values()) {
      for (const sm of b.seams) {
        inBundle.set(sm.id, b.key);
      }
    }
    const stacks: LabelStack[] = graph.edges
      .filter((e) => !inBundle.has(e.id))
      .map((e) => {
        const arrows = arrowsFor(planView ? { ...e, flows: planView.flows.get(e.id) } : e, lens);
        const chipChars = e.id.length + (e.openPlans.length ? 9 : 0) + (e.issues.length ? 10 : 0) + (e.collision || e.findings.length || e.quietFor !== undefined ? 3 : 0) + (draftMarks?.has(e.id) ? 24 : 0);
        return {
          key: e.id,
          from: e.from,
          to: e.to,
          arrows: arrows.map((a) => ({ offset: a.offset, label: a.kind === "structure" ? null : { width: flowLabelWidth(a), height: 17 } })),
          chip: { width: textWidth(chipChars, 11, 22), height: 26 },
          chipAbove: CHIP_ABOVE_FLOW,
        };
      });
    for (const b of bundles.values()) {
      stacks.push({ key: `bundle-${b.key}`, from: b.from, to: b.to, arrows: [{ offset: 0, label: null }], chip: { width: textWidth(9, 10.5, 20), height: 22 }, chipAbove: 0 });
    }
    const placed = placeLabels(stacks, cards);
    for (const [seam, key] of inBundle) {
      const t = placed.get(`bundle-${key}`);
      if (t !== undefined) {
        placed.set(seam, t);
      }
    }
    return placed;
  }, [grouping, nodes, graph, bundles, lens, planView, draftMarks]);
  const edges: (SeamFlowEdge | BundleFlowEdge | TerritoryFlowEdge)[] = useMemo(() => {
    const chevronsFor = (e: GraphEdge) => {
      if (!showTraffic || !e.traffic) {
        return null;
      }
      const c = { forward: chevronCount(perMinute(e.traffic.forward_calls, windowSecs)), reverse: chevronCount(perMinute(e.traffic.reverse_calls, windowSecs)) };
      return c.forward + c.reverse > 0 ? c : null;
    };
    // A seam with no calls says when it was last seen, or that nothing can observe it (ADR 0083).
    const seamQuiet = (e: GraphEdge) => quietState("seam", e.id, graph.traffic ?? { state: "not_configured" });
    const chipFor = (e: GraphEdge) => (showTraffic ? seamChip(e, graph.traffic ?? { state: "not_configured" }) : undefined);
    const bundleEdges: BundleFlowEdge[] = [];
    for (const b of closed) {
      bundleEdges.push({
        id: `bundle-${b.key}`,
        type: "bundle" as const,
        source: b.from,
        target: b.to,
        className: litNow !== null ? "bundle dim" : "bundle",
        selectable: false,
        focusable: false,
        data: { labelT: labelTs.get(`bundle-${b.key}`), onPin, bundle: b, inDraft: (id: string) => draftMarks?.has(id) ?? false, dim: litNow !== null, onToggle: onBundleToggle, onDragOpen: setDragBundle },
      });
    }
    const bundleOf = (e: GraphEdge) => (bundles.has(bundleKey(e.from, e.to)) ? bundleKey(e.from, e.to) : undefined);
    const drawn: SeamFlowEdge[] =
      graph.edges.filter((e) => !folded.has(e.id)).flatMap((e) =>
        arrowsFor(planView ? { ...e, flows: planView.flows.get(e.id) } : e, lens).map((arrow, i) => {
          // A live pipe's head keeps its size whatever the pipe's width, in the pipe's colour.
          const tier = showTraffic && e.traffic ? liveTier(e.traffic, windowSecs) : 0;
          const tone = tier > 0 && e.traffic ? health(e.traffic.errors, e.traffic.forward_calls + e.traffic.reverse_calls) : null;
          const marker = tone ? { type: MarkerType.ArrowClosed, width: 20 + tier * 4, height: 20 + tier * 4, markerUnits: "userSpaceOnUse", color: `var(--pipe-${tone})` } : { type: MarkerType.ArrowClosed, width: 18, height: 18 };
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
              showTraffic && e.quietFor !== undefined ? "quiet" : "",
              // A seam that carried calls is a live pipe: heavier for more calls, coloured by its error share.
              tone ? `live live-${tier} health-${tone}` : "",
              showTraffic && !e.traffic && e.quietFor === undefined ? quietClass(seamQuiet(e)) : "",
              draftMarks?.has(e.id) ? `draft draft-${draftMarks.get(e.id)?.state}` : "",
              litNow !== null && !litNow.seams.has(e.id) ? "dim" : "",
            ]
              .filter(Boolean)
              .join(" "),
            ...(arrow.reversed ? { markerStart: marker } : { markerEnd: marker }),
            data: { seam: e, arrow, chip: i === 0, selected: selectedSeam === e.id, onSelect: (id: string) => onSelect({ kind: "seam", id }), onPin, draft: draftMarks?.get(e.id), particles: particleMode(arrow, lens, step?.arrow === arrow.id), still: reducedMotion, chevrons: chevronsFor(e), chipTraffic: chipFor(e), bundle: bundleOf(e), labelT: labelTs.get(e.id) },
          };
        }),
      );
    // The particle cap (design §5): past it, flows next to the selection or a lit part keep
    // moving and the rest show the still marker.
    const painted = new Set(graph.nodes.filter((n) => n.paint).map((n) => n.id));
    const near = (d: SeamEdgeData) => (litNow !== null && litNow.seams.has(d.seam.id)) || painted.has(d.seam.from) || painted.has(d.seam.to);
    const territory: TerritoryFlowEdge[] = showTraffic
      ? (graph.territory ?? []).map((u) => ({
          id: `territory-${u.from}-${u.to}`,
          type: "territory" as const,
          source: u.from,
          target: u.to,
          className: "territory",
          selectable: false,
          focusable: false,
          data: { edge: u, windowSecs, reduced: reducedMotion, still: false },
        }))
      : [];
    // Calls to or from a ghost: territory too, drawn the same way (issue #140).
    for (const g of ghostView.edges) {
      territory.push({
        id: `territory-${endId(g.from)}-${endId(g.to)}`,
        type: "territory" as const,
        source: endId(g.from),
        target: endId(g.to),
        className: "territory",
        selectable: false,
        focusable: false,
        data: { edge: { from: endText(g.from), to: endText(g.to), calls: g.calls, errors: g.errors }, windowSecs, reduced: reducedMotion, still: false },
      });
    }
    // Dots, rings and chevrons share one cap; only one lens is on, so they never add up.
    const ambient = [
      ...drawn.flatMap((ed) => (ed.data && ed.data.particles === "ambient" ? [{ id: ed.id, near: near(ed.data) }] : [])),
      ...drawn.flatMap((ed) => (ed.data?.chevrons ? [{ id: ed.id, near: near(ed.data), count: ed.data.chevrons.forward + ed.data.chevrons.reverse }] : [])),
      ...territory.map((t) => ({ id: t.id, near: painted.has(t.source) || painted.has(t.target), count: chevronCount(perMinute(t.data?.edge.calls ?? 0, windowSecs)) })),
    ];
    const moving = particleBudget(ambient);
    return [
      ...bundleEdges,
      ...drawn.map((ed) => (ed.data && (ed.data.particles === "ambient" || ed.data.chevrons) && !moving.has(ed.id) ? { ...ed, data: { ...ed.data, still: true } } : ed)),
      ...territory.map((t) => (t.data && !moving.has(t.id) ? { ...t, data: { ...t.data, still: true } } : t)),
    ];
  }, [graph, lens, selectedSeam, onSelect, onPin, litNow, planView, draftMarks, step, reducedMotion, showTraffic, windowSecs, ghostView, bundles, closed, folded, labelTs, onBundleToggle]);
  // A bundle opened from the keyboard: focus its first chip once it is drawn.
  useEffect(() => {
    const key = focusAfterOpen.current;
    if (!key || !pinnedBundles.has(key)) {
      return;
    }
    focusAfterOpen.current = null;
    const first = graph.edges.find((e) => bundleKey(e.from, e.to) === key);
    setTimeout(() => (first ? document.querySelector<HTMLElement>(`[data-testid="seam-${first.id}"]`)?.focus() : undefined), 0);
  }, [pinnedBundles, graph]);
  // Particles and halos rest while a part is dragged or the page is hidden.
  const [dragging, setDragging] = useState(false);
  const [hidden, setHidden] = useState(typeof document !== "undefined" && document.visibilityState === "hidden");
  useEffect(() => {
    const on = () => setHidden(document.visibilityState === "hidden");
    document.addEventListener("visibilitychange", on);
    return () => document.removeEventListener("visibilitychange", on);
  }, []);

  // The lens switch, the scenario player, the legend and the map actions sit in a band above the
  // canvas, never over it: no part is hidden under them.
  const band = (
    <div className="map-band" data-testid="map-band">
      <div className="map-band-row">
        <div className="lens" role="radiogroup" aria-label="What the arrows show">
          {LENSES.map((l) => (
            <button
              key={l.id}
              type="button"
              role="radio"
              aria-checked={lens === l.id}
              className={[lens === l.id ? "active" : "", l.id === "traffic" ? `traffic-lens ${trafficFresh ? "fresh" : "unknown"}` : ""].filter(Boolean).join(" ")}
              onClick={() => setLens(l.id)}
              disabled={planView !== null}
              title={planView ? "Close the issue to choose what the arrows show" : l.id === "traffic" ? "What the running system actually calls, observed" : undefined}
            >
              {l.id === "traffic" && <i className="live-dot" aria-hidden="true" />}
              {l.label}
              {(l.id === "data" || l.id === "control") && ` (${counts[l.id]})`}
              {l.id === "traffic" && vitalsAt && vitalsAt.calls > 0 && <span className="lens-rate" aria-hidden="true"> · {compactRate(vitalsAt.perMin)}/min</span>}
            </button>
          ))}
          {graph.scenarios.length > 0 && (
            <button type="button" role="radio" aria-checked={lens === "scenario"} className={lens === "scenario" ? "active" : ""} onClick={() => setLens("scenario")} disabled={planView !== null} title={planView ? "Close the issue to choose what the arrows show" : "Play a journey through the system, step by step"}>
              Scenarios ({graph.scenarios.length})
            </button>
          )}
        </div>
        <div className="map-actions">
          <button type="button" className={grouping ? "map-action active" : "map-action"} aria-pressed={byRepo} onClick={() => setByRepo((on) => !on)} disabled={editing} title={editing ? "Leave edit mode to group the map by repo" : "Box each repo's parts together: what each repo provides to the whole"}>
            By repo
          </button>
        </div>
        {(onAutoArrange || onToggleEdit) && (
          <div className="map-actions">
            {onToggleEdit && (
              <button type="button" className={editing ? "map-action active" : "map-action"} aria-pressed={editing} onClick={onToggleEdit} title="Change the map as one shared draft: add, change and remove parts and seams, then propose the draft as a pull request">
                Edit map
              </button>
            )}
            {onAutoArrange && (
              <button
                type="button"
                className="map-action"
                onClick={() => onAutoArrange(grouping ? "grouped" : "flat")}
                disabled={grouping ? Object.keys(stored?.grouped ?? {}).length + Object.keys(stored?.boxes ?? {}).length === 0 : Object.keys(stored?.positions ?? {}).length === 0}
                title={grouping ? "Remove every saved position of the By repo arrangement, for the whole team, and lay it out automatically. The flat map stays as it is." : "Remove every saved position, for the whole team, and lay the map out automatically"}
              >
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
      {editing && (
        <p className="edit-hint" data-testid="edit-hint">
          <strong>Editing the shared draft.</strong> Double-click the canvas to add a part, drag from the dot on a part&apos;s right edge to another part to draw a seam, or select a part or seam to change it in the panel. Nothing reaches the map until the draft is proposed and merged.
        </p>
      )}
      {lens === "traffic" && trafficFresh && graph.traffic && <TrafficVitals traffic={graph.traffic} quietSeams={vitalsAt?.quietSeams ?? 0} seams={graph.edges.length} />}
      {lens !== "scenario" && (
        <div className="legend" data-testid="legend">
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
          {lens === "traffic" && <TrafficLegendText traffic={traffic} onConfigure={onConfigure} />}
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
        </div>
      )}
      {lens === "traffic" && trafficActions}
    </div>
  );

  return (
    <div className="map-frame">
      {band}
      <div className={dragging || hidden ? "map-canvas paused" : "map-canvas"} ref={canvasRef}>
        {!positions ? (
          <div className="empty">Laying out the map…</div>
        ) : (
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            onNodesChange={onNodesChange}
            onNodeDragStart={() => setDragging(true)}
            onNodeDragStop={() => setDragging(false)}
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
            onNodeClick={(_e, n) => {
              if (n.type === "map") {
                onSelect({ kind: "node", id: n.id });
              }
            }}
            onEdgeClick={(_e, edge) => {
              // A territory edge is not a seam: there is nothing to select.
              if (edge.type !== "territory") {
                onSelect({ kind: "seam", id: (edge as SeamFlowEdge).data?.seam.id ?? edge.id });
              }
            }}
            onPaneClick={() => {
              onSelect(null);
              setPinnedBundles(new Set());
            }}
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

/** The Traffic lens's legend: why nothing is shown, or what chevrons, hollow seams and dotted lines mean (design §5.5, §5.6). */
function TrafficLegendText({ traffic, onConfigure }: { traffic: NonNullable<Graph["traffic"]>; onConfigure?: (() => void) | undefined }) {
  const unknown = unknownSentence(traffic);
  if (unknown) {
    return (
      <>
        <span>{unknown}</span>
        {traffic.state === "not_configured" && onConfigure && (
          <>
            {" "}
            <button type="button" className="legend-action" onClick={onConfigure}>
              Configure workspace…
            </button>
          </>
        )}
      </>
    );
  }
  const l = trafficLegend(traffic);
  return (
    <>
      <i className="key-mark chevron" aria-hidden="true" /> {l.text}
      {l.unmappedCount > 0 && (
        <details className="unmapped">
          <summary>
            {l.unmappedCount} service{l.unmappedCount === 1 ? "" : "s"} with no namespace and no part to match
          </summary>
          {l.unmapped.join(", ")}
        </details>
      )}
    </>
  );
}

export function MapView(props: Props) {
  return (
    <ReactFlowProvider>
      <Inner {...props} />
    </ReactFlowProvider>
  );
}
