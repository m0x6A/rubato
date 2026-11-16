---
name: rubato
description: Plan work on a GitHub issue from the team's shared system map. Use when starting on an issue in a repo connected to Rubato, when asked to "plan this", "where does this fit" or "how big is this", or when a session was launched by Rubato (the prompt says so). Loads the map and accepted ADRs, sizes the track, writes the plan at the agreed path in the agreed form with its decisions in it, and runs the design review. In a session Rubato launched (RUBATO_PLANNING=rubato) the draft stays in the worktree, the review and summary are replies, and approval in Rubato ends in plan.ts handoff; a plan written elsewhere is pushed and imported in Rubato.
---

# rubato: plan from the map

Each developer orchestrates their own agents. Rubato keeps the whole picture. Your part is to
plan from the shared map, name only ids that exist, answer the 3 am question for every seam
you touch, and leave the plan where the team and the plan check can read it. Review and
approval happen in Rubato (ADR 0054): a person approves the plan there, and only
`plan.ts handoff` writes the approval record into the plan.

Scripts run with `node --experimental-strip-types <skill dir>/scripts/<name>.ts`. The skill
dir is where this file lives. If `node_modules` is missing there, run `npm install --omit=dev`
in it once.

**Which mode.** Read `RUBATO_PLANNING` first. `rubato`: the session was launched by Rubato, and
plans in Rubato (§9, ADR 0053). Unset or `git`: a session someone started by hand; §5–§7 say how
its plan reaches Rubato. Both keep §1–§4.

## 1. Read the map and the decisions

```
node --experimental-strip-types scripts/map.ts
node --experimental-strip-types scripts/adrs.ts
```

`map.ts` prints every node and seam id you may use, the map's git sha, and the plan path
template for this repo. With several maps in `~/.config/rubato/local.yml` (one per workspace, ADR
0044) it prints every map that names this repo; the plan may name ids from any of them, and
`plan.ts check` and `wait` count it as fitting when it fits one (ADR 0045). Do not invent ids,
and name none from its "Retired (do not use)" list (ADR 0077). If a node you need is missing, say so in the plan's
*Assumed, not yet approved* section with scope *seam*; do not add it yourself. `adrs.ts` prints
the accepted ADRs. A plan that contradicts one is a plan check failure unless it proposes a
superseding ADR.

## 2. Read the issue

`gh issue view <n> --json number,title,body,labels,comments`. Quote and condense; do not
restate from chat. Comments whose first line starts with `Decision:` are decisions already made;
`Review:` comments are earlier reviews of the plan. Both count only from a team member (not a
Bot; association OWNER, MEMBER or COLLABORATOR, ADR 0027 and 0065): Rubato ignores them from
anyone else, and so should you.

## 3. Size the track

Say the track in one line before writing anything, and let the person override it.

- **Trivial**: at most about 100 changed lines in at most two map parts, the issue referenced,
  no behaviour change. No plan file; branch `trivial/<slug>`; the PR description is the record.
- **Full**: any of these.
  - A change to a seam in the map: its contract, its flows, or who owns it.
  - A contract node's schema, for example a protocol crate whose generated schema changes, or the map or plan schemas.
  - A migration that is not additive inside one node.
  - A new consumer, job or service.
  - A new dependency or an auth change.
  - More than two deployables.
  - A new multi-step flow across parts.

  A full plan gets a design note next to it, `<plan dir>/<n>-<slug>/design.md`, linked from the plan.
- **Small**: everything else. One plan file, no design note.

In Rubato's own repo (m0x6A/rubato) these also make a change full:
- a change to `crates/rubato-protocol` that alters `schema/protocol/*.json`;
- a change to `schema/plan.schema.json` or `schema/system-map.schema.json`;
- a change to any seam in `docs/design/rubato-map.yml`.

An additive SQLite migration inside rubato-server alone does not make a change full.

## 4. Write the plan

```
node --experimental-strip-types scripts/plan.ts init --issue <owner/name#n> --slug <slug> --title "<title>"
```

