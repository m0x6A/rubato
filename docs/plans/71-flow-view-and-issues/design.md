# Interactive flow view, issues from the app, and one planning workflow

Issue: m0x6A/rubato#71 · Plan: [../71.md](../71.md) · ADRs: 0024–0028 (proposed)

Approved as a plan by Mattias Jonsson on 2026-11-12 (plan mode). His answers that shaped it:
scenarios for "moving parts"; issues created by the GitHub App; voice-note issues placed by a
`Map:` line, else by a suggested pin; feature-workflow folded into the rubato skill for this repo,
with review and approval watched in Rubato instead of written into plan files.

## Context

#71 (from a voice note) says Rubato as a planning tool "shoots beside the mark". It asks for:
- an interactive view of the system with moving parts, showing data and control flowing between parts;
- issues created from voice notes showing where they fit on the map;
- issues created from inside the app, following the right pattern.

You also asked whether the feature-workflow skills should be tied in or retired here.

**Today:**
- The data and control lenses draw declared seam flows as static arrows. Only a plan's proposed flows move (`web/src/features/map/flows.ts:191-249`, `styles.css:138-141`).
- Nothing creates issues: the GitHub App has Issues: read (ADR 0022), and the port has no `create_issue` (`kernel/github/api.rs:87-157`).
- Voice-note issues land in "Not pinned".
- feature-workflow and the rubato skill both answer "plan this" and both write `docs/plans/<N>.md`. The track, review and approval keys that feature-workflow adds are checked by nothing and contradict ADR 0009.

**Your decisions:**
1. **Moving parts = scenarios.** The map declares named journeys, and the view plays them step by step.
2. **Issues are created by the GitHub App.** It gets Issues: write and pins the issue at creation.
3. **Voice-note placement:** a `Map: node|seam <id>` line in the body is read first, and otherwise Rubato suggests a pin that a person confirms.
4. **feature-workflow:** fold it into the rubato skill, for this repo only. Review and approval are not written into plan files; Rubato shows them (derived from issue comments).

Track: full, since this changes the map schema, the protocol, the GitHub App permissions and several seams.

## Affected parts (ids from `docs/design/rubato-map.yml`)

