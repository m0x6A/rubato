---
issue: m0x6A/rubato#43
title: Fixture plan with a Flows table
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
| web | rubato-server | `web-server` | Control (a request) | asks for it |
| | | | | |
