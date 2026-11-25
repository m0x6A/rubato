# UX walkthrough, 2026-11-26

A walk through every screen of the web UI as someone new to Rubato, on the seeded e2e server
(`web/e2e/run.sh` data), at 1440×900 and 390×844. Each finding names where it was seen and
whether this branch fixes it. Decider for the fixes: agent (Claude), for a person to review.

## Fixed here

| # | Where | What confused | Fix |
| --- | --- | --- | --- |
| 1 | Panel tabs | At 1440 px the tabs wrapped onto two rows ("Repos" and "WIDER" alone on the second), and "(3 proposed)", "(0 live)" made the row long. | One row: each count is a quiet chip, the accessible name still says what it counts; the panel-size button is a glyph with its name as tooltip. |
| 2 | Issues list and Not pinned | An unpinned issue with a plan (#901, #914, #915) was listed twice: once tracked ("not pinned") and again under Not pinned. | Not pinned leaves out what the list above already shows (`@issues.listed-once`). |
| 3 | Repos tab | The rack said "9 open issues", the folded list under it "12 open issues": the same three issues counted twice. | The folded list counts each issue once. |
| 4 | Header | "4 ISSUES" next to "9 open issues" in Repos, with nothing saying they count different things. | "4 tracked issues", with a tooltip saying what tracked means. |
| 5 | Map and panel | Escape did nothing: the inspector or an open issue could only be closed with its button. | Escape closes the innermost thing: the new-issue form, then the issue, then the selection; never while typing or in a menu (`@map.escape-closes`). |
| 6 | Inspector | Two "new issue" buttons at once: "New issue…" above the inspector and "New issue here…" in it. | The generic button shows only when nothing is selected. |
| 7 | New issue form | Started with nothing selected, "Where it fits" silently chose the first part (rubato-protocol): an issue would be pinned somewhere nobody picked. | It starts on "Choose a part or seam…" and Create waits for a choice (`@new-issue.choose-place`). |
| 8 | Inspector header | The part's id broke over two lines ("rubato-\nserver") because the two buttons squeezed it. | The id has its own row; kind and buttons sit under it. Headers without a kind (New issue) keep one row. |
| 9 | Inspector body | "Touching it now" (issues, plans, sessions) came last, under a long list of flows. | It comes right after the description. |
| 10 | Part cards | "3 planned here" read as three issues in the *Planned* state; it counts open plans of unpinned issues that name the part. | "3 plans name it", with a tooltip naming the issues. |
| 11 | Issue detail | A large $0.00 gauge before anything ran. | One line, "spend cap $5 · nothing spent yet", until there is spend, a session or history. |
| 12 | Issue detail | "No companion of yours is connected; start one with rubato-companion run" was a dead end for someone who never paired. | It says a companion starts sessions and offers "pair one first", which opens Companions (`@companions.pairing-steps`). |
| 13 | Companions | An empty box with "code shown by rubato-companion pair" and a disabled Approve: no way to learn what to run. | Three steps, with the exact `rubato-companion pair --server <this Rubato>` and `run` commands, and a labelled code field. |
| 14 | Traffic lens | "Set one in Configure workspace." was plain text. | A "Configure workspace…" button opens the settings. |
| 15 | Edit map | Turning on Edit map changed nothing visible on the map except handle dots: how to add a part or draw a seam was nowhere. | A hint above the canvas says how, and that nothing changes until the draft is merged (`@map.edit-hint`). |
| 16 | Phone width | The lens buttons broke each label over three lines; after that, the map column forced the page wider than the screen. | The lens row scrolls sideways; the map column may shrink (`min-width: 0`). |
| 17 | Plan detail | "Open issue" looked like bold text, not a control. | A button, "Open issue →". |
| 18 | Stage rail | Seven segments, only the current one labelled. | Each station names itself on hover. |
| 19 | Not pinned | An issue key there leaves for GitHub, while the same key in the list above opens it in Rubato. | The external link is marked ↗ with "Open on GitHub". |
| 20 | Header | "map rubato @ 5fc6857" unexplained. | Tooltip: the commit the map was read at. |

## Seen and left for later

- **Seam labels overlap on the flat map**: `companion-mcp` sits over `companion-claude-code`; on Data flow the flow labels pile up around `rubato-server`. A layout change (label placement, ELK), larger than this pass.
- **The issue detail says the plan twice**: Where it fits shows the Parts affected table, and "The plan on the map" below lists the same changes.
- **Where it fits lists all 13 parts** as chips, with the affected ones outlined; only the affected ones matter to the reader.
- **Review and approval contradicts Accept plan** while the cutover is unset: the section asks for an `Approved:` comment and the card offers Accept plan, which then says this Rubato cannot comment. Goes away once the cutover is set (ADR 0055).
- **The scenario caption** prints the whole flow text of a step and is cut at two lines.
- **Plans tab with an issue open** shows that issue's plan, not the board; going back takes "← all plans".
- **Telemetry relay 404**: with no relay configured, every page load logs one failed request in the console (the client stops after it, `shared/trace.ts`).

## How it was checked

- `pnpm typecheck` and `pnpm test` in `web/`: 476 unit tests pass.
- `web/e2e/ux.spec.ts`: five browser tests for the scenarios above, against the seeded servers.
- The full `web/e2e/run.sh` suite, and `pnpm scenarios check`.
- Screenshots before and after at 1440×900 and 390×844; the page no longer scrolls sideways on a phone.
