import { useEffect, useId, useMemo, useState } from "react";
import type { DraftRevision, DraftSummary, IssueSummary, Me } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import { relative } from "../../shared/format.ts";
import { useWebResource } from "../live/index.ts";
import { draftFailureText, planDraftsApi } from "./api.ts";
import { type FileChange, bundleOrder, diffBundles, hunks } from "./diff.ts";
import { readSeen, writeSeen } from "./seen.ts";

const sameLogin = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const count = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

/**
 * The issue's plan draft in Rubato (ADR 0053): who owns it, its newest settled revision, whether
 * the owner's session is working on it, what changed since you last looked (on this device), and
 * every revision's files. Everyone reads drafts; nothing here writes one.
 */
export function DraftSection({ issue, me }: { issue: IssueSummary; me: Me | null }) {
  const d = issue.draft;
  // Keyed by the draft, so another issue's draft starts with its own "last looked at".
  return d ? <Draft key={d.draft_id} d={d} me={me} /> : null;
}

interface RevisionState {
  rev: number;
  data?: DraftRevision;
  error?: string;
}

/** One revision's files. Revisions never change once stored, so each is read once per draft view. */
function useRevision(read: (rev: number) => Promise<DraftRevision>, rev: number | null): RevisionState | null {
  const [state, setState] = useState<RevisionState | null>(null);
  useEffect(() => {
    if (rev === null) {
      return;
    }
    let current = true;
    read(rev).then(
      (data) => current && setState({ rev, data }),
      (e: unknown) => {
        if (current && !(e instanceof ApiError && e.status === 401)) {
          setState({ rev, error: draftFailureText(e) });
        }
      },
    );
    return () => {
      current = false;
    };
  }, [read, rev]);
  return rev !== null && state?.rev === rev ? state : null;
}

