---
issue: acme/pantry#2
title: Show login state in the header
status: proposed
spend_cap_usd: 5
attempt_limit: 2
---

# Show login state in the header

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| pantry-web | affected | header reads `/api/me` | pantry-web-api |
| pantry-api | affected | `/api/me` endpoint | pantry-web-api |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| pantry-web-api | header shows logged out while the cookie is valid | browser error counter `pantry.web.me_failed` | Pantry | pantry-api |
