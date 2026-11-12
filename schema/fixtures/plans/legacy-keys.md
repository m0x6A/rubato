---
issue: m0x6A/rubato#43
title: Fixture plan with front matter from before ADR 0027
review: { small: passed, by: design-reviewer, date: 2026-11-10 }
approval: "given up front"
spend_cap_usd: 10
attempt_limit: 2
---

# Fixture: legacy front matter

`review` and `approval` are counted as legacy keys (ADR 0027). The plan still passes.

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| rubato-server | affected | reads the new field | companion-server |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| companion-server | server drops the event | counter `rubato.events.dropped` | Companions | rubato-server |