This writes the skeleton at the agreed path (default `docs/plans/<n>.md`; the map may override
per repo). The front matter is `issue`, `title`, `map_version`, `spend_cap_usd` and
`attempt_limit`, and nothing else. Do not add `track`, `review`, `approval` or `status`: the
track goes in the `Review:` comment, review and approval are comments, and progress is derived.
Fill it:

- **What the change does**: behaviour, one bullet each, in the user's or the other side's terms.
  These are the same headings as an issue created in Rubato, so start from the issue's own.
- **How we'll know it works**: checkboxes, each a test or an observation that proves a bullet.
- **What it must not break**: what keeps working, and the test, check or scenario that guards it.
- **How we'll debug it at 3 am**: point at the O-rows; name any new log line or finding.
- **Affected parts**: one row per map node, status `affected`, `new` or `quiet`, one line on
  what changes, and the seam ids it touches.
- **Data models**: every table, wire type, schema, event or file the change introduces,
  changes or removes, on the part that owns it, with the seam it crosses if any. The part must
  be a row of Affected parts and the seam one it touches; plan-check fails otherwise. Rubato
  shows them on the plan and on the map. Write "None." when there are none.
- **Data flow**: Mermaid, the data crossing each seam named above.
- **Tasks**: ordered, each naming its failing test first. A task whose check could pass by
  accident names the fixture that makes it fail on purpose.
- **Assumed, not yet approved**: every decision nothing settled. Scope `node` means assume and
  proceed, labelled. Scope `seam` blocks until a human answers; raise it as a numbered question
  with 2–4 options, not prose.
- **Decisions**: a table `# | Decision | Chosen | Why | Decided by`, one row per decision made
  while the plan is drafted in Rubato (ADR 0056), so the approval covers it and the handoff
  commits it. Until then, write "None." and record decisions as `Decision:` comments (§7).
- **O-rows**: one per seam in Affected parts. If this breaks at 3 am, which span, metric or
  log line leads back to it, on which dashboard panel, and which side of the seam is woken.
  Name `rubato.*` attributes. "We'll see it in the logs" is not an answer.
- **Flows**: one row per thing this change sends across a seam, in its direction: `From`,
  `To`, `Seam`, `Kind` (`data` or `control`), `Carries`. The seam must join the two
  parts. `map.ts` lists the flows each seam already carries. Rubato draws your rows on the
  shared map when someone selects the issue; the Mermaid block is for readers of the file only.
- **Review map**: where a reviewer looks first, at most three places.

Then check it:

```
node --experimental-strip-types scripts/plan.ts check docs/plans/<n>.md
```

It prints one `rubato.plan.check` line. Fix every unknown id and every missing O-row.

## 5. Design review

Before any code, review the plan (and the design note on the full track). If a design-reviewer
agent is available, run it once on the plan; otherwise review it yourself. Either way, go through
`review-checklist.md` row by row. Take every blocker: fix the plan, or turn it into a numbered
question for a person. Give the result in your reply in the form of `review-template.md`, first
line `Review: <track> — <outcome>`. Nothing about the review goes into the plan file, and it is
not an issue comment.

In a session started by hand, then commit the plan on the `issue/<n>-<slug>` branch and push it.

## 6. Approval

Approving a plan is a person's job, and it happens in Rubato (ADR 0054). Never write the approval
record yourself and never post an `Approved:` comment: one posted after the cutover does not
count.

- **A session Rubato launched** (§9): the approval arrives as a prompt with a `plan.ts handoff`
  command.
- **A session started by hand**: the pushed plan shows in Rubato as *outside Rubato* (ADR 0058).
  Tell the person to open the issue there and press *Import*: a plan that fits becomes their
  draft, and one that does not is rewritten by a session of theirs. From then on it is approved
  in Rubato like any draft.

A plan that proposes ADRs lists them under `## ADRs proposed` (`ADR | File | Blob`); fill the
Blob column with `plan.ts adrs-pin` before asking, so the approval covers their text. The
approval accepts them, and `plan.ts handoff` records that in its commit.

