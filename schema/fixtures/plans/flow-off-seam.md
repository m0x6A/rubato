---
issue: m0x6A/rubato#44
title: "Fixture, a flow off its seam"
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
| web | github | web-server | control | a seam that does not join them |
| | | | | |
