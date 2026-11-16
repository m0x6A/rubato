# Design: open on GitHub by default, done views, a plans board, a wider panel (#265)

Plan: [../265.md](../265.md) · ADR proposed: [0079](../../decisions/0079-plan-text-renders-as-markdown-in-the-web.md)

## 0. Where it fits

Parts: `rubato-protocol`, `rubato-server`, `web`. Seam: `web-server` (the Overview gains one
field). Nothing else on the map moves; the GitHub poll is not touched. The companion, the skill, the plan file and the plan check are untouched.

Why the UI is stale today:

- The overview's issue set is a union of pins, launches, plan rows, drafts and sessions
  (`crates/rubato-server/src/overview/build.rs:278-282`). Launches and drafts are never deleted,
  pins only by hand, and a plan row survives its deleted branch.
- `issue_meta.github_state` is stored (`github_poll/poll.rs:36-98`) and used only to mute
  `MapVersionMismatch` (`build.rs:490-504`).
- Done is "a pull request from the `issue/<n>-` branch merged" (`rubato-protocol/src/progress.rs:136`).
  An issue closed any other way keeps its last state.
- The web hides Done in the plans list, the map and the repo view, each with its own filter, and
  not at all in the Issues list, its count or the top-bar track.

## 1. One rule: finished

```
closed   = GitHub says the issue is closed            (server fact, on IssueSummary)
merged   = progress is Done                           (already derived)
at_work  = a session of the issue is working or blocked, by its effective status
           (a session quiet for longer than session_stale_secs is unknown, not working)
finished = merged or (closed and not at_work)
```

- `closed` is a field, not a progress state. The progress model keeps eight states and Done
  keeps meaning merged; the rail, `blocked_from` and the gauges do not change shape.
- An issue with no `issue_meta` row reads as open. Every tracked issue is in the poll set: a
  draft needs a launch (`plan_drafts/intake.rs`) or an imported branch plan, and both are polled.
- `at_work` keeps a closed issue visible while an agent still spends on it (plan A1). "Not
  ended" would be wrong: session snapshots are never deleted, so an idle pane or a dead
  companion's last snapshot would hold an issue open for ever. Idle, ended and stale sessions do
  not count.
- Merged is finished whatever its sessions, as `group.ts` and `graph.ts` treat it today.
- The rule exists twice, once in Rust (`build.rs`, a small `fn finished`) and once in the web
  (`shared/finished.ts`). Both are a few lines over fields that cross the seam; sending a derived
  `finished` as well would give two facts that can disagree. A test on each side covers the same table of
  cases.

## 2. Server

`rubato-protocol/src/progress.rs`: `IssueSummary.closed: bool`, `#[serde(default,
skip_serializing_if = "std::ops::Not::not")]`, like `outside_rubato`.

`overview/build.rs`:

- Read `closed` from `meta` once per issue, where `MapVersionMismatch` reads it today, and set
  it on the summary.
- Per-issue findings are computed before the progress state is known in places; compute
  `finished` after `derive_progress` and drop the issue's plan findings when it is true:
  `UnknownIds`, `FlowOffSeam`, `PlanParseError`, `ApprovalStale`, `ApprovedWithoutFit`,
  `CommentApprovalIgnored`, `DraftOwnerConflict`. Handoff findings stay: an approval that never
  reached its branch is a "never silent" item whatever the issue's state.
- `SeamCollision` and `counts()` (`seam.open_plans`) skip finished issues, where they skip Done
  today. `connected_repos().plans` counts every plan today; it skips finished ones too.

`rooms/notices.rs`: the `merged` set becomes "merged or closed", read from `issue_meta`, with
no at-work exception (plan A7).

`overview/gauges.rs`: `rubato.issues.by_state` records every (`rubato.issue.state`,
`rubato.issue.finished`) pair, zeros included, as `blocked_from_counts` does: the gauge is
synchronous, so a pair that is not recorded keeps its last value. `plans_not_fitting` and
`plans_fit_none` skip finished issues. The panel "Issues by state" sums over the new attribute
and gets a second query split on it.

The poll is not touched. An issue deleted or transferred on GitHub reads as open
(`github_poll/poll.rs:91`); that is plan A8, a follow-up.

No migration. No route. No new finding.

Concurrency and idempotency: the overview is rebuilt from stored rows on every request; nothing
is written by this change. A webhook `issues.closed` and the poll both end in the same
`issue_meta` upsert.

## 3. Web: open by default, Done on request

