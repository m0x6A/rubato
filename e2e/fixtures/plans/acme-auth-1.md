---
issue: acme/auth#1
title: Add token introspection endpoint
status: proposed
spend_cap_usd: 5
attempt_limit: 2
---

# Add token introspection endpoint

## Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| auth-service | affected | new `/introspect` handler | auth-service-contract |
| auth-contract | affected | new operation in the OpenAPI document | auth-service-contract, auth-contract-pantry |

## Assumed, not yet approved

| # | Decision | Chosen | Alternatives | Scope |
| --- | --- | --- | --- | --- |
| A1 | Response shape | RFC 7662 | custom | seam |

## O-rows

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| auth-service-contract | handler and contract disagree | openapi diff check in CI; counter `auth.contract.mismatch` | Auth | auth-contract |
| auth-contract-pantry | pantry sends a token shape introspection rejects | counter `auth.introspect.rejected` by caller | Auth | auth-contract |
