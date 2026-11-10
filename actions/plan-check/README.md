# plan-check

Fails a PR whose plan names a node or seam the map does not declare, or touches a seam without
an O-row. One JSON log line per plan (`event: rubato.plan.check`) and a summary line with counts.

```
node --experimental-strip-types actions/plan-check/src/cli.ts \
  --map docs/design/rubato-map.yml --plans "docs/plans/*.md"
```

Exit 0 all plans pass, 1 a plan failed, 2 bad usage or an invalid map. Fixtures that must fail
live in `schema/fixtures/plans/`. The Rust server parses the same plan files; `valid.json` is
the shared expected output.

## Slices

plan-check is one capability, and its files are the stages of one pipeline (ADR 0015, design
D10): `map.ts` loads and validates the map, `parse-plan.ts` reads a plan, `schema.ts` validates
it, `check.ts` compares the plan with the map and counts O-rows, `cli.ts` reports. The skill
vendors `src/` as is, so these paths do not move.
