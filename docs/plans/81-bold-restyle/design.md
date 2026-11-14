# Design: bold restyle of the web app (#81)

Plan: [../81.md](../81.md) · Map: node `web` only · Track: **full** because the self-hosted type
pairing adds a dependency (two npm font packages).

The app reads like a settings page: system font, grey hairlines, outline badges, words where
state should be. #81 asks for a restyle that makes the map and the work on it feel alive. This
note sets the direction (moodboards, then one choice), the type, the full token table, the motion
system, and what leads on each surface. Everything here uses data the web already receives in the
`Overview` (`web/src/protocol.ts:1074`); anything that needs more is a follow-up (§9).

## Where it sits on the map

`web` only. No change to `web-server`: the same `GET /api/overview`, `/ws/web` and request
types. No protocol type, no server code and no schema changes. `rubato-server` serves `web/dist`
from disk as today (`ServeDir`, `app.rs:84`); the fonts ship inside `dist` as hashed `.woff2` assets.

## 1. Moodboards

**A. Control room.** The map is a near-black canvas (`#05070d`) with a faint dot field every
24 px, like a plotter bed. Parts are slabs of lit hardware: a top-to-bottom gradient face
(`#18223a` → `#0f1626`), a 1 px rim that catches light along the top edge, the id in mono caps
offset like a part number, a thin activity strip along the bottom edge where issue states sit as
coloured segments. **At rest** a part is dark and quiet; only its rim and the strip show.
**At work** (a session `working` on one of its issues) a soft cyan halo outside the card breathes
on a 2.8 s cycle, the session pips in the strip glow green, and the conduits leaving it carry
particles. Seams are conduits: a dark 6 px channel with a 1.5 px bright core. The **issue panel**
is a layered instrument stack: a stage rail across the top with the current stage lit, a spend
gauge on the right, then sessions as lanes, then text.

**B. Blueprint.** Deep blue-black ground, cyan linework, parts as outlined schematic blocks with
corner ticks, flows drawn as dimensioned lines with arrow callouts, everything 1 px and precise.
A part at work fills with a cyan hatch that slides slowly. The issue panel is a spec sheet:
tables framed by rules, the stage rail as a ruler. Crisp, but the hatch and outlines leave little
room for "lit" versus "not lit", and hatching is busy at 900 px.

**C. Synthwave terminal.** Black, magenta and cyan neon, glow on everything, a CRT scanline
over the canvas, type in mono throughout. Loud and fun for a screenshot; tiring for a tool open
all day, the glow carries no meaning when everything glows, and scanlines fight small text.

**Chosen: A, control room.** It is the only one where light *means* something: dark is quiet,
lit is affected or new, breathing is working, particles are declared flows, marching dashes are
proposed. B is a good light theme (§3 borrows its linework for light mode); C spends contrast and
meaning on mood. A also keeps every text block on a solid surface, which the contrast test can
check (§4), while the gradients and glows live on rims, faces under no small text and halos
outside the card.

## 2. Type

**Space Grotesk (UI) + JetBrains Mono (ids, node names, numbers, badges).**

- Space Grotesk is a grotesk with geometric bones and a few engineered quirks (the `a`, `G`,
  `t`), which reads as "technical" at headings and 13–15 px UI text without the sameness of
  Inter. It has a variable `wght` axis 300–700, so one file covers every weight.
- JetBrains Mono is built for code at small sizes: tall x-height, `0` with a dot, distinct
  `l 1 I`, which matters for ids like `rubato-companion` and `companion-server`, sha prefixes and
  `#81`. Tabular by nature, so counts and dollars line up. Variable `wght` 100–800.
- Badges, kind labels and every number use the mono in small caps-like uppercase with
  `letter-spacing: 0.06em`; the instrument feel comes from that, not from colour.
- Runner-up: Geist + Geist Mono (one family, very coherent) — rejected because it is the
  default of a lot of current tooling and would read as a template. Inter Tight + IBM Plex Mono —
  rejected as too neutral for "be bold".

