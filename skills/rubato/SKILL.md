---
name: rubato
description: Plan work on a GitHub issue from the team's shared system map. Use when starting on an issue in a repo connected to Rubato, when asked to "plan this" or "where does this fit", or when a session was launched by Rubato (the prompt says so). Loads the map and accepted ADRs, writes the plan at the agreed path in the agreed form, and records decisions as Decision: comments.
---

# rubato: plan from the map

Each developer orchestrates their own agents. Rubato keeps the whole picture. Your part is to
plan from the shared map, name only ids that exist, answer the 3 am question for every seam
you touch, and leave the plan where the team and the plan check can read it.

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
restate from chat. Comments whose first line starts with `Decision:` are decisions already made.

## 3. Write the plan

```
node --experimental-strip-types scripts/plan.ts init --issue <owner/name#n> --slug <slug> --title "<title>"
```

This writes the skeleton at the agreed path (default `docs/plans/<n>.md`; the map may override
per repo) with the frontmatter and the sections the check reads. Fill it:

- **Affected parts**: one row per map node, status `affected`, `new` or `quiet`, one line on
  what changes, and the seam ids it touches.
- **Data flow**: Mermaid, the data crossing each seam named above.
- **Tasks**: ordered, each naming its failing test first.
- **Assumed, not yet approved**: every decision nothing settled. Scope `node` means assume and
  proceed, labelled. Scope `seam` blocks until a human answers; raise it as a numbered question
  with 2–4 options, not prose.
- **O-rows**: one per seam in Affected parts. If this breaks at 3 am, which span, metric or
  log line leads back to it, on which dashboard panel, and which side of the seam is woken.
  Name `rubato.*` attributes. "We'll see it in the logs" is not an answer.
- **Review map**: where a reviewer looks first, at most three places.

Then check it before committing:

```
node --experimental-strip-types scripts/plan.ts check docs/plans/<n>.md
```

It prints one `rubato.plan.check` line. Fix every unknown id and every missing O-row. Commit
the plan on the `issue/<n>-<slug>` branch before any code; the plan travels as a file, not
through an API.

## 4. Record decisions

A decision made during the work is an issue comment, from `decision-template.md`:

```
gh issue comment <n> --body-file <file>
```

First line `Decision: <one-line title>`, then `## Context`, `## Options`, `## Chosen`,
`## Why`. Rubato shows it on the issue's node. A decision that touches a seam is an ADR in the
map repo instead; draft it as *proposed* and let a human accept it.

## 5. Caps

The launch carried a spend cap and an attempt limit (frontmatter `spend_cap_usd`,
`attempt_limit`). When the companion stops the session for either, the issue is *Blocked*;
write what you learned to the issue before you run out.

## Don't

- Don't invent node ids; read the map.
- Don't write the plan anywhere but the agreed path.
- Don't steer another developer's agent; coordinate through the issue.
- Don't install the hooks yourself; the companion injects `hooks.json` at launch. To opt a
  manual session in: `node --experimental-strip-types scripts/hooks.ts --port <p> --token <t>
  --out .claude/settings.local.json`.
