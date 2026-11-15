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
| approval | `scripts/plan.ts wait`, `scripts/adrs.ts accept`, `scripts/lib/comments.ts`, `scripts/lib/approval.ts` | retired at the cutover (ADR 0054): both commands print `approval_in_rubato` and exit 11, since plans are approved in Rubato and `plan.ts handoff` records the ADRs accepted. The libraries stay for the handoff and for plans approved by comment before the cutover |
| handoff | `scripts/plan.ts handoff`, `scripts/lib/handoff.ts`, `prompt-rubato.md` | in Rubato mode (`RUBATO_PLANNING=rubato`), turns an approval made in Rubato into one commit: the approval record, the pinned ADRs accepted in the map repo, the push, and a `rubato.plan.handoff` line posted to the companion (ADR 0053, 0055) |
| adrs | `scripts/adrs.ts` | lists accepted ADRs so a plan does not contradict one |
| hooks | `scripts/hooks.ts`, `hooks.json` | renders the hooks file for a companion endpoint; the token stays in the companion's 0600 header file (ADR 0013, 0063) |
| install | `scripts/install.ts` | copies the skill, the vendored plan check and the schemas into a repo |
| telemetry | `scripts/otel-emit.ts` | one span per command and a client span per call to another part, over OTLP/JSON with no dependencies (ADR 0061) |

`scripts/lib/` is the shared kernel (git, the local config `local.ts`, vendoring). The files the
skill ships as is (`SKILL.md`, `prompt.md`, `prompt-rubato.md`, `decision-template.md`, `review-template.md`,
`review-checklist.md`, `local.example.yml`)
must stay byte-identical across refactors; connected repos receive them on install.

## Telemetry from any skill

`scripts/otel-emit.ts` sends spans using only Node built-ins (ADR 0061).

### Settings

It reads the standard OpenTelemetry env. Inside a Claude Code session, which strips `OTEL_*` from
the commands it runs, it reads the companion's `RUBATO_OTEL_*` mirror instead:

| Setting | Read from |
| --- | --- |
| endpoint | `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, else `OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/traces`, else the `RUBATO_` mirrors |
| headers | `OTEL_EXPORTER_OTLP_HEADERS` (`k=v,…`), else `RUBATO_OTEL_EXPORTER_OTLP_HEADERS` |
| `service.namespace` and other resource attributes | `OTEL_RESOURCE_ATTRIBUTES`, else `RUBATO_OTEL_RESOURCE_ATTRIBUTES` |
| parent span | `TRACEPARENT`, which Claude Code sets for Bash when its tracing is on |

`service.name` is the skill's name.

### When it sends

- **Nothing is sent** when there is no endpoint, the protocol is `grpc`, or
  `OTEL_SDK_DISABLED=true`.
- **Otherwise one POST** per process, with an 800 ms timeout.
- **Failures are silent.** Stdout, stderr and the exit code never change.

### Using it

A TypeScript skill wraps its entry point and its calls to other parts:

```ts
import { call, runCommand } from "./otel-emit.ts";

process.exitCode = await runCommand("my-skill", "sync", async () => {
  call("github", () => gh(["issue", "list"])); // client span, peer.service=github
  return 0;
});
```

A skill that is only Markdown and shell wraps a command. The command's output and exit code are
kept:

```sh
node --experimental-strip-types <skill dir>/scripts/otel-emit.ts span --skill my-skill --peer github -- gh issue list
```

### Naming

- **The other side** is named in `--peer` or `call()` by its map id. Declare that part's
  `service` in the map (ADR 0059) so the service graph's edge lands on it.
- **Span and attribute names** come from `crates/rubato-protocol/src/otel.rs`.
  `test/otel-names.test.ts` fails on any other `rubato.*` name in the scripts.
