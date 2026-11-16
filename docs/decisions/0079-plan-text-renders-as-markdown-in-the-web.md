# 0079. Render plan text as Markdown in the web, with react-markdown and no raw HTML

- **Status:** accepted
- **Date:** 2026-11-16
- **Decided by:** Mattias Jonsson (m0x6A), by approving plan #265 at 89d8c31 on 2026-11-16 (https://github.com/m0x6A/rubato/issues/265#issuecomment-6927411780); drafted by agent
- **Seam:** none (a dependency of `web`)
- **Issue:** m0x6A/rubato#265

## Context

A plan draft's files are shown as raw Markdown in 12 px monospace (`web/src/features/plans/DraftSection.tsx`).
Issue #265 asks that plans can be read in Rubato, in a panel that can be widened. Tables, which
carry most of a plan (affected parts, O-rows, decisions), are unreadable as raw text. `web/` has
no Markdown library, and a new dependency is an ADR (AGENTS.md, "Decisions are records").

Plan text is written by agents and people on the team, and by anyone whose branch a team member
imports. It is untrusted input to the page.

## Decision

The web renders the `.md` files of a plan draft with `react-markdown` and `remark-gfm`, both
pinned to exact versions, loaded lazily so the first paint does not pay for them.

- **No raw HTML.** `rehype-raw` is not used; HTML in a plan shows as text. Nothing uses
  `dangerouslySetInnerHTML`.
- **No images.** An image becomes a link with its alt text, so opening a plan makes no request
  to a host the plan names.
- **Links** keep react-markdown's URL filter (no `javascript:`), and open in a new tab with
  `rel="noopener noreferrer"`.
- **Mermaid and other code blocks stay code.** No diagram library.
- **Raw stays one click away**, and the "what changed" diff stays raw text.
- Only the web reads the rendering. The plan file, the plan check and the server's parser are
  unchanged ("Markdown plans; no HTML" still describes the files).

A Vitest test feeds `<script>`, `<img onerror>`, a `javascript:` link and an image through the
component and asserts none reaches the DOM as an element, and prints how many cases it checked.

## Alternatives considered

- **Keep raw text, larger and wider**: no dependency, but tables stay unreadable; Mattias chose rendering.
- **`marked` + DOMPurify**: smaller, but renders through an HTML string and `dangerouslySetInnerHTML`; safety then rests on the sanitiser's configuration.
- **`markdown-it`**: same HTML-string path.
- **Render on the server**: puts presentation in `rubato-server` and HTML on the `web-server` seam.
- **Mermaid rendering too**: about 1 MB more script for diagrams the map already draws from the plan's Flows.

## Consequences

Plans read as documents in the wide panel. `web/` carries two more dependencies and their
transitive unified/micromark tree, in a lazy chunk; `pnpm audit` covers them. A Markdown feature
the renderer mishandles is read through the Raw toggle. Rendering other Markdown (issue bodies,
room messages) is not decided here; it would reuse the same component and the same rules.

## Supersedes

None.