| Node | Change |
|---|---|
| system-map | optional `scenarios[]`; 2–3 scenarios added to rubato-map.yml |
| rubato-protocol | `Scenario`/`ScenarioStep`, `issue_body` (render #71 headings, parse and write the `Map:` line), `NewIssueRequest`, `ReviewRecord`/`ApprovalRecord`, overview fields, otel names |
| rubato-server | scenario validation; `Map:` lines and suggestions in github_poll; auto-pin in pins; new slice `new_issue`; `create_issue` port; comment parsing for Review/Approved |
| web | scenario lens and player; `new_issue` feature; suggestion confirm in Backlog; review and approval badges |
| plan-check, skill-rubato, plan-file | scenario validation in TS; legacy front-matter count; skill fold-in |
| github (external) | App permission Issues: write |

Companion, tmux-ide and claude-code are quiet.

**O-rows** for each seam (server-system-map, skill-system-map, server-github, web-server, server-plan-file, skill-plan-file, plan-check-plan-file) go in `docs/plans/71.md`.

Key signals:
- `rubato.map.load_failures` with `ScenarioStepNoFlow`
- `rubato.issues.created{outcome}`
- `rubato.backlog.map_lines{outcome}`
- `rubato.web.rejected` reasons `cannot_write|no_issue_permission|...`
- finding `ApprovalStale`
- the `rubato.plan.check` line's `legacy_keys`

New panels: "Issues created" and "Map lines".

## ADRs (accepted by Mattias Jonsson, 2026-11-12)

- **0024 Scenarios in the map.** `scenarios[]: {id, name, description?, steps[{seam, from, to, kind, says?}]}`.
  - Each step must match a declared flow on that seam, so flows still need no id (ADR 0017).
  - Scenario ids are unique, but they can't be pinned and plans can't name them.
  - Map `version` stays 1.
- **0025 Rubato creates issues as the GitHub App.**
  - The App gets Issues: write. This supersedes only 0022's permission sentence; 0022's status is annotated and its text unchanged.
  - The server renders the body with #71's headings and the creator in backticks, ending with the `Map:` line.
  - The issue is pinned at creation.
  - Creation is offered only when the installation token reports `issues: write`.
- **0026 The `Map:` line places an issue.**
  - The last `Map: node|seam <id>` line in the body wins.
  - It is read from the open-issues listing, which already returns the body (`api.rs:160`), so it costs no extra calls.
  - It pins once per (issue, target), so a person's unpin sticks.
  - An issue without the line gets a scored suggestion, which a person confirms with one click.
- **0027 Review and approval are issue comments, derived by Rubato.**
  - `Review: <track> — <outcome>` is posted by the skill's review step.
  - `Approved: plan <blob-sha>` is posted by a human. It counts only for a non-Bot author with association OWNER, MEMBER or COLLABORATOR.
  - An approval of an older blob is shown as "earlier version" (`ApprovalStale`).
  - This supersedes 0009's key list: front matter holds `issue, title, spend_cap_usd, attempt_limit` and optionally `map_version`.
  - Existing plans are left as they are; plan-check counts their legacy keys without failing.
- **0028 One planning workflow here: the rubato skill.**
  - Track sizing, the design-review step and the spec headings move into `skills/rubato`.
  - `.claude/profiles/feature-workflow.md` is retired. The plugin itself is not changed.
  - `docs/FEATURES.md` is frozen; Rubato shows plan, review and approval per issue.

## PR sequence (each mergeable alone, test first, branch `issue/71-<slug>`)

**PR 0 `issue/71-plan`: ADRs 0024–0028, `docs/plans/71-flow-view-and-issues/design.md`, `docs/plans/71.md`.**
- The front matter is already in the 0027 form.
- Its design review goes on #71 as a `Review:` comment, not into the file.
- plan-check passes with an O-row for every seam.
- You accept the ADRs before PR 1 merges.

**PR 1 `issue/71-scenarios-in-map`: schema, protocol, both validators, rubato-map.**
- Shared fixtures `schema/fixtures/maps/scenarios*.yml`: valid, step-off-flow and unknown-seam. Both the Rust and the TS suites must fail on the bad ones.
- `crates/rubato-protocol/src/map.rs`:
  - `SystemMap.scenarios` with `serde(default)`.
  - `validate()` (:177-220) adds `ScenarioStepNoFlow`, `ScenarioUnknownSeam`, `EmptyScenario` and duplicate ids, reusing `Seam::joins`. `ids()` stays unchanged.
- Regenerate `schema/system-map.schema.json`, `schema/protocol/*` and `web/src/protocol.ts`.
- `actions/plan-check/src/map.ts` `structuralProblems` (:65-91) gets the same rules.
- `map_draft/yaml.rs` `unknown_keys` (:55) learns `scenarios`. `map_draft/apply.rs` refuses removing a seam or flow a scenario uses, naming the scenario.
- `skills/rubato/scripts/map.ts` prints scenarios.
- `docs/design/rubato-map.yml` gets its scenarios last ("start a session on an issue", "a plan comes back", "propose a map change"), after the model knows the key.
- Test: a map with an off-flow scenario keeps the previous map and counts a failure (next to `map/load.rs` `invalid_flows_keep_the_previous_map`).

**PR 2 `issue/71-scenario-player`: web.**
- New pure `web/src/features/map/scenario.ts`:
  - `stepsToArrows` resolves each step to the `arrowsFor` arrow id (`flows.ts:50-71`). An unresolved step is counted as `unplaced`, as `lens.ts` does.
  - `playerReducer` holds the step, play/pause and speed.
- `Lens` gains `"scenario"`.
- `MapView.tsx`:
  - A scenario picker next to the lens radiogroup (:282-290).
  - Play, pause, step back and step forward, keyboard operable, with an `aria-live` caption ("Step 3 of 8: rubato-server → rubato-companion, control: LaunchCommand").
  - `SeamEdge` (:42-68) moves a pulse along the path with SVG `animateMotion`. Data is a filled dot, control a ring. This stays distinct from the plan lens's `.moving` dash, which means "proposed".
  - Only the active step's parts are lit; the rest are dimmed.
- Inspector shows the step list.
- Under reduced motion, no animation: the step is highlighted and the caption carries it.
- Tests:
  - `scenario.test.ts`
  - map-view step test
  - `flows.test.ts` proves the plan lens is unchanged
  - `web/e2e/map.spec.ts` plays a scenario with and without reduced motion

**PR 3 `issue/71-map-line`: `Map:` line and pin suggestions.**
- `rubato-protocol` `issue_body.rs`: `parse_map_line` and `map_line`. Shared fixtures `schema/fixtures/issues/*.md`: a voice-note body, a line inside a code fence (ignored), two lines (the last wins), an unknown id.
- `issues_from_page` (`api.rs:160-190`) reads `body`, giving `map_line` and a 4 KB excerpt that never reaches the web.
- Migration `0006`: `open_issues` gets `map_ref` and `suggestion`, and new table `map_line_pins` is owned by pins.
- `github_poll/backlog.rs:39-78` stores them, and the pure `github_poll/suggest.rs::suggest(map, title, body)` scores suggestions. A log line `backlog placement` carries the counts.
- `tasks.rs` calls the new `pins::apply_map_lines`.
  - It reuses the id and kind checks extracted from `pins/handlers.rs:27-47` into `pins::check_target`, plus `store::add_pin`.
  - The counter `rubato.backlog.map_lines{outcome}` records each outcome.
- `fake.rs`, `local.rs` and `web/e2e/fixtures/issues.json` carry bodies.
- `Backlog.tsx` shows "Suggested: <part>" and a "Pin here" button using the existing `POST /api/pins`.
- `docs/QUICKSTART.md` gets "Placing issues from other tools", the contract for the voice tool.
- Tests:
  - pins once, and a later unpin is not undone (fails on purpose without the guard);
  - an unknown id is counted and not pinned;
  - the suggestion score is deterministic;
  - the existing `FakeGitHub.calls` order test shows no new calls;
  - e2e: a fixture issue with `Map: seam web-server` appears on that seam.

**PR 4 `issue/71-create-issue`: write port, `new_issue` slice, web form.**
- Port:
  - `GitHub::create_issue` and `can_create_issues()` (octocrab `issues().create`).
  - The installation-token `permissions.issues` is recorded in `kernel/github/app.rs`.
  - `fake.rs` gets a failure mode and a timeout. `local.rs`'s `open` becomes a Mutex, so dev mode works.
  - `web/e2e/github-standin.ts:151` gets `issues: "write"` and a POST route.
- Protocol `issue_new`: `NewIssueRequest{request_id, repo, target, title, summary, does[], know[], must_not_break[], context?, open_questions[]}`, `ISSUES = "/api/issues"`. `issue_body::render_issue` writes the headings, checkboxes for `know`, "Opened in Rubato by `login`" (no bare @) and the `Map:` line.
- Slice `crates/rubato-server/src/new_issue/` owns `issue_creations(request_id PK, …, state)` (migration 0007). The handler:
  1. checks the target with `pins::check_target`;
  2. checks the repo is in `repos_named()`;
  3. checks the capability;
  4. stores the request as `opening`;
  5. calls `create_issue`;
  6. pins the issue, then `notify("issue")`.

  It is idempotent per `request_id`. After a timeout the row stays `opening`, and the `Map:` line repairs the pin on the next listing.
- `app.rs` route. The AGENTS.md server and protocol blocks get `new_issue`/`issue_creations` and `issue_new`. `otel.rs` gets the new names, and the overview gets `can_create_issues`.
- Web feature `web/src/features/new_issue/`:
  - "New issue here…" from a selected part or seam in the Inspector, and from the issue list.
  - The repo comes from the part, or from the seam's owner.
  - Required: title, summary, at least one "does", one "we'll know" and one "must not break".
  - When creation isn't possible, the form is disabled and says why.
  - AGENTS.md web block and `features.test.ts` updated.
- `docs/QUICKSTART.md:35`: Issues: write, and re-approving the installation.
- Tests:
  - snapshot of the rendered body;
  - handler tests: success pins, a duplicate request id makes one issue, 403, read-only permission refused (fails on purpose for the gate), unknown target 422, timeout;
  - slice-layout test;
  - web form test;
  - e2e in both the dev and app projects: an issue created from a part appears pinned on it.

**PR 5 `issue/71-review-approval`: derive and show.**
- `rubato-protocol/src/github.rs`: `ReviewRecord`/`ApprovalRecord` parsers modelled on `DecisionRecord::parse_body` (:62-107). `IssueSummary.review` and `.approval{record, current}`, and `Finding::ApprovalStale`.
- `api::Comment` (`api.rs:49-56`) gains `author_association` and `author_is_bot`, taken from the existing `issue_comments` call.
- `github_poll/poll.rs:236-258` parses Decision, Review and Approved in one pass with no new calls. Table `plan_signals` (migration 0008). A log line `approval ignored` gives the reason.
- The `plans` slice records the plan's git blob sha (computed locally), and `overview/build.rs` compares it with the approval.
- Web: IssuePanel shows "Reviewed (small, passed) · Approved by `x` · current / earlier version", with badges on issue rows and PartCard.
- The UI says signals arrive within 300 s (the ADR 0023 full lane).
- Fixtures `schema/fixtures/comments/*.md`.
- Tests:
  - an approval from a Bot or with association NONE is ignored (fails on purpose);
  - a stale approval raises the finding;
  - `Decision:` parsing is unchanged.

**PR 6 `issue/71-skill-workflow`: fold feature-workflow into the rubato skill.**
- `skills/rubato/SKILL.md`:
  - a "Size the track" step, with this repo's triggers moved over from the profile (:27-31);
  - body headings does / we'll know / must not break / 3 am, matching the issue form;
  - a "Design review" step: run the design-reviewer, then post a `Review:` comment from the new `review-template.md`, with the sha from `plan.ts sha`;
  - an "Approval" step: ask the human to comment `Approved: plan <sha>`; the agent never posts it;
  - no review, approval or track front matter.
- `scripts/plan.ts:26-33` writes only the allowed keys and the new headings. Update `prompt.md` and `install.ts`, which copies the new templates.
- `actions/plan-check/src/parse-plan.ts` and `check.ts` report `legacy_keys` without failing.
- Delete `.claude/profiles/feature-workflow.md`. Its caveat rows (:43-48) move to `skills/rubato/review-checklist.md`.
- `docs/FEATURES.md` gets a frozen note, and AGENTS.md "How we work" describes the one workflow, with review and approval as comments.
- Tests:
  - `init` output has no legacy keys and has the four headings;
  - no script or template emits `Approved:`;
  - fixture `legacy-keys.md` reports `legacy_keys=2` and passes;
  - the must-fail fixture still fails;
  - the full `docs/plans/*.md` still passes.

## Assumed, not yet approved (inside a node)

- **A1:** scenario steps need not be contiguous.
- **A2:** a draft that removes a seam or flow a scenario uses is refused (no cascade).
- **A3:** approval is a badge, not a new progress state.
- **A4:** "must not break" is required in the form.
- **A5:** a seam's issue goes to the owner node's repo.
- **A6:** suggestions are shown at a score of 0.5 or more, top one only.
- **A7:** a 4 KB body excerpt is stored, server side only.
- **A9:** scenarios are YAML-only in v1, not editable through drafts.
- **A10:** an approval without a sha shows as "version not named".
- **A8 (near the server-github seam, needs your yes or no):** a `Map:` line pins whatever the issue author is, but it can only name existing map ids.

## Verification

```
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd web && pnpm gen:types && git diff --exit-code -- src/protocol.ts
pnpm typecheck && pnpm test
pnpm plan-check --map docs/design/rubato-map.yml --plans "docs/plans/*.md"
pnpm observability:check && pnpm observability:test && pnpm observability:rules
cargo build -p rubato-server && (cd web && pnpm build) && ./web/e2e/run.sh
pnpm e2e
```

**Manual check:** run the server in dev mode on rubato-map.yml.
- Play "start a session on an issue" step by step.
- Create an issue from the `rubato-server` part and see it pinned there.
- Paste a body with `Map: seam web-server` into a fixture issue and see it placed.
- Post a `Review:` comment and an `Approved: plan <sha>` comment on a test issue and see both badges.

New metrics get panels in `observability/dashboards/rubato-overview.json`.

## Changes after the design review (2026-11-12)

The review found 2 blockers and 5 should-fix items. All are taken, and they override the PR text
above where the two differ. Mattias Jonsson decided the two open choices: members-only trust, and
an approval hash that ignores task checkboxes.

1. **Duplicate issues (blocker, PR 4).**
   - The create POST runs with octocrab retries off (`Cargo.toml:26` enables retry; `api.rs:12-13`).
   - The body carries `Rubato-Request: <id>`.
   - PR 3's listing reconciles `opening` rows from that line, moving them to `created`. A row is `failed` after 10 minutes.
   - A resubmit of an `opening` id answers "still opening" and sends nothing.
   - Gauge `rubato.issues.opening`.
2. **Untrusted text (blocker, PR 3 and PR 5).**
   - `Map:` lines and `Review:` comments count only from OWNER, MEMBER or COLLABORATOR authors, or the App for issues it created. The listing and comment payloads already carry `author_association`.
   - Anyone else's `Map:` line becomes only the suggestion, with outcome `untrusted_author`. So strangers cannot add issues to the tracked set (`tasks.rs:97`).
3. **Unpin sticks for issues created in the app (PR 3 and PR 4).** Creation pins through a pins function that also records the `map_line_pins` row.
4. **Approval binding (PR 5).**
   - The approval names `plan <sha>` and optionally `on <branch>`.
   - The sha is the blob sha of the plan with checkboxes normalised (`[x]` → `[ ]`).
   - Rubato compares it with the plan on the named branch, or on the branch it reads the plan from (`poll.rs:112-127` picks the first `issue/<n>-*`).
   - Tested with two branches.
5. **Ambiguous steps (PR 1).** A step may name `carries`, and a step matching more than one flow fails as ambiguous. companion-server has two server→companion control flows (`rubato-map.yml:84-85`). A shared fixture covers it.
6. **Permission source (PR 4).**
   - `can_create_issues` comes from `GET /app/installations/{id}` → `permissions.issues`, read as the App at start and on every full pass. `crab.installation()` does not expose permissions (`api.rs:228-235`).
   - A 403 on create clears it until the next read.
7. **Panels (all PRs).**
   - O-rows now name panels that exist (Web writes rejected, Plan checks, Plan check log lines, Map age).
   - New panels come in the PR that adds their signal: "Map lines" (PR 3), "Issues created" with the opening gauge (PR 4), "Approvals" with `rubato.plans.approval_stale` (PR 5).

### Concurrency (PR 4)

| Case | What happens |
| --- | --- |
| Same `request_id` submitted twice while the first is opening | the second answers "still opening"; one POST |
| Same `request_id` after created | the stored issue and url are returned; no POST |
| POST times out | the row stays `opening`; the next listing finds the `Rubato-Request:` line, stores the number and pins it; after 10 min unseen, `failed` |
| Client retry inside octocrab | off for this call |
| Listing reconcile and handler finish together | both write the same number through an `UPDATE … WHERE state = 'opening'`; the pin insert is `OR IGNORE` |
| Two people create the same title | two issues; no dedupe on content |
