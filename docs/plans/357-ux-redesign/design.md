# Design: routed views, one job per screen (#357)

Plan: [../357.md](../357.md) · Skill: `.claude/skills/rubato-ux/SKILL.md` · no ADR.

This note holds what the `rubato-ux` skill asks for: for every screen, who uses it and for what
job, the path with one primary action per step, the audit of every element, its states and its
motion. Line references are to `web/src/` on `main` at 74d033d.

## 0. Where it fits

Part: `web`. Seam `web-server` is touched only in which reads a view fires; no route, type or
schema changes. The server, the companion, the skill, the plan file and the plan check are
untouched.

## 1. What is wrong today

| # | Problem | Where | What it costs the person |
| --- | --- | --- | --- |
| 1 | No view has a URL. The tab is `useState`, only `?t=` and `?w=` are in the URL, there is no history. | `App.tsx:51`, `shared/workspace.ts:51-65` | Reload returns to Issues, Back leaves the app, an issue cannot be linked in a chat or a review. |
| 2 | One 400px panel carries everything, chosen by a chain of conditions, with several things stacked. | `App.tsx:552-707` | Findings and the map draft sit above every tab; the inspector, the new-issue form and the list show at once; settings, members and the operator page are squeezed into 400px. |
| 3 | The issue detail is one scroll of about fifteen sections for ten jobs. | `features/issues/IssuePanel.tsx:89-275`, `App.tsx:697` | The next step is somewhere in the scroll. Decisions, O-rows, seam questions and spend are each shown twice. |
| 4 | The map band stacks up to five blocks above the canvas. | `features/map/MapView.tsx:914-1090` | The map, the thing the screen is for, loses a third of its height in the Traffic lens. |
| 5 | The top bar mixes up to eleven items of different kinds; three toggles look alike. | `App.tsx:473-510` | Nothing says which are places, which are switches and which are status. |
| 6 | A part card shows up to eight kinds of badge, a seam chip up to seven fragments. | `features/map/PartCard.tsx:74-127`, `SeamChip.tsx:41-69` | Badges that do not change what you do hide the one that does. |
| 7 | Pairing instructions sit above the lanes for good. | `features/companions/CompanionsPanel.tsx:39-59` | A daily view starts with a once-per-machine task. |
| 8 | Repos nests pull requests and issues in `<details>` inside each rack; a pull request opens inline. | `features/repos/RepoWork.tsx:26-77` | A merge happens three levels deep in a 400px column. |
| 9 | Configure workspace also creates and deletes workspaces. | `features/settings/SettingsView.tsx:386-424` | Two jobs, one of them destructive, on one form. |
| 10 | The map draft panel holds six buttons and four forms, two of them always open. | `features/drafts/DraftPanel.tsx:293-322, 429-499` | No one primary action; the change list, which is the draft, is the smallest part. |
| 11 | Plans board: big counts repeat the column headings; Accept sits on every row. | `features/plans/PlansBoard.tsx:200-210, 64-74` | Several equal buttons side by side; accepting without reading is one click. |
| 12 | Four two-step confirms and two menus are each written by hand. | `SettingsView.tsx:405`, `MembersView.tsx:113`, `AdminView.tsx:208`, `SessionChat.tsx:131` | The same thing looks and behaves slightly differently in each place. |

## 2. Routes

The fragment names the view. The query keeps `?t=<tenant>&w=<workspace>` as today
(`shared/workspace.ts` already keeps the fragment in every `replaceState`).

| Fragment | View | Up (Escape, back link) |
| --- | --- | --- |
| `#/issues` | issue list, Open | — |
| `#/issues/not-pinned`, `#/issues/done` | issue list, the other segments | `#/issues` |
| `#/issues/new?at=<kind>:<id>` | new issue | `#/issues`, or the inspector it came from |
| `#/issues/<owner>/<repo>/<n>` | issue, Overview | `#/issues` |
| `#/issues/<owner>/<repo>/<n>/plan` · `/room` · `/sessions` · `/pr` | issue, that section | the issue list, or where it was opened from (rule 3) |
| `#/plans`, `#/plans/done` | plans board | — |
| `#/companions` | lanes | — |
| `#/companions/pair` | pair a companion | `#/companions` |
| `#/repos` | repos | — |
| `#/repos/<owner>/<repo>` | one repo | `#/repos` |
| `#/pulls/<owner>/<repo>/<n>` | one pull request | its repo |
| `#/map?sel=part:<id>`, `#/map?sel=seam:<id>` | inspector of the selection | `#/issues` |
| `#/map/draft` | map draft (edit mode) | `#/issues` |
| `#/map/territory` | map versus territory | `#/issues` |
| `#/findings` | findings | `#/issues` |
| `#/settings`, `#/settings/workspaces`, `#/members`, `#/operator` | full pages | `#/issues` |
| anything else | not found | `#/issues` |

