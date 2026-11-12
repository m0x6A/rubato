---
issue: m0x6A/rubato#80
title: Must fail, a data model crossing a seam the plan does not touch
---

# Must fail: model off seam

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| rubato-protocol | affected | one more type | web-server |
| web | affected | shows it | web-server |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| web-server | missing | counter `rubato.web.rejected` | Web writes rejected | rubato-server |

## Data models

| Model | Kind | Part | Change | Crosses | Note |
| --- | --- | --- | --- | --- | --- |
| `HookEvent` | type | rubato-protocol | changed | companion-server | companion-server is not touched |