**Self-hosted.** `@fontsource-variable/space-grotesk` and `@fontsource-variable/jetbrains-mono`
(both 5.3.0, OFL-1.1) as `web` dependencies, imported once in `main.tsx`
(`import "@fontsource-variable/space-grotesk/wght.css"`, same for the mono). Vite copies the
`.woff2` files into `dist/assets` with hashed names; the server serves them from disk with the rest of `dist`. No request leaves
the origin at runtime: an e2e check fails on any request to a host other than the app's (§7).
Only the `latin` and `latin-ext` subsets load (their `unicode-range` makes the browser fetch only
what a page uses). `font-display: swap` with metric-matched fallbacks (`--font-ui-fallback:
"Segoe UI", system-ui, sans-serif`) so a slow load never blanks text.

Tokens: `--font-ui: "Space Grotesk Variable", system-ui, sans-serif;`
`--font-mono: "JetBrains Mono Variable", ui-monospace, Menlo, monospace;`. The five places that
spell `ui-monospace, SFMono-Regular, Menlo, monospace` today (`web/src/styles.css`) use
`var(--font-mono)`.

Scale (px, line-height): 11/1.3 mono badges and kinds, 12/1.4 mono ids and table cells,
13/1.45 UI body, 15/1.3 panel headings (600), 20/1.2 numerals in gauges (mono 500), 28/1 big
counts on the Plans and Repos headers (mono 600).

## 3. Palette and tokens

Dark first: `:root` holds the dark values; light is the override. Accent roles:

- **cyan** — selection, focus, affected by a plan, the scenario's current step, live data.
- **violet** — new and proposed: a part a plan adds, a flow a plan proposes, the shared draft.
- **magenta** — heat: open plans per seam, and the seam collision (two plans on one seam).
- **signal green** — working sessions, done, live connection, fits.
- **amber** — in review, stale approvals, findings and warnings.
- **red** — blocked, errors, does not fit.

Each role has a base (borders, fills, glows, large text; at least 3:1 on `--panel` for
non-text contrast), an `-ink` (small text, 4.5:1 on every surface it sits on) and a `-wash`
(a solid tint under a filled badge, so badge text sits on a token, not on a `color-mix`).

Light mode is not an inversion: cool paper ground, white panels, B's linework for the map
(a 1 px `--grid` dot field on `--canvas`, parts with a white-to-mist face and a crisp `--line`
rim), glows become tinted drop shadows, and every accent is deepened one step so it holds 3:1 on
white.

