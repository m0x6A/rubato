# skills/rubato

The Claude Code skill installed into every connected repo. This README is for people working
on Rubato; `scripts/install.ts` does not copy it, so connected repos never see it.

## Slices

Already one file per capability (ADR 0015, design D10); nothing here moved when the
deployables were sliced, because `install.ts`, `vendor.ts`, `e2e/run.ts` and CI name these
paths.

| Capability | File | What it does |
| --- | --- | --- |
| map | `scripts/map.ts` | prints the system map: every node and seam id a plan may use |
| plan | `scripts/plan.ts`, `scripts/lib/plan-sha.ts` | creates and checks the plan file at the agreed path (ADR 0005, 0009, 0027); prints the sha an approval names; pins proposed ADRs by blob (`adrs-pin`) |
| review | `review-template.md`, `review-checklist.md` | the design-review step and its `Review:` comment (ADR 0027, 0028) |
| approval | `scripts/plan.ts wait`, `scripts/adrs.ts accept`, `scripts/lib/comments.ts`, `scripts/lib/approval.ts` | waits for a person's approval and records the acceptance of the ADRs the plan pins (ADR 0035, seam `skill-github`) |
| adrs | `scripts/adrs.ts` | lists accepted ADRs so a plan does not contradict one |
| hooks | `scripts/hooks.ts`, `hooks.json` | renders the hooks file for a companion endpoint (ADR 0013) |
| install | `scripts/install.ts` | copies the skill, the vendored plan check and the schemas into a repo |

`scripts/lib/` is the shared kernel (git, workspace config, vendoring). The files the
skill ships as is (`SKILL.md`, `prompt.md`, `decision-template.md`, `review-template.md`,
`review-checklist.md`, `workspace.example.yml`)
must stay byte-identical across refactors; connected repos receive them on install.