Rules:

1. **Why the fragment.** The server answers an unknown path with `index.html` and status 404
   (`crates/rubato-server/src/app.rs:392`, pinned by the test at `:1262-1285`). Path routes would
   make every deep link a 404 to a proxy or a monitor. The fragment needs no server change.
2. **The map selection is a parameter**, `?sel=part:<id>` or `?sel=seam:<id>` inside the fragment,
   valid on any view that shows the map, and the only place the selection is written. The
   inspector is the view `#/map` with a selection; it has no id in its path. It has to be a
   parameter because an issue can be open while a part stays selected (`@map.escape-closes`),
   the list filters by it (`IssuePanel.tsx:279-283`), the draft's change form follows it
   (`DraftPanel.tsx:353`) and "take into draft" sets it (`App.tsx:391-393`).

   | On this view | A click on a part or seam | A click on the empty map | Escape |
   | --- | --- | --- | --- |
   | the inspector (`#/map`) | selects it, same entry (replace) | clears the selection and shows the issue list (replace) | the issue list, nothing selected |
   | the map draft, map versus territory | selects it, stays on the view (replace) | clears the selection (replace) | the issue list; the selection stays |
   | an issue, a list, a board, any other panel view | opens the inspector for it (push) | clears the selection (replace) | the view's parent; with a selection, the inspector for it; at a top view, clears the selection |
   | a full page | the map is hidden | — | where the person came from |

   So from an issue opened beside a selected part, the first Escape closes the issue and shows
   the part's inspector, and the second deselects it (`@map.escape-closes`, `ux.spec.ts:23`).
   Until PR 4 the inspector still shows above the issue list, as today, and the fragment is
   `#/issues?sel=…`.
3. **History and the way up.** A change of view pushes. A change of only `sel`, and the switch
   made while an issue is dragged over Companions (`App.tsx:578`), replace. Escape and a view's
   back link go *up*, never `history.back()`: a pasted link has no history. Up is the parent in
   the table, with one refinement: an issue section opened from the plans board, a lane or a repo
   carries `from=plans`, `from=companions` or `from=repo` in the fragment, and its way up is
   that view, so reading several plans from the board returns to the board each time. A full page
   returns to the view it was opened from, remembered for the visit; opened from a link, to the
   issue list. The browser's own Back always works beside this.
