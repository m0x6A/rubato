# 0061. Skills and plan-check emit spans through one dependency-free script; the companion mirrors the OTLP env for sessions

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-14 by approval comment https://github.com/m0x6A/rubato/issues/140#issuecomment-6882263766 (self); drafted by Mattias Jonsson (#142 Q1–Q3, answered in planning #140, 2026-11-14); drafted by agent (Claude Code, claude-opus-5-5, issue #140) — proposed
- **Seam:** companion-claude-code, skill-github, skill-plan-file, skill-system-map, plan-check-plan-file
- **Issue:** m0x6A/rubato#142

## Context

Rubato assumes every running part emits telemetry (#140). The rubato skill's scripts and the
plan-check action emit none, so they are missing from the service graph.

#142 asked for one simple, dependency-free script. It assumed a launched session's commands
inherit `OTEL_RESOURCE_ATTRIBUTES`. They don't: Claude Code documents that it "doesn't pass
`OTEL_*` environment variables to the subprocesses it spawns, including the Bash tool"
(code.claude.com/docs/en/monitoring-usage). It does set `TRACEPARENT` for Bash subprocesses when
its tracing is on.

Design: `docs/plans/140-observed-system/design.md` §2.

## Decision

**The script.** `skills/rubato/scripts/otel-emit.ts` uses only Node built-ins. It offers a
`span()` library and a `span -- <cmd>` CLI, and sends OTLP/JSON over HTTP to the standard OTLP
endpoint: one POST per process, an 800 ms timeout, silent on any failure, stdout and the exit
code untouched. The rubato skill's scripts and plan-check use it. Calls to other parts are client
spans with `peer.service` set to the map id of the other side.

**Settings.** The script reads the standard `OTEL_EXPORTER_OTLP_*` and `OTEL_RESOURCE_ATTRIBUTES`
first.

**Inside a session.** Because Claude Code strips `OTEL_*`, the companion's launch script:

- merges `OTEL_RESOURCE_ATTRIBUTES` instead of overwriting it;
- exports the same values a second time as `RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT`, `_PROTOCOL`,
  `_HEADERS` and `RUBATO_OTEL_RESOURCE_ATTRIBUTES`;
- expands the headers at run time and never writes them to disk.

The script reads these mirrors only when the standard name is unset.

**Identity.** `service.name` is the skill's name. `service.namespace` comes only from the
resource attributes; Rubato never sets one.

**Tracing.** A valid `TRACEPARENT` is used as the parent.

**Out of scope.** gRPC endpoints are skipped, and spans derived from hooks are left to a later
issue.

## Alternatives considered

- **Endpoint and namespace from `~/.config/rubato/local.yml`**: a Rubato-only config of OTel
  settings, and the session's `rubato.*` attributes would reach a span only through the trace,
  when Claude Code's tracing beta is on.
- **Standard `OTEL_*` only**: skills would be dark inside every launched session, which is the
  main case.
- **The OpenTelemetry JS SDK**: a new dependency tree for one span per process.

## Consequences

- **The mirror.** One Rubato-named variable family exists only because of Claude Code's
  stripping. It means exactly its `OTEL_*` twin and loses to it when both are set.
- **What lights up.** Skills appear in the service graph wherever the developer's environment
  has an endpoint and a namespace.
- **O-rows** of plan #140:
  - `companion-claude-code`: no `rubato.skill.command` spans for a launched `rubato.issue`;
  - the skill seams: `rubato.skill.call` with `peer.service`.

## Supersedes

None.
