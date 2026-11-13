# 0036. Self-host the web's type pairing from npm font packages

- **Status:** accepted
- **Date:** 2026-11-13
- **Decided by:** m0x6A, accepted 2026-11-13 by approval comment https://github.com/m0x6A/rubato/issues/81#issuecomment-6861609460 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #81) — proposed
- **Seam:** none (dependency)
- **Issue:** m0x6A/rubato#81

## Context

Issue #81 restyles the web app and asks for a distinct type pairing. The web uses the system font
stack today and has no font files. A pairing needs font files, and AGENTS.md makes a dependency an
ADR. Rubato runs on a team's own server; the page must not reach a third-party host when it loads.
Design: `docs/plans/81-bold-restyle/design.md` §2.

## Decision

The `web` package depends on `@fontsource-variable/space-grotesk` and
`@fontsource-variable/jetbrains-mono` (OFL-1.1), pinned to an exact version, imported once in
`web/src/main.tsx`. Vite bundles their `.woff2` files into `web/dist/assets` with hashed names and
`rubato-server` serves them with the rest of `dist`. The page requests fonts from its own origin
only; an e2e test fails on a request to any other host. The CSS names the families through two
tokens, `--font-ui` and `--font-mono`, with system fallbacks, so a missing file degrades to today's
look.

## Alternatives considered

- **A CDN (Google Fonts)**: a request to a third party on every page load, and no fonts offline.
- **Copy the `.woff2` files into the repo**: no npm dependency, but no version or licence record
  and manual updates.
- **System fonts only**: no new dependency, but no distinct pairing, which #81 asks for.

## Consequences

`dist` grows by the latin and latin-ext subsets the page uses (tens of kilobytes per family). A
font upgrade is a package bump. Nothing crosses a seam; the server code is unchanged.

## Supersedes

None.
