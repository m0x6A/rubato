# 0018. Edit the map as a shared draft on the server and propose it as a pull request

- **Status:** proposed
- **Date:** 2026-11-10
- **Decided by:** agent (Claude Code, issue #48 design session); proposed for Mattias Jonsson to accept
- **Seam:** server-system-map, server-github, web-server
- **Issue:** m0x6A/rubato#48

## Context

Issue #48 and its planning session ask that the map can be modified from the UI: add a part, draw a seam, set flows. ADR 0007 makes the map a versioned file whose ids are the only ones pins and plans may name, and ADR 0010 has the server read that file from GitHub. An edit in the UI therefore cannot simply change what the server holds. Design: `docs/plans/48-interactive-map/design.md` (D6, D7, D8).

## Decision

`rubato-server` keeps one shared draft as an ordered list of operations (add, update and remove for parts and seams), each with its author and time, in SQLite, owned by a `map_draft` slice. The draft is shown on the map as an overlay. Draft ids are not map ids: they cannot be pinned and plans naming them still have unknown ids. "Propose map change" makes the server write the resulting YAML to a branch `map/draft-<time>` of the map repo, started at the map branch's head, and open a pull request whose body lists the operations and their authors. This is the first time the server writes to GitHub; it writes as the GitHub App installation of ADR 0022, which is also what decides who may ask for it. The server refuses to propose when the map file at that head is not the one it has loaded, when the file has keys the model does not know, or when an operation no longer applies. One proposal can exist at a time; from the moment it is created the draft accepts no operations, it empties when the pull request is merged and the map reloaded, and it is editable again, with every operation kept, when the pull request is closed or the proposal is withdrawn. Ids cannot be renamed. The proposed file is serialised from the map model, so hand-written comments in the file are not kept.

## Alternatives considered

- **The server as the map's source of truth**: fastest to use, but supersedes 0007 and 0010 and breaks the skill and the plan check, which read the file.
- **Layout and flows only, parts added by hand in YAML**: does not meet "modify".
- **A stored whole-map draft**: loses who changed what and cannot be replayed when the merged map moves.
- **Opening the pull request with the user's own token**: no user token is kept after login, and keeping one for every user is more to protect.
- **A personal token with write access**: tied to one person and their rate limit; the team chose the GitHub App registration (ADR 0022).

## Consequences

The map stays a reviewed, versioned file and merging stays a human act on GitHub. The App needs Contents and Pull requests write on the map repo; the server makes three kinds of write with it (one branch, one file, one pull request). Without an installation, or when the map is read from a local file, the UI offers the YAML to copy. A map change merged while a draft is open shows as a conflict on the pull request, not as a silent revert. A new failure mode, a proposal that cannot be opened or is not seen to merge, is covered by the `server-github` O-row (`rubato.map.proposals`, `rubato.map.proposal.state_since`, the `map proposal failed` log line with its step).

## Supersedes

None. It keeps 0007 and 0010.
