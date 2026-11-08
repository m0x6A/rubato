---
issue: m0x6A/rubato#43
---

# Fixture: names a node the map does not have

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| rubato-companion | affected | something | companion-server |
| rubato-orchestrator | new | a node nobody declared | |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| companion-server | x | counter `rubato.events.dropped` | Companions | rubato-server |
