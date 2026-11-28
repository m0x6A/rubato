---
name: rubato-ux
description: Design or review Rubato's own web UI (web/) so the user always knows where they are, what they can do and what to do next. Use when adding or redesigning a screen, panel, menu or flow in web/, when changing motion, glow or styling, when asked to declutter or simplify a view, or when asked to review a screen or the whole app for UX. Gives the core rules, the motion and glow rules, a workflow from scenario to checklist, and the anti-patterns to catch.
---

# rubato UX/UI design

Design every rubato screen the way an Apple designer would. The goal is not a pretty UI. It is a UI where the user always knows **where they are, what they can do, and what to do next**. Decluttering is a side effect of designing this way, not a separate step.

The glow and motion stay. They are part of rubato's identity. They have to serve clarity, though, and never compete with it.

## The core rules

1. **Start from the task, not the data.** Every screen exists for one job a person (or a person with their agent) is doing. Name that job in one sentence before designing anything. If you can't, the screen shouldn't exist, or it is two screens.
2. **One primary action per screen.** Exactly one thing is visually the obvious next step. Secondary actions are quieter. Rare actions sit behind a menu, a disclosure or a settings view.
3. **Show only what the task needs right now.** For every element on screen, ask: does the user need this *to complete this task, at this moment*? If not, move it to on-demand (hover, expand, detail view, inspector) or remove it.
4. **Progressive disclosure.** Simple first, depth on request. The default view works for 80% of visits; detail is one deliberate action away, never zero and never three.
5. **Clear paths, no dead ends.** Each user path has a visible start, a clear next step at every point, and an obvious finish or way back. No screen should leave the user wondering "now what?"
6. **Hierarchy through restraint.** Use size, weight, spacing and position for hierarchy before you reach for color, borders, badges or icons. Generous whitespace is a feature. Fewer competing accents.
7. **Consistency over cleverness.** The same thing looks and behaves the same everywhere. Reuse existing components and patterns before inventing new ones.
8. **Deference.** The UI recedes and the user's content (plans, work, collaborators, agent state) is the hero. Chrome, labels and decoration stay as quiet as possible.
9. **Respect the app vs workspace boundary.** Application (server) settings and workspace settings never mix on one screen; app settings aren't editable at runtime.

## Motion and glow

Motion is allowed and encouraged when it does one of these jobs:
- **Orients:** shows where something came from or went (a panel sliding in from the item that opened it).
- **Signals state:** something is live, working, waiting or done (an agent running, a collaborator present).
- **Draws attention once:** to the primary action or a new event, then settles.

Rules:
- At most one thing on screen moves for attention at a time. Ambient glow is fine but stays calm.
- Motion never blocks input and never delays the user's next step.
- Glow carries meaning (live, active, focus). Don't use it as decoration on static elements.
- Respect `prefers-reduced-motion`: keep the meaning, drop the movement.

## Where things are in this repo

- **Scenarios:** `features/<capability>.feature`, ids `@<capability>.<slug>` (ADR 0069). `features/look.feature` holds the look-and-feel ones (widths, colour schemes, reduced motion).
- **Tokens and styling:** `web/src/styles.css`. Colours, the `--glow-*` shadows, durations and easings are custom properties at the top; use them, don't write raw values.
- **The motion switch:** `web/src/shared/motion.ts` sets `data-motion` on `<html>`. Every animation and transition is written under `:root[data-motion="full"]`; JS that moves things reads `useReducedMotion`. New motion goes through the switch, never around it.
- **Components:** one folder per feature under `web/src/features/`, shell in `web/src/App.tsx`. Look there for an existing pattern before adding one.
- **E2E:** Playwright specs in `web/e2e/` (`look.spec.ts`, `motion.spec.ts`, `ux.spec.ts` and one per capability); each test cites its scenario ids in its `tag` detail, and `pnpm scenarios check` verifies that.
- **Earlier findings:** `docs/design/ux-walkthrough-2026-11-26.md`.

## Workflow

Follow these steps for any new screen, redesign or review. Produce the outputs in the PR description or a design note in the repo (`docs/plans/<issue>-<slug>/design.md` on the full track).

### 1. Read before designing
- Read the existing components, layout, design tokens and styling setup in the repo so the design fits what's already there.
- Read the written scenarios (`features/*.feature`) that touch this screen. Scenarios are the source of truth for user paths. If none exist for the task, write one first.

### 2. Name the job and the user
Write:
- **Who:** a person, a person with their agent, or (later) an agent-only client.
- **Job:** one sentence: "I want to ___ so that ___."
- **Done looks like:** what the user sees when the job is finished.

### 3. Map the path
Write the path as numbered steps from entry to done, including where the user came from and where they go after. Mark the **one primary action** on each step. Flag any step with more than one plausible next step, a dead end, or a backtrack. Those are design bugs.

### 4. Audit every element
For an existing screen, list everything on it in a table:

| Element | Needed for the job now? | Decision |
|---|---|---|
| … | yes / sometimes / no | keep / on demand / move elsewhere / remove |

Be ruthless. "Nice to know" is "on demand". "Someone might want it" is "remove" until a scenario needs it.

### 5. Design the screen
- Place the primary action where the eye lands last in the reading flow (or where the content naturally ends).
- Group what's left into as few groups as possible. Each group gets a clear label only if position alone doesn't make its purpose obvious.
- Write copy short and in plain verbs: "Start plan", not "Initiate planning session workflow".
- Design empty, loading, error and "agent working" states. They are part of the path, not afterthoughts.
- Decide which motion, if any, orients or signals state on this screen.

### 6. Review against the checklist
Before calling it done, check that:
- [ ] A first-time user can say what this screen is for within 3 seconds.
- [ ] There is exactly one primary action, and it is visually obvious.
- [ ] Every visible element passed the audit (needed now).
- [ ] The path from the scenario works with no dead ends, and the way back is obvious.
- [ ] Empty, loading, error and agent-working states are designed.
- [ ] At most one element moves for attention; glow means something.
- [ ] Reduced motion keeps the meaning.
- [ ] Existing components and tokens are reused; anything new is justified.
- [ ] App settings and workspace settings are not mixed.
- [ ] Keyboard navigation and focus order follow the path; contrast is readable.
- [ ] The scenario's e2e test covers the path (add or update it).

### 7. Report
Summarize: the job, the path, what was removed or moved behind disclosure and why, and any open questions. When a trade-off is a judgment call (for example removing information some user might want), say so as a numbered question with options and let Mattias decide.

## When reviewing without changing code
If asked to review a screen or the whole app, do steps 2–4 for each screen and return a prioritized list: worst path problems first, then clutter, then polish. For each item give the problem, the user impact, and the proposed fix.

## Anti-patterns to catch
- Dashboards that show everything "in case". Show what this user needs for their next action.
- Several buttons of equal weight side by side.
- Settings, metadata, IDs or debug info on task screens.
- Navigation that exposes every feature at the top level.
- Badges, counts and status dots that don't change what the user does.
- Motion on several elements at once, or glow on things that aren't live.
- Modals that interrupt a path when an inline step would do.
