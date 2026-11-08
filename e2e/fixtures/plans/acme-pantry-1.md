---
issue: acme/pantry#1
title: Validate tokens through introspection
status: proposed
spend_cap_usd: 5
attempt_limit: 2
---

# Validate tokens through introspection

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| pantry-api | affected | call `/introspect` instead of decoding locally | auth-contract-pantry |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| auth-contract-pantry | introspection down or slow | span `pantry.auth.introspect` latency and error rate | Pantry | auth-contract |