| Token | Dark | Light | Role | Small text? |
| --- | --- | --- | --- | --- |
| `--bg` | `#070a12` | `#f3f5fa` | page ground, topbar strip | surface |
| `--canvas` | `#05070d` | `#eceff6` | the map canvas under the dot field | surface |
| `--panel` | `#0c111d` | `#ffffff` | side panel, bands, cards | surface |
| `--raised` | `#121a2b` | `#f7f8fc` | rows, chips, inputs, menus on a panel | surface |
| `--node-top` | `#18223a` | `#ffffff` | part face, top of its gradient | surface (gradient end) |
| `--node-bottom` | `#0f1626` | `#f1f4fa` | part face, bottom of its gradient | surface (gradient end) |
| `--line` | `#222d45` | `#d6dce8` | hairlines, idle rims, idle conduits | no |
| `--line-strong` | `#3a4a6b` | `#a9b3c7` | hovered rims, conduit channel | no |
| `--grid` | `#1c2742` | `#cdd4e3` | the canvas dot field | no |
| `--ink` | `#e8edf7` | `#0d1324` | body text | **yes** |
| `--muted` | `#8d9ab5` | `#525d75` | secondary text (`.muted`, 12 px) | **yes** (now checked) |
| `--chip-ink` | `#aab5cc` | `#3d475d` | seam chips, flow labels, legends | **yes** |
| `--accent` | `#22d3ee` | `#0891b2` | cyan role: selection, affected, focus ring, step | no |
| `--accent-strong` | `#22d3ee` | `#0e7490` | filled primary buttons, active tab | no (fill) |
| `--on-accent` | `#031418` | `#ffffff` | text on `--accent-strong` | **yes** (on `--accent-strong`) |
| `--accent-ink` | `#67e8f9` | `#0e6a80` | cyan small text | **yes** |
| `--accent-wash` | `#0a2a33` | `#e0f6fb` | under cyan badges | surface |
| `--new` | `#a78bfa` | `#7c3aed` | violet role: new parts, proposed flows, draft | no |
| `--new-ink` | `#c4b5fd` | `#6227c9` | violet small text | **yes** |
| `--new-wash` | `#1e1638` | `#efe9fe` | under violet badges | surface |
| `--heat` | `#f472b6` | `#db2777` | magenta role: plans per seam, collision | no |
| `--heat-ink` | `#f9a8d4` | `#a8175a` | magenta small text | **yes** |
| `--heat-wash` | `#33122a` | `#fde8f1` | under magenta badges | surface |
| `--ok` | `#34d399` | `#059669` | green role: working, done, live, fits | no |
| `--ok-ink` | `#6ee7b7` | `#04704f` | green small text | **yes** |
| `--ok-wash` | `#0a2a20` | `#e2f7ee` | under green badges | surface |
| `--warn` | `#fbbf24` | `#d97706` | amber role: in review, stale, findings | no |
| `--warn-ink` | `#fcd34d` | `#8f4a05` | amber small text | **yes** |
| `--warn-wash` | `#2e2208` | `#fdf1dc` | under amber badges | surface |
| `--bad` | `#f87171` | `#dc2626` | red role: blocked, errors, does not fit | no |
| `--bad-ink` | `#fca5a5` | `#a91d1d` | red small text | **yes** |
| `--bad-wash` | `#331314` | `#fde7e7` | under red badges | surface |
| `--quiet` | `#5b6780` | `#8a93a6` | unknown, ended, external parts | no |
| `--affected` | `var(--accent)` | `var(--accent)` | alias kept for existing rules | no |

Non-colour tokens (not checked by the contrast test, never under text):
`--glow-accent`, `--glow-new`, `--glow-ok`, `--glow-heat` (pre-composed `box-shadow` stacks,
e.g. `0 0 0 1px var(--accent), 0 0 18px 2px rgb(34 211 238 / 0.35)`), `--rim`
(`inset 0 1px 0 rgb(255 255 255 / 0.06)`), `--radius-card: 12px`, `--radius-chip: 999px`,
`--font-ui`, `--font-mono`, and the motion tokens in §5.

