---
issue: m0x6A/rubato#42
title: Fixture plan that passes
spend_cap_usd: 10
attempt_limit: 2
---

# Fixture: valid plan

Prose before the tables is ignored by the parser.

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| `rubato-companion` | affected | forwards one more hook field | companion-server, `companion-claude-code` |
| rubato-server | affected | reads the new field | companion-server |
| web | quiet | — | |

## Assumed, not yet approved

| # | Decision | Chosen | Alternatives | Scope |
| --- | --- | --- | --- | --- |
| F1 | Field name | `tool_name` | `tool` | node (companion) |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| companion-server | server drops the event | counter `rubato.events.dropped` | Companions | rubato-server |
| companion-claude-code | hook stops firing | counter `rubato.hook.forwarded` at zero | Hooks/min | rubato-companion |
