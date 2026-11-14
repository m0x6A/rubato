# 0059. A map part names the service it is, by the namespace and name that service emits

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-14 by approval comment https://github.com/m0x6A/rubato/issues/140#issuecomment-6882263766 (self); drafted by Mattias Jonsson (#140 Settled and Q2, 2026-11-14; namespaceless ends answered in planning, 2026-11-14); drafted by agent (Claude Code, claude-opus-5-5, issue #140) — proposed
- **Seam:** server-system-map, skill-system-map, web-server
- **Issue:** m0x6A/rubato#140

## Context

ADR 0039 says that `client` and `server` in a `TrafficReport` are OpenTelemetry `service.name`
values. Plan #104 matched a service to the part whose id equals that name
(`crates/rubato-protocol/src/traffic.rs:212`).

#140 settles that a part is identified by `service.namespace` + `service.name`, exactly as the
service emits them. Rubato never sets, derives or rewrites the namespace, and there is no
name-only matching. Two services with the same name in different namespaces are two parts.
Services that match no part become ghost parts on the overlay instead of a count.

Design: `docs/plans/140-observed-system/design.md` §4.

## Decision

**The `service` field.**

- A map part may carry `service: { namespace?, name }` in `system-map.yml`
  (`schema/system-map.schema.json`, `MapNode.service` in `rubato-protocol`).
- There is no default. A part without `service` is never matched, not by its id and not by name.
- No two parts in one map may carry the same `service`. Both the map loader and the draft refuse
  it.

**The wire.** `TrafficEdge` gains `client_namespace` and `server_namespace`, each optional and
additive.

**Matching.** An edge end matches a part only when both namespace and name are equal, an absent
namespace equalling only an absent one. A namespaceless `service`, for example `{name: github}`,
therefore matches only the servicegraph's virtual nodes built from `peer.service`. It never
matches a service that emits a namespace.

**What does not match.**

- An end with a namespace that matches no part is a ghost: `ObservedTraffic.ghosts`, the edges
  to it in `ghost_edges`, and `Finding::ObservedOnlyPart`.
- An end without a namespace that matches no part is counted only, in `unmapped_services`.

**Quiet.** `SeamQuiet` additionally requires both parts to carry `service`.

**Changing the field.** `service` is changed like any part field, through the shared draft
(ADR 0018).

**What stands from ADR 0039.** Everything else: one report shape, freshness, "unknown is never
quiet", and traffic never being an input to progress, pins, plans or approvals.

## Alternatives considered

- **The part id is `<namespace>/<name>`**: every existing part would need a new id, and ids
  can't be renamed (ADR 0018).
- **Keep id = `service.name`, add the namespace beside it**: still a name-only match whenever the
  namespace is missing, which #140 rules out.
- **Never match a namespaceless end**: contracts and externals, seen only through `peer.service`,
  could then never be lit. That defeats #142.
- **A mapping in server config**: identity would live outside the versioned map.

## Consequences

- **Existing maps and dev files.** Today's dev traffic files keep parsing, but their ends now
  match only parts that declare a namespaceless `service`. QUICKSTART says so, and Rubato's own
  map declares `service` on its parts.
- **A count that changes meaning.** `unmapped_services` keeps its name and gauge but now counts
  only ends with no identity to match; services that emit but are not on the map are listed as
  ghosts instead.
- **O-rows.** The `server-system-map` O-row (refusal `duplicate_service`) and the `web-server`
  O-row (`rubato.traffic.ghost_parts`, `rubato.traffic.unmapped_services`) of plan #140.

## Supersedes

0039, in part: the sentence "where `client` and `server` are OpenTelemetry `service.name` values"
and plan #104's rule that a service maps to the part whose id equals its name. The rest of 0039
stands.
