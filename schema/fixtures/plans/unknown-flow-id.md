---
issue: m0x6A/rubato#45
title: "Fixture, a flow with an unknown id"
---

# Fixture: a plan with flows

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| rubato-server | affected | serves one more field | web-server |
| `web` | affected | draws it | web-server |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| web-server | the field is missing | counter `rubato.web.rejected` | Web writes rejected | rubato-server |

## 4. Flows

Prose between the heading and the table is ignored.

| From | To | Seam | Kind | Carries |
| --- | --- | --- | --- | --- |
| rubato-server | `web` | web-server | data | the new field, in `Overview` |
| web | rubato-orchestrator | web-nowhere | control | ids the map does not have |
| | | | | |
