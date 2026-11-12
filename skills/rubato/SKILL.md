---
name: rubato
description: Plan work on a GitHub issue from the team's shared system map. Use when starting on an issue in a repo connected to Rubato, when asked to "plan this", "where does this fit" or "how big is this", or when a session was launched by Rubato (the prompt says so). Loads the map and accepted ADRs, sizes the track, writes the plan at the agreed path in the agreed form, runs the design review and posts it as a Review: comment, asks a person for approval, and records decisions as Decision: comments.
---

# rubato: plan from the map

Each developer orchestrates their own agents. Rubato keeps the whole picture. Your part is to
plan from the shared map, name only ids that exist, answer the 3 am question for every seam
you touch, and leave the plan where the team and the plan check can read it. Review and
approval are issue comments that Rubato reads and shows; they are never written into the plan.

Scripts run with `node --experimental-strip-types <skill dir>/scripts/<name>.ts`. The skill
dir is where this file lives. If `node_modules` is missing there, run `npm install --omit=dev`
in it once.

## 1. Read the map and the decisions

```
node --experimental-strip-types scripts/map.ts
node --experimental-strip-types scripts/adrs.ts
```

`map.ts` prints every node and seam id you may use, the map's git sha, and the plan path
template for this repo. Do not invent ids. If a node you need is missing, say so in the plan's
*Assumed, not yet approved* section with scope *seam*; do not add it yourself. `adrs.ts` prints
the accepted ADRs. A plan that contradicts one is a plan check failure unless it proposes a
superseding ADR.

## 2. Read the issue

`gh issue view <n> --json number,title,body,labels,comments`. Quote and condense; do not
restate from chat. Comments whose first line starts with `Decision:` are decisions already made;
`Review:` comments are earlier reviews of the plan.

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
question for a person. Then commit the plan on the `issue/<n>-<slug>` branch and push it.

Post the result on the issue, from `review-template.md`. Its first line is
`Review: <track> — <outcome>`, and it names the plan's sha:

```
node --experimental-strip-types scripts/plan.ts sha docs/plans/<n>.md
gh issue comment <n> --body-file <file>
```

Rubato shows the review on the issue. Nothing about the review goes into the plan file.

## 6. Approval

Approving a plan is a person's job. Ask for it in your reply and on the issue. To approve, the
person comments a line that reads `Approved: plan <sha>`, or `Approved: plan <sha> on <branch>`
when the issue has several branches, with the sha that `plan.ts sha` prints.

That sha ignores task checkboxes, so ticking tasks keeps an approval current; any other edit
needs a new one, and Rubato marks the old approval as for an earlier version. Never post an
approval yourself, even with the person's `gh`. Rubato counts it only from a team member, and
writing it for them defeats the record. A plan with a `seam` question open waits for the answer
and the approval. A `node` assumption does not wait.

**Wait for it yourself** (ADR 0035). Right after asking, push the plan and start the wait in the
background (in Claude Code: a background command, or Monitor on its output), and keep doing what
does not need the approval:

```
node --experimental-strip-types scripts/plan.ts wait docs/plans/<n>.md
```

It prints one `rubato.plan.wait` line per event (`started`, then `review`, `decision`,
`approved_earlier`, `untrusted`, `edited`, `poll_failed`) and ends with `approved` (exit 0) or
`exited` with a reason: `not_pushed` or `local_edit` (push the plan, exit 4), `does_not_fit`
(fix it, exit 5), `gh_failed` (check `gh auth status`, exit 6), `plan_changed` (the plan changed
on the branch; ask for a new approval, exit 7). It counts only what Rubato counts: the newest
approval from a team member, never edited, naming the sha of the plan as pushed, with the plan
fitting the map.

**The approval accepts the plan's ADRs.** A plan that proposes ADRs lists them under
`## ADRs proposed` (`ADR | File | Blob`); fill the Blob column with `plan.ts adrs-pin` before
asking, so the approval covers their text. On `approved`, in the map repo's checkout on the plan
branch:

```
node --experimental-strip-types scripts/adrs.ts accept --comment <the approval's url>
```

It reads the comment again from GitHub, checks everything again, and moves each pinned ADR still
at its blob to `accepted`, naming the approver, the date, the comment and `(self)` when the
approver runs this session. Commit that change on the plan branch with the comment's link and
push. An ADR changed after it was pinned is refused (`adr_changed`): update the plan, pin again,
ask again. Outside the map repo it writes nothing and prints what the approval would accept.

## 7. Record decisions

A decision made during the work is an issue comment, from `decision-template.md`:

```
gh issue comment <n> --body-file <file>
```

First line `Decision: <one-line title>`, then `## Context`, `## Options`, `## Chosen`,
`## Why`. Rubato shows it on the issue's node. A decision that touches a seam is an ADR in the
map repo instead; draft it as *proposed* and let a human accept it.

## 8. Caps

The launch carried a spend cap and an attempt limit (frontmatter `spend_cap_usd`,
`attempt_limit`). When the companion stops the session for either, the issue is *Blocked*;
write what you learned to the issue before you run out.

## Don't

- Don't invent node ids; read the map.
- Don't write the plan anywhere but the agreed path.
- Don't write track, review, approval or status into the plan's front matter.
- Don't post an approval; ask a person for it.
- Don't steer another developer's agent; coordinate through the issue.
- Don't install the hooks yourself; the companion injects `hooks.json` at launch. To opt a
  manual session in: `node --experimental-strip-types scripts/hooks.ts --port <p> --token <t>
  --out .claude/settings.local.json`.
