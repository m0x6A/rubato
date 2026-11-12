---
issue: m0x6A/rubato#80
title: Fixture plan with a Data models table
---

# Fixture: a plan with data models

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| rubato-protocol | affected | one more type | web-server |
| rubato-server | affected | one more table | web-server |
| `web` | affected | shows it | web-server |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| web-server | the type is missing | counter `rubato.web.rejected` | Web writes rejected | rubato-server |

## 5. Data models

Prose between the heading and the table is ignored.

| Model | Kind | Part | Change | Crosses | Note |
| --- | --- | --- | --- | --- | --- |
| `user_tokens` | table | `rubato-server` | new | | encrypted, owned by `auth` |
| `PlanFit`, `FitReason` | Type (serde) | rubato-protocol | New | `web-server` | per issue |
| `Plan.old_field` | type | rubato-protocol | removed | — | |
| | | | | | |