function Draft({ d, me }: { d: DraftSummary; me: Me | null }) {
  const heading = useId();
  const id = d.draft_id;
  // The server says `updated` for this draft when a revision is stored.
  const view = useWebResource({ kind: "draft", draft_id: id }, () => planDraftsApi.draft(id), draftFailureText);
  // What you had seen when you opened it: the diff's baseline while the view stays open.
  const [baseline] = useState(() => readSeen(id));
  const [chosen, setChosen] = useState<number | null>(null);
  const read = useMemo(() => {
    const cache = new Map<number, Promise<DraftRevision>>();
    return (rev: number) => {
      let p = cache.get(rev);
      if (!p) {
        p = planDraftsApi.revision(id, rev);
        cache.set(rev, p);
        p.catch(() => cache.delete(rev));
      }
      return p;
    };
  }, [id]);

  const revisions = view.data?.revisions ?? [];
  const newest = revisions.at(-1)?.rev ?? null;
  const shown = chosen ?? newest;
  // A baseline the draft does not hold (another server, a reset) counts as nothing seen.
  const since = baseline !== null && revisions.some((r) => r.rev === baseline) ? baseline : null;
  const comparing = since !== null && newest !== null && since !== newest;

  useEffect(() => {
    if (newest !== null) {
      writeSeen(id, newest);
    }
  }, [id, newest]);

  const before = useRevision(read, comparing ? since : null);
  const after = useRevision(read, comparing ? newest : null);
  const files = useRevision(read, shown);
  const changes = useMemo(() => (before?.data && after?.data ? diffBundles(before.data.files, after.data.files) : null), [before, after]);

  const whose = me && sameLogin(me.login, d.owner) ? "your session" : `${d.owner}'s session`;
  const missing = d.missing ?? [];
  return (
    <section className="draft" aria-labelledby={heading} data-testid="draft">
      <h3 id={heading} style={{ marginTop: 12 }}>
        Draft · revision {d.rev}
      </h3>
      <p>
        <span className={`badge ${d.live ? "draft-live" : "draft-parked"}`}>{d.live ? `being worked on in ${whose}` : "parked"}</span>{" "}
        <span className="muted">
          owned by {d.owner} · revision {d.rev},{" "}
          <time dateTime={d.at} title={new Date(d.at).toLocaleString()}>
            {relative(d.at)}
          </time>{" "}
          · on <code>{d.branch}</code>
        </span>
      </p>
      {!d.live && <p className="muted">No session of {d.owner}'s is working on it; the draft waits in Rubato.</p>}
      {d.parse_error && <p className="warn">The plan does not parse: {d.parse_error}</p>}
      {missing.length > 0 && (
        <div className="warn">
          <p>The plan links these, which are not written yet:</p>
          <ul className="draft-missing" aria-label="The plan links these, which are not written yet">
            {missing.map((p) => (
              <li key={p}>
                <code>{p}</code>
              </li>
            ))}
          </ul>
        </div>
      )}
      {view.error && <p className="warn">This draft could not be read: {view.error}</p>}
      {view.data === null && !view.error && <p className="muted">Loading the revisions…</p>}
      {newest !== null && (
        <>
          {since === null ? (
            <p className="muted">You have not looked at this draft before; this is revision {newest}.</p>
          ) : since === newest ? (
            <p className="muted">Nothing changed since you last looked (revision {newest}).</p>
          ) : (
            <Changes since={since} newest={newest} changes={changes} error={before?.error ?? after?.error ?? null} />
          )}
          <h4>Revisions</h4>
          <ol className="plain draft-revisions" aria-label="Revisions of this draft">
            {[...revisions].reverse().map((r) => (
              <li key={r.rev}>
                <button type="button" aria-pressed={r.rev === shown} onClick={() => setChosen(r.rev === newest ? null : r.rev)}>
                  Revision {r.rev}
                </button>{" "}
                <time className="muted" dateTime={r.at} title={new Date(r.at).toLocaleString()}>
                  {relative(r.at)}
                </time>
                {r.rev === since && <span className="muted"> · you last looked at this one</span>}
                {r.parse_error && <span className="warn"> · does not parse</span>}
                {(r.missing ?? []).length > 0 && <span className="muted"> · {count((r.missing ?? []).length, "file")} not written yet</span>}
              </li>
            ))}
          </ol>
          {shown !== null && <Files rev={shown} state={files} openPlan={!comparing} />}
        </>
      )}
    </section>
  );
}

function Changes({ since, newest, changes, error }: { since: number; newest: number; changes: FileChange[] | null; error: string | null }) {
  const heading = useId();
  return (
    <section className="draft-changes" aria-labelledby={heading}>
      <h4 id={heading}>What changed since you last looked</h4>
      <p className="muted">
        revision {since} → {newest}
        {changes && `: ${changes.length === 0 ? "the files are the same" : `${count(changes.length, "file")} changed`}`}
      </p>
      {error && <p className="warn">The changes could not be read: {error}</p>}
      {changes === null && !error && <p className="muted">Comparing…</p>}
      {changes?.map((c) => (
        <FileDiff key={c.path} c={c} />
      ))}
    </section>
  );
}

function statusText(c: FileChange): string {
  switch (c.status) {
    case "changed":
      return `changed · ${c.added} added, ${c.removed} removed`;
    case "added":
      return `added · ${count(c.added, "line")}`;
    case "removed":
      return `removed · ${count(c.removed, "line")}`;
  }
}

function FileDiff({ c }: { c: FileChange }) {
  return (
    <details className="draft-change" open data-testid="draft-change">
      <summary>
        <code>{c.path}</code> <span className="change-status">{statusText(c)}</span>
      </summary>
      {!c.exact && <p className="muted">Too long to compare line by line: the changed part is shown removed, then added.</p>}
      <ol className="diff-lines" aria-label={`Changes in ${c.path}`}>
        {hunks(c.lines).map((h, k) =>
          "gap" in h ? (
            <li key={k} className="gap">
              … {count(h.gap, "unchanged line")}
            </li>
          ) : h.op === "add" ? (
            <li key={k} className="add">
              <span className="sr-only">added: </span>
              <span className="mark" aria-hidden="true">
                +
              </span>
              <ins>{h.text}</ins>
            </li>
          ) : h.op === "del" ? (
            <li key={k} className="del">
              <span className="sr-only">removed: </span>
              <span className="mark" aria-hidden="true">
                −
              </span>
              <del>{h.text}</del>
            </li>
          ) : (
            <li key={k} className="same">
              <span className="mark" aria-hidden="true">
                {" "}
              </span>
              <span>{h.text}</span>
            </li>
          ),
        )}
      </ol>
    </details>
  );
}

function Files({ rev, state, openPlan }: { rev: number; state: RevisionState | null; openPlan: boolean }) {
  const heading = useId();
  return (
    <section className="draft-files" aria-labelledby={heading}>
      <h4 id={heading}>Files of revision {rev}</h4>
      {state === null ? (
        <p className="muted">Loading the files…</p>
      ) : state.error ? (
        <p className="warn">The files could not be read: {state.error}</p>
      ) : (
        bundleOrder(state.data?.files ?? []).map((f, i) => (
          // The plan opens when there is no diff above to read first.
          <details key={f.path} className="draft-file" open={i === 0 && openPlan}>
            <summary>
              <code>{f.path}</code>
              {i === 0 && <span className="muted"> · the plan</span>}
            </summary>
            <pre className="draft-text">{f.text}</pre>
          </details>
        ))
      )}
    </section>
  );
}
