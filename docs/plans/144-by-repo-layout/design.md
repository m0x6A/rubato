# Design: By repo, readable and arrangeable (#144)

Plan: [../144.md](../144.md) · ADR 0068 (*proposed*, adds to ADR 0016).

## UX review of the grouping as it is (main 8e9bcf0)

Looked at with the e2e seed (`docs/design/rubato-map.yml`, one repo `m0x6A/rubato` with eight
parts, three externals), at 1440×900 and 390×844, in the Structure and Control flow lenses.

**What someone opening By repo needs to see.** By repo answers "what does each repo provide to
the whole, and what crosses from one repo to another?". So, at a glance: each repo's box with its
name, every part in it by name and kind, and the seams that leave the box (here
`companion-tmux-ide`, `companion-claude-code`, `server-github`, `skill-github`) with their chips,
because those are the contracts between repos and teams. Paint, plan counts and findings on parts
keep their meaning.

**What can fold away.** The seams inside one repo. On this map that is seven of eleven seams
(`companion-server`, `web-server`, `skill-plan-file`, `server-plan-file`, `plan-check-plan-file`,
`skill-system-map`, `server-system-map`). Their existence still matters (a line between two
parts), but their chips and, in a flow lens, their flow labels are detail to open on demand. A
warning on one of them (collision, finding) must not fold away with it.

**What is wrong, seen.**

1. *Desktop, Structure lens.* ELK puts the box's parts in three layers 200 px apart. The chips
   of `skill-system-map`, `skill-plan-file`, `server-system-map`, `plan-check-plan-file` and
   `server-plan-file` stack in the one column between the second and third layer, touching each
   other; `server-plan-file` and `skill-plan-file` sit on the `system-map` and `plan-file`
   cards. The `web-server` chip sits under `rubato-server`'s card and the `companion-server` chip
   (with its collision warning) covers the top of `rubato-companion`.
2. *Desktop, Control flow lens.* Worse: every flow adds a label, so `web` and `rubato-companion`
   are covered by four labels each, truncated at 28 characters ("LaunchRequest: start a sess…",
   "Welcome with limits, server…"). The labels' `title` is the only way to read them, and only
   with a mouse and a wait.
3. *Nothing moves.* Drag is off in the grouping, and the box cannot be moved. ELK's result is the
   only arrangement.
4. *Phone (390 px).* The canvas is a 230 px band above the side panel. The view fitted at desktop
   size is off screen after a resize until Fit is pressed; fitted, the whole box is at about 0.3
   zoom, where nothing is readable, and the zoom controls sit over the box's left column. Reading
   it means zooming into one part at a time, as on the flat map. With the inside seams folded
   there is much less to read at that zoom.

**What the change keeps.** The box label, parts drawn by `PartCard` with every badge, cross-repo
seams drawn by `SeamEdge` with every lens, edit mode turning the grouping off, ghosts only on the
flat map.

## D1. The grouping is a second shared arrangement (ADR 0068, Decision 1)

`MapLayout` (rubato-protocol `layout.rs`) gains two maps, both `#[serde(default)]`:

```rust
pub struct MapLayout {
    pub positions: BTreeMap<String, Position>, // flat map, ADR 0016, unchanged
    pub grouped: BTreeMap<String, Position>,   // part id → relative to its repo's box (canvas if no repo)
    pub boxes: BTreeMap<String, Position>,     // "owner/name" → box top-left on the canvas
}
```

A `PUT /api/layout` carries the moved entries of any of the three; it is refused whole when an
id is not a part of the merged map or the draft, when a box key is not a repo some such part
names, or when a position is off the canvas. `DELETE /api/layout` clears `positions` only (as
today); `DELETE /api/layout/grouped` clears `grouped` and `boxes`. Both answer with what they
removed, so a `PUT` of the answer is Undo; the answer holds only rows for the map's current
parts and repos (a stale row would make the Undo `PUT` refused whole). A write counts once in
`rubato.layout.writes` per arrangement it touches, so a body with both `positions` and
`grouped` counts `flat` and `grouped`; the `layout written` line is at info.

Storage: one additive table, migration `0024_map_layout_grouped.sql`:

```sql
CREATE TABLE map_layout_grouped (
    workspace_id TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('part', 'box')),
    id           TEXT NOT NULL,
    x REAL NOT NULL, y REAL NOT NULL,
    updated_by   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (workspace_id, kind, id)
);
```

A separate table rather than columns on `map_layout`: a row there means "this part has a flat
position", and a part placed only in the grouping would need NULLs there. Rows for ids the map
no longer has are not served and are deleted on the next write, as in `map_layout`.