Checked numbers (computed with the test's own formula before writing this table): 55 pairs per
scheme, every one at least 4.5:1 in both; the lowest is 5.36:1 (`--on-accent` on
`--accent-strong`, light). Every role base is at least 3:1 on `--panel` in both schemes.

**Text never sits on a gradient without a solid token underneath.** The only gradient under
text is the part face, between two solid tokens `--node-top` and `--node-bottom`. The test
checks small text against both ends, and checks that every channel of `--node-top` is at least
the one of `--node-bottom` (monotone), which makes luminance monotone along the gradient, so
both ends passing means every point passes. Glows are `box-shadow` outside the card; badges
sit on their `-wash`; the canvas dot field is under no text (seam chips and flow labels carry
their own `--raised` background).

## 4. The contrast test, restructured

`web/src/styles.test.ts` stays a parser and keeps its shape; it follows the new structure:

- `schemes()` reads `:root { … }` as **dark** and the block inside
  `@media (prefers-color-scheme: light) { :root:not([data-theme="dark"]) { … } }` as **light**
  (light inherits the rest from dark). A second rule `:root[data-theme="light"]` repeats the
  light block for a forced theme; the test asserts the two light blocks are identical, so they
  cannot drift. `:root[data-theme="dark"]` needs nothing (it is the default).
- If either block is missing, `schemes()` throws instead of returning `{}` (today a missing dark
  block silently checks light twice).
- `PAIRS` grows from 6 to 55 per scheme: `--ink`, `--muted`, `--chip-ink` on every surface
  (`bg`, `canvas`, `panel`, `raised`, `node-top`, `node-bottom`); each `-ink` on `panel`, `bg`,
  `raised`, `node-top`, `node-bottom` and its own `-wash`; `--on-accent` on `--accent-strong`.
- New `GRADIENTS`: `[["node-top","node-bottom","part face"]]`, checked for channel monotony.
- New `NON_TEXT`: each role base on `--panel` at 3:1 (WCAG 1.4.11).
- `uses` grows: `.badge.<state>` rules carry `color: var(--<role>-ink)` and
  `background: var(--<role>-wash)`; `.muted` carries `--muted`; `.flow-label` and `.seam-chip`
  carry `--chip-ink` on `--raised`; `button.primary` carries `--on-accent`.
- Rules stay single lines starting with `selector {`, as `rule()` expects.
- **Must fail, kept and extended**: light `--accent` on `--bg` (3.38:1), light `--warn` on
  `--panel` (3.19:1) and dark `--quiet` on `--panel` (3.32:1) are each below 4.5:1, which proves
  the base tokens are not text tokens; a fixture CSS string with the light block removed makes
  `schemes()` throw; a fixture gradient whose top is darker in one channel fails `GRADIENTS`.
- The light block defines every colour token the dark block defines (no pair is checked with a
  dark value by inheritance).
- `rule()` finds each selector exactly once and matches `color:` only where it is not
  `background-color:`.
- A scan fails any rule whose `color:` is a role base (`--accent`, `--new`, `--heat`, `--ok`,
  `--warn`, `--bad`, `--quiet`) or whose text sits on a `color-mix(…)` background; such rules use
  an `-ink` on a `-wash` or `--raised` instead. It prints the rules it scanned.
- A fixture with one `-ink` lowered below 4.5:1 must be reported by the pair loop (the checker,
  not only the tokens, fails on purpose).
- It still prints how many pairs, rules and targets it checked.

## 5. Motion system

**One switch.** A `web/src/shared/motion.ts` module owns `prefers-reduced-motion`: it sets
`document.documentElement.dataset.motion` to `full` or `reduced` from `matchMedia` (and on
change), and exports `useMotion()` for the few JS decisions (fitView duration, whether to render
particles at all). The `usePrefersReducedMotion` hook in `MapView.tsx:127` moves there. **Every
CSS animation and transition is written under `:root[data-motion="full"]`**; nothing animates by
default. If the script never runs, nothing moves. One `@media (prefers-reduced-motion: reduce)`
block remains as a belt (the e2e proves the switch without leaning on it, §7a): `*, *::before, *::after { animation: none !important; transition: none
!important; }`.

**No SMIL, no rAF loops.** The scenario pulse today is an SVG `<animateMotion>`
(`MapView.tsx:68`), which `document.getAnimations()` does not see. It is replaced by CSS
`offset-path: path(...)` + `offset-distance` keyframes on the same circle, so every moving thing
is a CSS animation the e2e can count and the switch can stop. Nothing in the app calls
`requestAnimationFrame` for movement.

Motion tokens: `--dur-fast: 120ms`, `--dur-base: 200ms`, `--dur-slow: 320ms`,
`--dur-fill: 480ms`, `--ease-out: cubic-bezier(.2,.8,.2,1)`, `--ease-in-out:
cubic-bezier(.65,0,.35,1)`.

| # | Animation | Means | Duration, easing | Implemented as | Under reduced motion |
| --- | --- | --- | --- | --- | --- |
| M1 | **Flow particles** on declared flows, in the data lens (filled dots) and the control lens (rings) | a flow the map declares, in its direction; data is a **filled dot**, control a **ring** | 64 px/s (duration from path length, 2–6 s), linear, infinite; 2 particles per flow, staggered half a cycle | elements with `offset-path` = the edge path, `offset-distance` 0→100% | not rendered; one static marker of the same shape at 30 % of the path, clear of the label |
| M2 | **Proposed flow** (`.react-flow__edge.moving`, every flow of the plan lens) | a flow a plan *proposes*: not there yet | 1.2 s linear infinite (as today) | violet dashed conduit `6 6`, `stroke-dashoffset` −24; **no particles ever** | static dashes; the label gains a "proposed" tag (also shown with motion) |
| M3 | **Scenario step pulse** with a comet trail | the step being played, in its direction, kind by shape | 1200 ms `--ease-in-out`, infinite while the step shows; edge brightens over `--dur-base` | head circle (dot or ring) + 3 followers at 60/120/180 ms delay, opacity .5/.3/.15, same shape; the followers carry `pulse-trail`, not `pulse` or a `pulse-<id>` test id; other particles hidden in the scenario lens | head sits still at 30 % of the path, no trail |
| M4 | **Working breathe** | a session is `working` on an issue on this part (status from the overview, never for `unknown`) | 2.8 s `--ease-in-out` infinite, opacity .35↔.85 | a pre-blurred `box-shadow` on a `::after` layer; only its `opacity` animates | halo at fixed .6 opacity |
| M5 | **Light up** | a part or seam becomes affected, new or selected | `--dur-slow` `--ease-out` | `transition` on `box-shadow` of a `::before` layer's opacity and the rim colour | instant |
| M6 | **Badge change** | a state changed (planned → started → …) | colour transition `--dur-base`; one 360 ms flash (wash brightens then settles) | keyed by state so the element re-mounts with a `badge-in` keyframe | instant swap |
| M7 | **Stage rail / progress ring fill** | how far the issue is along the progress model | `--dur-fill` `--ease-out` | `stroke-dashoffset` transition on the ring, `transform: scaleX` on the rail fill | instant |
| M8 | **Panel and inspector open** | something was selected | `--dur-base` `--ease-out` | opacity 0→1, `translateY(6px)`→0 | instant |
| M9 | **Live ping** | the web socket is live | 2 s `--ease-out` infinite; a ring scales 1→2.4 and fades | `transform` + `opacity` on a `::after` | static green dot; disconnected is red with no ping either way |
| M10 | **Spend gauge sweep** | spend against the plan's cap | `--dur-fill` `--ease-out` on change | `stroke-dashoffset` on an arc | instant |
| M11 | **Tab indicator slide** | which tab is open | `--dur-base` `--ease-out` | `transform: translateX` + `scaleX` of one bar | instant |
| M12 | **Collision throb** | two open plans on one seam (a "never silent" finding) | 1.6 s `--ease-in-out` infinite | magenta `::after` halo on the seam chip, `opacity` only | solid magenta halo plus the "2 plans" count |

Twelve animations. Under reduced motion every row loses its movement and keeps its meaning: shape
(dot vs ring), dash (proposed), halo (working, collision) and text all remain.

**The `moving`-means-proposed encoding.** Two vocabularies, never mixed on one edge: **particles
travel** on declared flows (M1, M3), and **dashes march** only on proposed ones (M2). The lenses
keep what `flows.ts` does today (`flows.ts:58`, `:69`, `:71`): the data lens shows declared data
flows on solid conduits with dots; the control lens shows declared control flows on dashed `7 5`
conduits whose dashes **stand still**, with rings; the plan lens shows only the plan's flows, all
`moving`, violet, marching, and no particles; the scenario lens shows every flow with the step's
comet. So the line between declared and proposed is *moving particles versus moving dashes*, plus
violet and the "proposed" tag; a declared dashed control flow never marches. The legend in the map
band draws all three samples, and each flow label in the plan lens says "proposed". The shared
draft's seams stay dashed `3 4` in violet and do not move (they are a map change, not a flow).

**Honesty about particles.** Particle speed and density are fixed; they say "declared, this
way", not "traffic is flowing". Rubato does not observe traffic in v0 (map versus territory);
§9 lists that as a follow-up.

**Performance.** Only `transform`, `opacity`, `offset-distance` and `stroke-dashoffset`
animate. No animated `filter`; glows are static `box-shadow`s whose layer's `opacity` animates.
Particles are capped at 2 per flow and 120 in total; past the cap, particles run only on
flows touching the selection or a lit part (the rest show the static marker). Particles pause
(`animation-play-state: paused`) while a part is being dragged and while the page is hidden.
Checked by an e2e on a synthetic map of 40 parts and 60 seams (one flow each, data lens, 10
working parts, served by stubbing `/api/overview` with a typed `Overview` fixture,
`web/e2e/fixtures/big-map.ts`): it first asserts `long-animation-frame` is in
`PerformanceObserver.supportedEntryTypes` (else it fails, not passes), then over 5 s of steady
state records no frame over 50 ms; it prints the parts, seams, particles and the median frame
time but does not fail on the median (CI machines vary). Must fail: the same page with an
injected 80 ms busy rAF callback reports long frames.

## 7a. Proving reduced motion

- **Static check of the CSS** (Vitest, `web/src/motion.test.ts`): every rule in `styles.css` that
  declares `animation`, `animation-name` or `transition` (other than `none`) has a selector
  starting `:root[data-motion="full"]`. It prints how many rules it checked; a fixture rule
  outside the switch fails it on purpose.
- **E2e, the switch alone**: `reducedMotion: "no-preference"` with `data-motion="reduced"`
  forced by an init script, so the media-query belt cannot be what stops things. Then the same with
  `reducedMotion: "reduce"` and no forcing, to prove the switch follows the media query.
- **E2e, what it does**: in one `evaluate`, select a part, open an issue and start a scenario
  step, then read `document.getAnimations()` (CSS animations and running transitions) at once:
  empty. No `animateMotion`/`animate` element anywhere. Play is paused, and two screenshots of
  the map canvas 500 ms apart (the "refreshed" text masked) are identical, which catches movement
  driven from JS too.
- **Must fail**: under `no-preference` without forcing, the pulse's own `getAnimations()` is
  non-empty and the two screenshots differ.

## 6. Surfaces: the visual leads, the text supports

| Surface | Visual that leads | Text that supports it | What animates |
| --- | --- | --- | --- |
| **Topbar** | live LED, a team-wide segmented track (issues by state, one segment per state, width by count), total spend as a mono numeral | "live · refreshed 4 s ago", map name and version in mono, login | M9, M7 on the track |
| **Map canvas** | dark dot field; parts as lit slabs; conduits; particles; seam chips with a plan count | node id, kind, flow labels, legend | M1, M2, M3, M4, M5, M12 |
| **Part at rest** | gradient face, rim, kind glyph (context ▣, contract ⧉, external ◌), bottom **activity strip** (issue states as coloured segments, session pips) | id in mono caps, kind label, counts in badges | — |
| **Part at work** | cyan halo breathing, green session pips lit, its flows carrying particles | "2 sessions working" badge | M4, M1 |
| **Part lit by a plan** | cyan (affected) or violet (new) rim + glow; a violet pip per plan placed here | "affected by a plan", "+1 model" | M5 |
| **Seam chip** | conduit core lit by role; a magenta count bubble for open plans; collision throb | seam id, "2 plans" | M5, M12 |
| **Map band** | lens switch as a segmented control with an indicator; legend as drawn samples (particle dot, particle ring, proposed dashes) | lens names, legend words | M11 |
| **Scenario player** | step rail: one dot or ring per step, current lit, past filled | caption, "3 / 11" | M3, M7 |
| **Issue list rows** | progress ring (7 stations, Planned → Done; blocked is off the rail, a red ring with a gap; unknown a dashed grey ring) | #n, title, pins, state word | M6, M7 |
| **Issue panel (selected)** | **stage rail** across the top (Planned → Done, current lit, blocked as a red notch at the stage it left from when known, else at Started); **spend gauge** (arc, `spend_usd` against the plan's `spend_cap_usd`, amber at 80 %, red at cap); sessions as lanes with status LEDs | state word and reason, "last signal 2 min ago", pins, decisions, plan summary | M7, M10, M8, M6 |
| **Inspector** | kind glyph and a mini activity strip for the part or seam; flows listed with their shape glyph | description, flows, issues | M8 |
| **Plans tab** | header numerals (Proposed 3 · Open 2, 28 px mono); per plan a **mini map**: every part as a dot at its layout position, affected cyan, new violet, its seams lit; a fit glyph (✓ / ✕) and an approval stamp | issue, title, parts and seams chips, review and approval words | M5 on hover, M8 |
| **Where it fits** | the system strip as lit modules, data models as cards on their part | tables | M5 |
| **Connected repos** | each repo as a **rack unit**: two LEDs (in map, covered), four bars (open issues, plans, sessions, open PRs) scaled to the team's max, an amber count for unattributed sessions | repo name in mono, the numbers, the "not covered" word | M7 on bars |
| **Companions and lanes** | per companion a lane with a connection LED (M9 ping when connected), a **freshness bar** draining from `last_seen` towards the stale threshold (10 min), sessions as LEDs, queue as numbered slots | hostname, user, "last seen", queue titles | M9, M4 on working session LEDs, M7 |
| **Findings** | amber count chip in the map band and on parts (corner tick) | the finding text | M6 |
| **Draft panel** | violet blueprint: dashed frame, op list as a numbered rail | ops, buttons | M8 |
| **Login page** | the dot field and the wordmark, one lit slab as the card | the sign-in words | — |

Freshness bar: a client-side visual derived from `last_seen` and the clock; nothing is stored.
The 10 min threshold matches the default in AGENTS.md; the server's configured N is not in the
overview, so a team with another N sees the bar disagree with the alert (follow-up §9).

## 7. Layout at 900 px and below

`.body` still collapses to rows (55 % / 45 %). Glows are `box-shadow` (no layout). The map
canvas clips (`overflow: hidden`), so halos and particles never scroll the page. Badges wrap
inside the activity strip; the stage rail drops its labels to the current stage only below
600 px; the rack unit bars stack under the repo name. An e2e at 900 × 900 and 600 × 900 asserts
`document.documentElement.scrollWidth <= innerWidth` and that the panel's bounding box is inside
the viewport.

## 8. Neighbours and what must hold

- **Data versus control without colour**: dot and ring in M1 and M3, in the legend, in the
  scenario steps list (`.flow-kind`), and in the static markers under reduced motion. The
  existing e2e for the legend and the scenario keeps passing; a new Vitest asserts the particle
  element for a control flow has the `control` class and a ring geometry (fill `--canvas`).
- **Unknown never looks working**: M4 runs only for `working`; `unknown` is a dashed grey ring
  everywhere (AGENTS.md progress model).
- **No CDN**: the e2e records every request; any host other than the app's fails it.
- **The slice test** (`web/src/features/features.test.ts`): the motion switch lives in
  `shared/`, which slices may name; new visual components live in the slice that shows them
  (`map` for the activity strip, `issues` for the stage rail and ring, `repos` for the rack unit,
  `companions` for the freshness bar); a ring used by two slices goes in `shared/`.

## 9. Follow-ups (need data the web does not have)

1. **Sparklines of spend per issue over time** — needs a spend history in the overview or an
   endpoint; today `spend_usd` is a snapshot.
2. **Observed traffic on seams** (particles that mean traffic, map-versus-territory findings
   drawn on conduits) — needs the server to summarise observed calls.
3. **The stale threshold N in the overview**, so the freshness bar matches the alert. Done by
   #105: `Overview.companion_silent_secs`, and the gauge `rubato.companion.silent_secs` for the alert.
4. **Which stage a blocked issue left from** — `Progress` gives the state, not the previous one.

## 10. Failure

All of this is presentation. The failure modes are visual: text that fails contrast (the test),
motion that runs when it should not (the reduced-motion e2e), a layout that overflows (the 900 px
e2e), dropped frames on big maps (the performance e2e), and a font that does not load (fallback
stack plus the no-CDN e2e). None crosses a seam, so there are no O-rows; a broken page is seen by
whoever opens it, and the e2e suite is the signal before that.