4. **Links inside a view.** A link to something on the same page (the session log's "see the
   reply", `chat/SessionChat.tsx:185,229,232`) scrolls to it and focuses it; it never writes the
   fragment, which would otherwise read as an unknown view.
5. **Login.** The OAuth callback redirects to `/` (`crates/rubato-server/src/auth/handlers.rs:222`),
   dropping the query and the fragment. The login page stores both (`?t=&w=` and the fragment)
   in `sessionStorage`, and the app restores them once after login, unless the person came back
   to an address that already names something.
6. **Tenant and workspace.** Switching either goes to `#/issues`, because an issue or a part of
   one workspace means nothing in another (`App.tsx:133`, `:440`, `:482`). The exception is a
   switch made from the settings pages (a workspace just created, or `default` after a delete),
   which stays on the settings of the workspace just chosen. An operator in no
   tenant is sent to `#/operator` (today forced at `App.tsx:554`).
7. **Not found.** A fragment that parses to nothing, an issue the overview does not hold, a part
   the map does not have: the view says what it could not find and offers the way up. While the
   overview is still loading it says "Loading…", not "not found".
8. **Full pages** hide the map as `App.tsx:533` does for the full panel size, under a `data-page`
   attribute. The map stays mounted, so it keeps its lens and positions, and `data-panel` with its
   stored size is left alone (`look.spec.ts:96`).
9. **State that must outlive a section.** A section reads on mount (`live/useWebResource.ts:52`),
   so switching sections refetches; that is acceptable and bounded by one read per switch. What a
   person typed must not be lost: the room's unsent text and optimistic posts
   (`room/RoomSection.tsx:57`), a session's message box (`chat/SessionChat.tsx:249`) and the nine
   new-issue fields (`new_issue/NewIssueForm.tsx:51-59`) are held above the section, keyed by
   issue; the new-issue form is keyed by where it was begun, and keeps its request id
   (`NewIssueForm.tsx:50`) with the fields, so a remount never turns one Create into two issues.

`shared/route.ts` exports the `Route` union, `parseRoute`, `formatRoute`, `parentOf`,
`useRoute()` over `hashchange`, and `navigate(route, { replace? })`. No dependency.

## 3. The shell

**Who:** everyone, on every visit. **Job:** "I want to know which team and application I am
looking at and get to a place, so that I can start on what I came for." **Done:** the view they
wanted is in the panel.

```
┌ Rubato   Shop ▾                                      ● live    m0x6A ▾ ┐
├─────────────────────────────────────────────┬──────────────────────────┤
│  Structure  Data  Control  Traffic  Scenarios│ Issues  Plans 3          │
│                           View ▾   Edit map  │ Companions 2  Repos   ⇤  │
│                                              ├──────────────────────────┤
│                    map                       │  one view                │
│                                              │                          │
│                                              │                3 findings│
└─────────────────────────────────────────────┴──────────────────────────┘
```

| Element today | Needed now? | Decision |
| --- | --- | --- |
| "Rubato" | yes | keep; it links to `#/issues` |
| map name @ sha | sometimes | on demand: first line of the workspace menu, with its tooltip (Q3) |
| Tenant switcher (2+ tenants) | sometimes | move: the account menu, since it changes who you act as part of |
| Workspace switcher (2+ workspaces) | yes | keep as the workspace menu's button, drawn also with one workspace, because Configure workspace must always be reachable (ADR 0075, `@settings.single-workspace`) and the button names where you are |
| "Configure workspace" toggle | sometimes | on demand: an item of the workspace menu; opens the full page |
| Team track and "N tracked issues" | sometimes | move: the head of the issue list, where the count means something |
| live dot | yes | keep |
| "live · refreshed N ago" | sometimes | on demand: the dot's title; visible text when disconnected or stale, because a signal that stopped must never look live (Q7) |
| "Members" toggle | sometimes | on demand: account menu |
| "Operator" toggle | rarely | on demand: account menu, operators only |
| login name | yes | keep as the account menu's button; the menu ends with Log out, which the web does not offer today although `@auth.log-out` describes it; PR 3 adds it with its e2e test |
| panel nav: four entries with counts | yes | keep the entries; counts only on Plans (proposed, waiting for a person) and Companions (live) (Q4) |
| panel size button | sometimes | keep where it is |
| Findings list above every tab | sometimes | move: its own view; a quiet "N findings" link at the foot of the panel nav, hidden at zero |
| Map draft above every tab | sometimes | move: its own view (§9) |
| error, no-tenant and notice banners | yes | keep; the notice keeps Undo and Dismiss |

States: loading ("Loading the map…", the panel shows the nav and "Loading…"); no tenant (the
banner, and for an operator the Operator page); signed out (the login page, fragment kept);
disconnected (the dot and its text). Motion: the nav indicator slides to the current entry (M11);
a view change uses the panel opening (M8) once; nothing else moves.

New shared pieces, each replacing copies: `shared/Menu.tsx` (button, `role="menu"`, arrow keys,
Escape, from `pins/PinMenu.tsx:96-100` and `queue/QueueMenu.tsx:79-90`) and `shared/Confirm.tsx`
(a button that arms, then asks once inline, for the four confirms in §1 row 12; the room's
call-in form, `Presence.tsx:195`, is a form with a cost estimate and stays one).

## 4. The issue view

**Who:** a person, often with their agent working on the issue. **Job:** "I want to see where
this issue stands and do the one thing it waits for, so that it moves." **Done:** the state in
the header has moved on, or the header says it waits for someone else.

```
← Issues
#412  Rotate signing keys                                    ↗
● Planned ─ ● Started ─ ● Designed ─ ○ Building ─ ○ In review ─ ○ Done
Designed · last signal 4 min ago                  [ Review plan ]
──────────────────────────────────────────────────────────────
Overview   Plan   Room 3   Sessions 1   Pull request
──────────────────────────────────────────────────────────────
(the chosen section)
```

Path, with the one primary action per step:

The header offers the action of the **first row that holds**, so there is never more than one.
It is a function of five things, not of the progress state alone: whether the issue is finished,
its pull request, whether a revision waits for approval, the sessions (whose, and alive or not),
and the viewer's companions (none paired, paired but none connected, connected).

| # | Holds when | The header says | Primary action | It leads to |
| --- | --- | --- | --- | --- |
| 1 | the issue is finished | the finished note | none | — |
| 2 | a pull request is open | "checks green", or what fails | **Review pull request** | Pull request, where Merge is the primary action |
| 3 | a draft revision or a plan waits for approval | "revision N waits for approval" | **Review plan** | Plan, where Approve is the primary action |
| 4 | Blocked on a question at a seam | the question | **Answer in the room** | Room |
| 5 | a session of yours is alive (working, waiting or blocked) | its agent line, or why it is blocked | **Open session** | Sessions |
| 6 | a teammate's session is alive | "X's session is working" | none; it says who it waits for | — |
| 7 | the issue has no pin | "Not on the map yet" | **Pin to…** | row 8 or 9 |
| 8 | you have paired no companion | "A companion starts sessions" | **Pair a companion** | pairing (`#/companions` until PR 8 gives it its own view) |
| 9 | your companions are paired but none is connected | "Start `rubato-companion run` on <host>" | none; nothing in the browser can start it | — |
| 10 | a draft of yours is parked on a branch | "revision N, parked" | **Resume on <branch>** | a session |
| 11 | otherwise | — | **Start session** (beside it, behind a menu: "Start as lead agent" when the flag is on) | a session |

A spend or attempt cap that stopped the session shows as the reason in row 5; raising a cap is
configuration and stays outside Rubato. A step with two plausible next steps is a design bug:
the only place two starts existed, "Start session" and "Start as lead agent"
(`IssuePanel.tsx:130-144`), becomes one button with the lead option behind it.

An issue nobody tracks (in `overview.backlog`, not `overview.issues`) has no issue view: its
row under Not pinned offers **Pin to…** and a link to GitHub, and pinning it makes it tracked and
opens it.

Audit of the detail (`IssuePanel.tsx:89-275` and `App.tsx:696`):

| Element | Needed now? | Decision |
| --- | --- | --- |
| back link, key, title, GitHub link | yes | keep: header |
| Stage rail | yes | keep: header |
| state badge, last signal, reason | yes | keep: header, one line |
| finished note | when finished | keep: header |
| Start buttons, the no-companion text | when Planned | become the header's next action |
| Spend gauge, sparkline, cap note | sometimes | Overview; the header shows spend only when it is at 80% of the cap or over |
| "Pinned to" with unpin, Pin to… | sometimes | Overview |
| Sessions table, agent line | when one runs | Sessions; the header carries the agent line of the working session |
| Session conversation, compose, Stop | when you talk to it | Sessions; one conversation open at a time; Stop through `Confirm` |
| Draft: revision, changes, approve, revisions, files | when a draft waits | Plan; revisions and files behind disclosures as today |
| "Plan · path", Where it fits, Accept, Import | when there is a plan | Plan |
| seam-questions warning | when there are any | Plan, once, as "Waiting for a human" |
| The plan on the map: Changes, Sends, O-rows, Waiting, Decisions | when reading the plan | Plan; Decisions leave it (they are in Overview) |
| Room: voice, agents, messages, compose | when discussing | Room; its count in the section nav is unread messages |
| PR line, View/Hide, pull view, Merge | when a PR exists | Pull request; the section is absent until there is one |
| Review and approval signals | sometimes | Overview, as one line each |
| Decisions | sometimes | Overview, once |
| "spend $X" last line | no | remove: the gauge says it |

Sections and their primary actions: Overview (none of its own), Plan (**Approve**, **Accept plan**
or **Import**, whichever applies), Room (**Post**), Sessions (**Send**), Pull request (**Merge**).

Voice (Q5): `room/VoicePanel.tsx` has no cleanup on unmount; `leave` (`:107-111`) is the only
disconnect. With sections, a person could leave the Room while joined and keep an open microphone
with no control on screen. The connection moves above the sections, into a holder that lives as
long as the issue view: while joined, the header shows "In voice · speech via <host> · Leave" on
every section (ADR 0073 asks for the host while joined), and closing the issue view leaves.
Spoken replies are driven by the room's messages (`VoicePanel.tsx:133-144`), so the holder keeps
reading the room while joined, and replies play on every section. The consent text stays in the
Room section, before the first utterance.

States: loading (header from the overview at once; a section shows its own "Loading…"); empty
(Plan: "No plan yet. A session writes it."; Room: "No messages yet."; Sessions: "No session yet"
with the header's Start; Pull request absent); error (the section's read error with Try again);
agent working (the header's agent line with a live glow, the one moving thing on the screen);
not found (rule 7). Motion: the section indicator slides (M11); a new room message while on
another section draws attention once on the Room entry, then settles.

## 5. The issue list

**Who:** a person choosing what to work on. **Job:** "I want to find the issue I should look at,
so that I can open it or start it." **Done:** an issue is open.

Path: land on Open → scan rows → **open an issue**. Side paths: New issue (the head's button),
Not pinned → **Pin to…**, drag a row to the map or a lane.

| Element (`IssuePanel.tsx:279-350`, `Backlog.tsx:77-133`) | Needed now? | Decision |
| --- | --- | --- |
| "pinned to part X · show all" | when a part is selected | keep |
| Team track | yes, here | arrives from the top bar, one line above the segments |
| Open / Done switch | yes | keep; gains Not pinned (Q8) |
| row: ring, key, title, state | yes | keep |
| row: pins line | sometimes | keep, quiet |
| row: closed badge | sometimes | keep: it changes what you do |
| row: signal badges | sometimes | keep "does not fit" and "approval stale" (they call for action); "reviewed", "approved", "unchecked", "pinned in", "counts in" move to the issue's Overview |
| row: Pin to… and Queue on… | sometimes | on demand: one "⋯" row menu, shown on hover and focus; drag stays |
| "New issue…" | sometimes | keep at the head; opens `#/issues/new` |
| Not pinned: hint, filter, rows, suggestion with "Pin here", Pin to… | when triaging | its own segment; "Pin to…" stays visible on each row, it is that row's action; Queue goes in the row menu |
| per-repo listing freshness | when stale | on demand: shown only for a repo whose listing failed or is stale |
| unpinned-sessions count | rarely | move: it is a finding (`unattributed_session`) and shows in Findings |

States: five empty texts stay (`:330-341`), each naming the next step; loading is the shell's.

## 6. The plans board

**Who:** a person who approves plans, or wants to see which wait. **Job:** "I want to see which
plans wait for a human, so that I can read one and decide." **Done:** a plan is open in its
issue's Plan section.

| Element (`PlansBoard.tsx`) | Needed now? | Decision |
| --- | --- | --- |
| big counts: proposed, open, do not fit | no | remove: the column headings carry the counts; "do not fit" is a badge on its rows |
| Open / Done switch | yes | keep |
| columns Proposed and Open | yes | keep |
| row: mini map, key, title, part and seam chips | yes | keep |
| row: meta line | sometimes | keep one line; branch and ref on demand (title) |
| row: badges (merged/closed, draft, fits, reviewed, approval) | some | keep "does not fit", "approval stale" and "draft"; the rest are in the plan |
| row: Accept plan | no | remove: a plan is accepted where it is read |
| plan detail (`PlanDetail`) | — | removed: the row opens `#/issues/<o>/<r>/<n>/plan` |

`@plans.board-from-another-tab` says the open issue stays open under Issues. With one view at a
time, the board replaces the issue, and Back returns to it (Q2).

## 7. The map band, map versus territory, the inspector

**Who:** anyone reading the shared picture. **Job:** "I want to read the map one way at a time,
so that I can see where work and traffic are." **Done:** the lens they wanted is on, and the
canvas has the room.

| Element (`MapView.tsx:914-1090`) | Needed now? | Decision |
| --- | --- | --- |
| lens group | yes | keep; the primary control |
| By repo, Auto-arrange | sometimes | on demand: a View menu |
| Edit map | sometimes | keep, quiet, at the row's end; goes to `#/map/draft` |
| scenario player | in Scenarios | keep as one compact row; the steps list stays behind its disclosure |
| edit hint paragraph | in edit mode | move: the map draft view's first line |
| Traffic vitals, seven tiles | in Traffic | one line: calls/min, errors %, and "N to settle" linking to `#/map/territory`; the seven tiles head that view |
| legend paragraph per lens | first visits | on demand: a "Key" disclosure at the row's end, per lens, closed by default; the "traffic is unknown" sentence stays visible, because unknown must never read as quiet |
| Map versus territory list | when settling | move: the panel view `#/map/territory`, which asks the map for the Traffic lens |

Map versus territory view. **Job:** "I want to settle what traffic shows and the map does not."
Primary action per row: **Add as a part…/seam…** (opens the draft with the form filled);
Dismiss is secondary; Dismissed stays behind its disclosure with Undo.

Inspector (`map/Inspector.tsx:128-259`). **Job:** "I want to know what this part or seam is and
what touches it now." It becomes the view at `#/map/part/<id>`; the issue list no longer renders
under it. It keeps "Touching it now" first, and ends with **New issue here…** as its one action.
Flows, depends-on lists and O-rows stay, as the content the view is for.

Cards (Q1). A part card shows id, kind, one activity signal and at most one warning mark with a
count; a seam chip shows its id, the traffic text in the Traffic lens, and one warning mark. The
issue-state counts, plan counts, model counts and approval counts are in the inspector, one click
away. This removes information from the picture and is a judgment call; PR 11 is separate so it
can be left out.

## 8. The map draft

**Who:** a person changing the shared map. **Job:** "I want to change the map and propose it,
so that the team can merge it." **Done:** the pull request is open.

Path: Edit map → change on the canvas or pick a discrepancy → the change appears in the list →
**Propose map change…** → review the diff → **Open the pull request**.

| Element (`DraftPanel.tsx:242-503`) | Needed now? | Decision |
| --- | --- | --- |
| "Map draft · N changes", intro | yes | keep, with the edit hint as its one line of help |
| proposal state with the PR link | when proposed | keep |
| change list with Remove | yes | keep: this is the draft |
| Propose map change… | yes | the primary action |
| Withdraw, Propose again, Discard, Undo discard, Copy YAML | rarely | on demand: a More menu; Discard through `Confirm` |
| review box: diff, Open the pull request, Not yet | when proposing | keep as the step after Propose |
| "Change part X" / "Change seam X" form | when one is selected | keep; it replaces the read-only inspector for the selection |
| "Add part" and "Add seam" forms, always open | no | on demand: "Add part…" and "Add seam…" open the form; double-click and drag on the canvas still do |

## 9. Companions

**Who:** a person with their own machine paired, and teammates looking. **Job:** "I want to see
who runs what and line up my own work." **Done:** the queue is in the order I want, or a session
started.

| Element (`CompanionsPanel.tsx`, `Lanes.tsx`) | Needed now? | Decision |
| --- | --- | --- |
| "Pair a companion": three steps, code, Approve | once per machine | move: `#/companions/pair`; shown inline only when you have no companion; otherwise a quiet "Pair a companion" link at the foot (`@companions.pairing-steps` is reworded to say where the steps are) |
| lane header: LED, host, owner, state, freshness | yes | keep |
| lane header: headless and auth badges | sometimes | on demand: the host's title |
| revoke | rarely | on demand: a lane menu, through `Confirm` |
| sessions with agent lines | yes | keep; a row opens the issue's Sessions section |
| queue with ↑ ↓ Remove Start | yes, own lanes | **Start** on the queue's head is the primary action; ↑ ↓ Remove in a row menu; drag stays |

## 10. Repos, a repo, a pull request

**Repos. Job:** "I want to see which repos the map names and how each stands." Primary action:
**open a repo**. The rack keeps its name, lights and four bars. "Provides", the graph line, the
pull requests and the issues move to the repo view. The shared context-graph lines show only
when a source is stale. "Sessions not tied to an issue" stays at the foot: it is a list of things
to fix.

**A repo. Job:** "I want to see what this repo holds and what is open in it." Pull requests
first (a row opens the pull request view), then issues (a tracked row opens the issue; an
untracked one offers **Pin to…**), then what it provides and how fresh its graph source is.

**A pull request. Job:** "I want to review and merge this." One view, used from the repo and as
the issue's Pull request section (`pulls/PullView.tsx`). **Merge** is the primary action; the
method select stays beside it.

## 11. Configure workspace, Workspaces, Members, Operator

Full pages, map hidden, one column of readable width, a "← Back" that returns to the view the
person came from (rule 3).

| Page | Job | Primary action | Change |
| --- | --- | --- | --- |
| Configure workspace (`#/settings`) | "change how this workspace reads its map and traffic" | **Save** | create and delete leave the page |
| Workspaces (`#/settings/workspaces`) | "add or remove an application" | **Create workspace** | the list of workspaces, the new-workspace form on demand, delete through `Confirm` |
| Members (`#/members`) | "see who is in, invite, remove" | **Invite** (admins) | unchanged content, room to read the access paragraph |
| Operator (`#/operator`) | "create tenants and attach installations" | **Create tenant** | three sections stay; attach, detach and purge are row actions, purge through `Confirm` |

Application settings (the server's config) are not on any of these pages and are not editable at
runtime; the Operator page is about tenants, not the server.

## 12. Scenarios that are reworded

Ids never change. The scenarios whose wording places something that moves, by PR:

| PR | Scenarios |
| --- | --- |
| 1 | `@plans.board-from-another-tab` (Q2: Back returns to the open issue; the behaviour changes with routing, so it is reworded here) |
| 3 | `@settings.single-workspace`, `@settings.move-map-ref`, `@map.every-part` ("name and version are shown": in the workspace menu), `@tenants.switch`, `@tenants.one-tenant-no-switcher`, `@workspaces.*`, `@admin.*`, `@findings.*`, `@auth.log-out` (gains its e2e test) |
| 4 | `@issues.*`, `@pins.*`, `@queue.*`, `@new-issue.*`, `@map.inspect-part-and-seam` |
| 5 | `@map.escape-closes`, `@plans.paints-the-map`, the `@room.*`, `@chat.*` and `@pulls.view` scenarios that say "in the issue's detail" |
| 6 | `@plans.board`, `@plans.accept-says-so`, `@plans.unfit-cannot-be-accepted` (Accept is in the plan, not on the row), `@plans.done-view` |
| 7 | `@repos.*`, `@pulls.listed-per-repo` |
| 8 | `@companions.pairing-steps`, `@companions.see-teammates` |
| 9 | `@map.ghosts-and-discrepancies`, `@map.dismiss-discrepancy`, `@map.empty-report-says-so`, `@map.traffic-first` |
| 10 | `@map.edit-hint`, `@drafts.*` |

`pnpm scenarios check --base origin/main` (new in PR 1) fails when an id that exists at the
base is gone, with a fixture that makes it fail on purpose.

## 13. Checklist per PR

Each PR description answers the skill's checklist for the screens it changes: the job in one
sentence; exactly one primary action; every element passed the audit; the scenario's path works
with no dead end; empty, loading, error and agent-working states; at most one thing moving;
reduced motion keeps the meaning; tokens and components reused; app and workspace settings not
mixed; keyboard and focus follow the path; the scenario's e2e test updated.

## 14. Questions for Mattias

Each is a judgment call inside `web`. The stack proceeds on the first option until answered.

1. **What stays on a part card and a seam chip?**
   a. Sessions working, and one warning mark with a count; the rest in the inspector.
   b. Issue-state counts and one warning mark.
   c. Leave the cards as they are and drop PR 11.
2. **`@plans.board-from-another-tab` says the issue stays open under Issues.**
   a. Reword: the board replaces the issue, and Back returns to it.
   b. The nav remembers each entry's last view (hidden state comes back).
3. **The map name and sha move into the workspace menu; `@settings.move-map-ref` says the header
   shows them.**
   a. Reword to "the workspace menu shows the map at that branch's commit".
   b. Keep the sha visible in the bar.
4. **Counts in the nav.**
   a. Only proposed plans and live companions.
   b. All four, as today.
5. **Voice when you leave the Room section while joined.**
   a. Stay joined across the issue view, with "In voice · Leave" in the header; leave when the
      issue closes.
   b. Leave on the section switch, with a notice.
6. **The plan lens on the map.**
   a. On every section of an issue that has a plan, as today.
   b. Only on Overview and Plan.
7. **"refreshed N ago".**
   a. Visible only when stale or disconnected; otherwise the dot's title.
   b. Always visible.
8. **Not pinned.**
   a. Its own segment of the issue list.
   b. Under the Open list, collapsed, so an unpinned issue sits beside the tracked ones while
      you drag it to the map.
