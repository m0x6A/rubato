# 0080. Rubato's own parts emit `service.namespace` `rubato` unless their environment sets another

- **Status:** accepted
- **Date:** 2026-11-18
- **Decided by:** Mattias Jonsson (m0x6A), accepted in the Claude Code session 2026-11-24 ("Yes"), on his answer of 2026-11-18 ("Rubato sets a default", value `rubato`); drafted by agent (session on m0x6A/rubato#290)
- **Seam:** companion-claude-code (the launch's `OTEL_RESOURCE_ATTRIBUTES`); the other emitters cross no seam of the map
- **Issue:** m0x6A/rubato#290

## Context

A part is matched to the service it says it is by `service.namespace` and `service.name` (ADR
0059). Until now every part of Rubato got its namespace only from its environment (ADR 0061,
QUICKSTART): the server's deploy, each developer's companion, and through the companion the
sessions it launches and the skills in them. The deploy of 2026-11-18 sets none. On a stack
shared with other applications Rubato's telemetry then has no namespace, cannot be matched to
Rubato's own map, and cannot be selected by a workspace that lists the namespaces it reads
(#288). One namespace per process, set by hand on every machine, is setup nobody did.

## Decision

Every part of Rubato emits `service.namespace=rubato` unless its environment sets another. The
value is `DEFAULT_SERVICE_NAMESPACE` in `crates/rubato-protocol/src/otel.rs`.

- **`rubato-server` and `rubato-companion`** add it to their resource when
  `OTEL_RESOURCE_ATTRIBUTES` (as the OpenTelemetry SDK reads it) has no `service.namespace`.
- **A session a companion launches** gets `service.namespace=rubato` at the front of its
  `OTEL_RESOURCE_ATTRIBUTES` when the developer's environment has none, before the `rubato.*`
  attributes are merged in. Claude Code's own telemetry and the mirror `RUBATO_OTEL_RESOURCE_ATTRIBUTES`
  then carry it.
- **The skill scripts and the plan check** (`otel-emit.ts`) use it when the resource attributes
  they read have none. A skill run by hand in a connected repo is in `rubato` too: the skill is a
  part of Rubato, not of the repo it plans.
- **The environment always wins.** `OTEL_RESOURCE_ATTRIBUTES=service.namespace=<other>` is kept
  everywhere, also when it is set to nothing.

What does not change: Rubato never sets, derives or rewrites the namespace of a service it
**observes**. Matching an end of a call to a part stays as ADR 0059 has it, and a map names the
namespace its services emit.

## Alternatives considered

- **From the environment only, with a warning when it is missing**: no ADR change, but the
  setup stays on every machine, and a missed one is silent on the map.
- **One setting on the server, sent to companions in Welcome**: one place to change it, but it
  changes what crosses `companion-server`, and a team with one Rubato has no need for another value.
- **Derive it from the workspace or the repo**: a session would then be in the namespace of the
  application it works on, and Rubato's own parts could not be told from that application's.

## Consequences

- With the deploy and the companions untouched, Rubato's telemetry is in `rubato`. Series that
  already exist without a namespace stay as they are; new ones carry it, so a dashboard or alert
  that groups by `service_namespace` sees a new value.
- Rubato's own map can declare `service: { namespace: rubato, name: … }` on its parts, and a
  workspace can list `rubato` among the namespaces it reads (#288). Both are separate changes (#287).
- A team that runs two Rubato instances on one stack must set the namespace in the environment
  of one of them.
- O-row: `companion-claude-code`, a launched session's spans and metrics without
  `service.namespace`, seen on the session telemetry check of `e2e/run.ts` and by
  `rubato.skill.command` spans lacking the attribute.

## Supersedes

The sentence "`service.namespace` comes only from the resource attributes; Rubato never sets
one" of ADR 0061 (its other decisions stand). It narrows ADR 0059's "Rubato never sets, derives
or rewrites the namespace" to the services Rubato observes; ADR 0059's matching rule stands.