Why relative positions: React Flow draws a child at a position relative to its parent
(`parentId`), and moving a box must move its parts without rewriting them.

Option (b) of the issue, derived from the flat layout, was weighed: it needs no state, but a drag
inside a box would then have nowhere to go except the flat positions, which the issue forbids.

## D2. Placement and moves in the browser (Decision 2)

A new pure module `web/src/features/map/grouped.ts`:

- `placeGrouped(elk, layout, held)`: each box at `held ?? stored boxes ?? ELK`; each part at
  `held ?? stored grouped ?? ELK` (relative), clamped to the box's padding (left 24, top 48);
  each box sized to fit its parts: right and bottom edges at the furthest part plus padding,
  never narrower than its name. ELK's own box size is not used, so a box shrinks when its parts
  move together.
- `clampInBox(p)`: a part's relative position is never left of or above the padding. Applied to
  every position change of a boxed part while it is dragged, so the part stops at the edge.
- `snapBack(...)`: at the end of a drag, a part whose card overlaps another repo's box, or a box
  that overlaps another box, goes back to where the drag began.
- `groupedToSave(changes, shown)`: like `positionsToSave` (ADR 0016) but writes `grouped` for
  parts and `boxes` for boxes, never `positions`.

`MapView` in the grouping: parts and boxes are draggable when `onMove` is given. React Flow 12
wraps a draggable node in an element that takes every pointer event, so a box gets
`dragHandle: ".repo-box-frame"` (its name bar and border), and `.react-flow__node-repo` has
`pointer-events: none` with `auto` on the frame: clicks and pans in the box's body reach the
pane. `mergeNodes` today keeps `style` and `draggable` from the node on screen, so a box would
keep its first size; it now takes `style`, `width`, `height`, `draggable` and `dragHandle`
from what is wanted (the measurement it keeps is `measured`). `onNodesChange` applies the
clamp, then refits the boxes from the current children so a box grows while a part is dragged,
then holds and saves through `groupedToSave`. The flat path (`positionsToSave`, `held` for the
flat map) is not entered while the grouping is on. Held grouped positions are a separate map, so
turning the grouping off draws every part from the flat layout as it was.

Auto-arrange calls `DELETE /api/layout/grouped` while the grouping is on (button title says
"By repo arrangement"), the flat one otherwise; App's notice and Undo work for both.

## D3. Bundles for seams inside one repo (Decision 3)

A new pure module `web/src/features/map/bundles.ts`:

- `bundlesOf(nodes, edges)`: seams whose two parts name the same repo, grouped by unordered pair
  of parts → `Bundle { key, a, b, seams }`. Seams with an end that has no repo, or ends in two
  repos, are never bundled.
- `isOpen(bundle, { lit, hovered, pinned })`: open when any of its seams is lit (`litNow.seams`:
  selection, plan lens, scenario step), when hovered or focused, or when pinned open by a click.
- `badgeState(bundle)`: the strongest of its seams' states, collision/finding > quiet > busy, plus
  a draft mark if any seam is in the draft; the badge shows ⚠ for collision or finding.

In the grouping, a closed bundle is one React Flow edge of a new type `bundle` between `a` and
`b`: a plain line and, at its middle, a badge button "N seam(s)" (`data-testid`
`bundle-<a>-<b>`), with the state class and a title listing the seams. Hover (pointer enter on
the badge, its line, or any open chip or label of the bundle; closes 400 ms after the last leave),
focus, a click (toggles pinned), and `dragenter` of an issue row open it; an open bundle draws
its seams exactly as the flat map does, so lenses, particles, reduced motion and the chip's drop
target are unchanged. A closed bundle draws no particles and no chevrons. Pointer events do not
fire during a native HTML5 drag, so a bundle opened by `dragenter` of an issue row stays open
until `drop` or `dragend` on the document, not on the 400 ms hover timer. Outside the grouping nothing is bundled.

ELK in the grouping: `elk.spacing.nodeNode` 40 → 60, so badges and part cards keep apart.

## D4. Flow labels read in full

The label's DOM holds the whole `carries` text; CSS clips it to the width of a layer gap with an
ellipsis, and `:hover` / `:focus-within` lift the clip, wrap the text to at most 36ch and raise
the label above its neighbours. The `title` stays. The inspector already lists every flow of a
selected seam in full (`SeamFlows`).

## What we watch

`rubato.layout.writes` with `rubato.layout.arrangement` (`flat`, `grouped`), `rubato.web.rejected`
for `/api/layout`, and the `layout written` / `layout cleared` lines (plan O-row). A `flat` count
that moves while people only use the grouping would mean a grouped move reached the flat layout.