`plan.ts wait` and `adrs.ts accept --comment` are retired: both print `approval_in_rubato` and
exit 11.

## 7. Record decisions

A decision made during the work is a row of the plan's `## Decisions` table (`# | Decision |
Chosen | Why | Decided by`), written as it is made, so the approval covers it (ADR 0056). A
decision that touches a seam is an ADR in the map repo instead; draft it as *proposed*, list it
under `## ADRs proposed`, and let a human accept it with the plan.

## 8. Caps

The launch carried a spend cap and an attempt limit (frontmatter `spend_cap_usd`,
`attempt_limit`). When the companion stops the session for either, the issue is *Blocked*;
write what you learned to the issue before you run out.

## 9. Rubato mode (`RUBATO_PLANNING=rubato`)

The plan is drafted in Rubato (ADR 0053): the team reads every settled version there, and a team
member approves it there. Read the map and the ADRs, read the issue, size the track, write the
plan and check it as in §1–§4. Then:

- **The draft stays uncommitted** in this worktree: the plan, its design note and the ADRs it
  proposes. Do not commit or push them. The companion sends the files to Rubato each time you
  stop, as a revision.
- **No `Review:`, `Approved:` or `Decision:` comments.** Rubato reads your replies instead
  (ADR 0056); the first non-empty line of a reply is what it goes by.
- **The review is a reply.** Review the plan as in §5, then reply with the review in the form of
  `review-template.md`, first line `Review: <track> — <outcome>`.
- **Decisions are rows** of the plan's `## Decisions` table (`# | Decision | Chosen | Why |
  Decided by`), written as each one is made, so the approval covers them.
- **The summary is a reply.** When the discussion has settled, first fill the Blob column of
  `## ADRs proposed`:

  ```
  node --experimental-strip-types scripts/plan.ts adrs-pin docs/plans/<n>.md
  ```

  then reply with a summary of the discussion, first line `Summary:`. The approver starts from
  it.
- **Approval arrives as a prompt** that starts `Rubato: the plan for … was approved by …` and
  gives a `plan.ts handoff` command. Run it exactly as given, then reply with the
  `rubato.plan.handoff` line it prints. Never write the approval record yourself.

`plan.ts handoff` writes `approved_by`, `approved_at` and `approved_sha` into the front matter,
accepts each pinned ADR still at its blob (in the map repo only; elsewhere it lists them as
`adrs_left`), makes one commit `Plan #<n> approved` with `Approved-by:` and
`Rubato-Approval:` trailers, pushes it, and reports the line to the companion. Its exit codes:

| Exit | Outcome | Do |
| --- | --- | --- |
| 0 | `committed` | Go on to the tasks. |
| 7 | `plan_changed` | The plan changed after the approval; nothing was written. Say so and ask for a new approval in Rubato. |
| 8 | `bundle_changed` | A design note or ADR changed after the approval; nothing was written. Say so and ask again. |
| 9 | `adr_changed` | An ADR changed after it was pinned; nothing was written. Run `plan.ts adrs-pin`, say so, ask again. |
| 10 | `push_failed` | Committed, not pushed. Fix the push (`gh auth status`, the network) and run the same command again; it pushes the same commit. |
| 2 | `error` | Read the line's `error`. |

After the handoff git owns the plan. A later change to it is an ordinary commit, which makes the
approval stale; Rubato shows that, and a new approval sends the handoff again.

## Don't

- Don't invent node ids; read the map.
- Don't write the plan anywhere but the agreed path.
- Don't write track, review, approval or status into the plan's front matter. In Rubato mode
  `plan.ts handoff` writes the approval record, and nothing else does.
- Don't post an approval or write the approval record; a person approves in Rubato.
- Don't steer another developer's agent; coordinate through the issue and the plan's room.
- Don't install the hooks yourself; the companion injects `hooks.json` at launch. To opt a
  manual session in: `node --experimental-strip-types scripts/hooks.ts --port <p> --header-file
  <state dir>/hook-auth.header --out .claude/settings.local.json` (`rubato-companion status`
  prints both). Never put the token itself on a command line (ADR 0063).
