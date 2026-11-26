-- Issue #327 (ADR 0084, plan 324 design §5). Additive.
--
-- Owner github_poll: who wrote each open issue, as GitHub's listing says it. The context graph
-- keeps an issue's title only when its author is on the team, the rule a `Map:` line is already
-- held to (ADR 0026). A row from before these columns has '' and reads as not on the team until
-- the next listing, one poll tick later.
ALTER TABLE open_issues ADD COLUMN author TEXT NOT NULL DEFAULT '';
ALTER TABLE open_issues ADD COLUMN author_association TEXT NOT NULL DEFAULT '';
