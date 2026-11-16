# 0076. A headless companion may run as a daemon or in a container, and Claude Code may use an API key; the companion says which, never the key

- **Status:** accepted
- **Date:** 2026-11-15
- **Decided by:** Mattias Jonsson (m0x6A), approved in the Claude Code session 2026-11-15; drafted by agent
- **Seam:** companion-server, companion-claude-code
- **Issue:** m0x6A/rubato#134

## Context

#134 asks for three things besides the lead agent. The docs should state today's assumptions:
tmux and a local Claude Code. An agent should be able to use "the API version of Claude". And an
agent should run as a daemon or in a Docker container. ADR 0062 already gives a companion with no
terminal (`mode: headless`, one `claude -p` turn at a time). Nothing yet says how to host it
unattended, or how a session is billed. Claude Code is the only agent (ADR 0003). A second agent
built on the Messages API would reopen that. Plan: `docs/plans/134.md`; design:
`docs/plans/134-orchestrator/design.md` §6.

## Decision

**Hosts.** `docs/design/companion-hosts.md` lists the ways to run a companion:
- interactive: WSL or macOS, tmux, tmux-ide, `claude` logged in (ADR 0003, today);
- headless on a machine: `rubato-companion run` as a systemd user unit or a launchd agent, no
  tmux (ADR 0062);
- headless in a container.

All three dial out only, pair by device code with one person, and keep their state dir private.

**The container.**
- `Dockerfile.companion` builds an image with the companion, `git`, Node and Claude Code at a
  pinned version.
- It runs as a non-root user, with one volume for the config, the state dir, the clones and the
  worktrees. Pairing is `docker run -it … rubato-companion pair`.
- It runs only in headless mode: the image refuses `mode: interactive` at start.
- There is no Docker-in-Docker and no Docker socket. A worker's tools run inside the same
  container.
- CI does not build or publish this image. Whoever runs it builds it from the repository.

**Claude through an API.** "The API version of Claude" is Claude Code authenticated by an API
key, not a second agent. The companion passes the environment Claude Code already reads to every
`claude` it starts, and adds nothing on the command line:
- `ANTHROPIC_API_KEY`;
- `CLAUDE_CODE_USE_BEDROCK` or `CLAUDE_CODE_USE_VERTEX` and their credentials;
- an `apiKeyHelper` in the user's Claude settings.

**Who pays is visible.** At start the companion works out how Claude is authenticated, never
reading a value: `api_key` (`ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` set, or an
`apiKeyHelper` key in the user's Claude settings), `bedrock`, `vertex`, or otherwise `login`. It
announces that in Hello as the capability `claude_auth:<kind>`. The web shows *API key* (or the
cloud's name) on the companion's lane, so the team can tell when spend is billed per token. The
label describes the companion's own environment; an interactive pane inherits the tmux server's,
which can differ. The key never leaves the machine, is on no command line, and is not logged.

## Alternatives considered

- **An agent loop on the Messages API or the Agent SDK inside the companion**: a second agent, against ADR 0003. Hooks, the session log, forks and plans would all need rebuilding for it.
- **Docker-in-Docker, or mounting the Docker socket, so each worker gets its own container**: the socket is root on the host. A container per worker is a later step with its own ADR.
- **Publish the image from CI**: changes a publishing workflow, which this change must not do. It is a later step once the image has been run by hand.
- **Send the auth kind as a new Hello field**: a schema change for one label. The `skill:` capability prefix already carries such a label.
- **Tell nobody how Claude is billed**: spend is shown either way, but an API key turns a cap hit into real money, and the team should see that.

## Consequences

Easier: a person can run their own agents on a server or in a container with an API key, under the
same pairing, caps and visibility as on their laptop. A lead agent (ADR 0072) has somewhere to run
unattended.

Harder: a container holds the clones, worktrees and the key. Its volume is as sensitive as the
developer's home directory, and the docs say so. The image pins a Claude Code version that someone
must bump. A wrong or expired key shows up as failed headless turns.

O-rows:
- companion-claude-code: `rubato.headless.turns{rubato.outcome="failed"}` with the turn's
  `result` error, and span `rubato.launch` with `rubato.claude.auth`;
- companion-server: `rubato.claude.auth` on the Hello span.

## Supersedes

None. ADR 0003's "Claude Code is the only agent" stands, and ADR 0062's headless mode is what
runs here.