- `shared/finished.ts`: `isFinished(i)`. Callers: `plans/group.ts`, `map/graph.ts` (`isOpen`
  and the part and seam issue lists), `map/explain.ts` (the Inspector's issues and sessions),
  `map/PartCard.tsx` (the strip), `repos/RepoWork.tsx`, `companions/Lanes.tsx` (a queued issue
  that finished is marked and offers only Remove), `issues/IssuePanel.tsx`,
  `issues/visuals.tsx` (`TeamTrack`), `App.tsx` (tab count).
- `WhereItFits` and `DraftSection` take `readOnly`: no Accept, Import, Approve or Resume. The
  issue detail and the plan detail pass it for a finished issue.
- `IssuePanel`: a two-button switch, `role="tablist"`, *Open n* / *Done n*, above the list.
  Done rows are the same row component with the actions left out and a *closed* badge when
  `closed` and not merged. Selecting a done issue opens the same detail, read-only where an
  action would start work.
- A map part or seam selection filters whichever view is showing.
- Empty states: Open with none says "Nothing open on GitHub." and names the Done count; Done
  with none says "Nothing finished yet."

## 4. Web: panel sizes

- `App.tsx` holds `panelSize`, written to `<div class="body" data-panel="…">` and to
  `localStorage["rubato.panel"]` in try/catch.
- CSS, all inside `@media (min-width: 901px)` so a stored size never reaches the stacked
  layout: normal `1fr 400px` (today); wide `1fr min(60vw, 900px)`; full `1fr`, one column, the
  map section `hidden` (so React Flow does not lay out at zero width and is out of the tab
  order). `hidden` is set only when the same media query matches.
- The button sits at the end of the tab `nav`, text glyph plus an accessible name "Widen panel",
  "Fill the window", "Back to normal width".
- Under 900 px the grid already stacks; the button is not rendered (plan A5).
- Returning from full re-fits the map once.

## 5. Web: the plans board

```
Plans tab
├─ counts: proposed · open · do not fit        (as today)
├─ switch: Open | Done
├─ Open view:  [ Proposed ] [ Open ]           columns; one column when the panel is normal
├─ Done view:  one list, newest first
└─ plan detail (replaces the board while a plan is selected)
   ├─ Back to plans · Open issue
   ├─ WhereItFits (fit, parts, seams, Accept / Import)
   └─ DraftSection (revisions, what changed, files as Markdown)
```

- `PlansBoard.tsx` replaces `PlansPanel.tsx`; `Row` is kept. Columns use a container query on
  the panel, so the layout follows the panel size without knowing about it.
- Selecting a row sets the shell's `selectedIssue` (so the map paints the plan) and the board's
  own "showing detail" state; the tab stays on Plans. *Open issue* is the old behaviour.
- `plans` imports nothing from `issues`; the detail composes `plans`' own `WhereItFits` and
  `DraftSection`. The room stays in the issue detail.
- A done row has no *Accept plan*; its detail passes `readOnly` (§3).
- `groupPlans` keeps issues with a plan, a fit or a draft. A draft-only row shows *draft*, its
  revision, owner and fit; it has no mini map, because the overview carries no parsed draft plan
  (plan A6).
- A plan with no Rubato draft has no body in Rubato; the detail links to the file on GitHub
  (plan A3).

## 6. Web: plan text as Markdown

ADR 0079. `features/plans/Markdown.tsx`, loaded with `React.lazy`, wraps `react-markdown` +
`remark-gfm` with: no raw HTML, `img` rendered as a link, links in a new tab with `rel`.
`DraftSection` renders a file through it when its path ends in `.md`, with a *Raw* toggle per
file; the diff view stays raw. Reading styles (`.md-body`) come from existing tokens: UI font,
15 px, 75ch measure, tables scroll inside their own box so the panel never scrolls sideways.

## 7. Caveats

| Risk | Why here | Guard |
| --- | --- | --- |
| Everything vanishes | a wrong `closed` hides an open issue | absent meta reads as open; integration test for reopen; the gauge split |
| Two rules drift | Rust and TS each derive finished | the same table of cases tested on each side; the web grep test |
| A hostile plan | plan text from an imported branch | ADR 0079 rules; the four hostile inputs test with its must-fail twin |
| Hidden spend | a closed issue with an agent at work | `at_work` keeps it in Open (A1) |
| Held open for ever | snapshots are never deleted | only fresh working or blocked sessions count |
| Stale gauge series | a synchronous gauge keeps its last value | every pair recorded, zeros included |
| Approving a finished plan | reused parts carry their actions | `readOnly`, tested in both details |
| Goldens churn | closed fixtures change shape | re-record once in PR 1, diff read and described in the PR |
| Bundle size | Markdown tree | lazy chunk; the perf project's budget on first paint is unchanged |
| Panel lost | full size, or a stored size on a phone | one column and `hidden` only above 900 px; width and 600 px tests |

## 8. Not in this change

- Deleting or archiving rows; unpinning closed issues.
- Rendering Markdown anywhere else (issue bodies, room messages).
- Fetching a plan file's body for plans without a draft.
- An issue deleted or transferred on GitHub (plan A8).
- A resizable (drag) panel.
