import { type DragEvent, useState } from "react";
import type { IssueRef, IssueSummary, Me, Overview, PairingInfo, QueuedIssue, SessionState, SessionStatus } from "../../protocol.ts";
import { getIssueDrag, hasIssueDrag } from "../../shared/dnd.ts";
import { issueKey, relative } from "../../shared/format.ts";

/**
 * A working session whose last signal is older than this is shown as unknown, never as working
 * (the trust check). The server's default for `session_stale_secs`.
 */
export const SESSION_STALE_SECS = 120;

/** A companion that has not reported for this long is stale (AGENTS.md, "never silent": N defaults to 10 minutes). */
export const COMPANION_STALE_MS = 10 * 60_000;

/** How fresh a companion's signal is: 1 just now, 0 at the stale threshold or never seen. */
export function freshness(lastSeen: string | null | undefined, now = Date.now(), staleMs = COMPANION_STALE_MS): number {
  if (!lastSeen) {
    return 0;
  }
  return Math.max(0, Math.min(1, 1 - (now - Date.parse(lastSeen)) / staleMs));
}

/** What to show for a session: its status, or unknown when the signal stopped flowing. */
export function shownStatus(s: SessionState, now = Date.now(), staleSecs = SESSION_STALE_SECS): SessionStatus {
  return s.status === "working" && now - Date.parse(s.last_seen) > staleSecs * 1000 ? "unknown" : s.status;
}

/** Lanes in the order they are shown: your own first, then by user and host; revoked last. */
export function orderLanes(companions: PairingInfo[], me: Me | null): PairingInfo[] {
  const rank = (c: PairingInfo) => (c.revoked_at ? 2 : me && c.user === me.login ? 0 : 1);
  return [...companions].sort((a, b) => rank(a) - rank(b) || a.user.localeCompare(b.user) || a.hostname.localeCompare(b.hostname));
}

const sameIssue = (a: IssueRef, b: IssueRef) => a.repo === b.repo && a.number === b.number;

/** A queued entry being dragged within its lane, to reorder it. */
const ENTRY_MIME = "application/x-rubato-queue-entry";

export interface LaneActions {
  /** Queue an issue on one of the caller's lanes. */
  onQueue: (issue: IssueRef, companionId: string) => void;
  /** The lane's full new order. */
  onReorder: (companionId: string, issues: IssueRef[]) => void;
  onUnqueue: (issue: IssueRef, companionId: string) => void;
  /** Launch on that lane's companion. Only the issue and the companion are sent; the server resolves the caps. */
  onStart: (issue: IssueRef, companionId: string) => void;
  onRevoke: (companionId: string) => void;
}

interface LaneProps extends LaneActions {
  companion: PairingInfo;
  own: boolean;
  sessions: SessionState[];
  queue: QueuedIssue[];
  issues: IssueSummary[];
  titles: Map<string, string>;
}

function Lane({ companion: c, own, sessions, queue, issues, titles, onQueue, onReorder, onUnqueue, onStart, onRevoke }: LaneProps) {
  // "yes": an issue is over this lane and it will take it; "no": it is not the caller's lane.
  const [over, setOver] = useState<"yes" | "no" | null>(null);
  const takes = own && !c.revoked_at;
  const order = queue.map((q) => q.issue);

  const dragOver = (e: DragEvent<HTMLElement>) => {
    if (!hasIssueDrag(e)) {
      return;
    }
    if (takes) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setOver("yes");
    } else {
      // No preventDefault: the browser shows the lane takes no drop, and none is delivered.
      e.dataTransfer.dropEffect = "none";
      setOver("no");
    }
  };
  const drop = (e: DragEvent<HTMLElement>) => {
    setOver(null);
    const issue = getIssueDrag(e);
    if (!takes || !issue) {
      return;
    }
    e.preventDefault();
    onQueue(issue, c.companion_id);
  };
  const move = (from: number, to: number) => {
    if (to < 0 || to >= order.length || from === to) {
      return;
    }
    const next = [...order];
    const [it] = next.splice(from, 1);
    next.splice(to, 0, it as IssueRef);
    onReorder(c.companion_id, next);
  };
  const state = c.revoked_at ? "revoked" : c.connected ? "connected" : c.last_seen ? `last seen ${relative(c.last_seen)}` : "never connected";

  return (
    <li
      className={["lane", own ? "own" : "", over === "yes" ? "drop-over" : "", over === "no" ? "drop-refused" : ""].filter(Boolean).join(" ")}
      data-testid={`lane-${c.companion_id}`}
      aria-label={`${own ? "Your companion" : `${c.user}'s companion`} ${c.hostname}`}
      onDragEnter={dragOver}
      onDragOver={dragOver}
      onDragLeave={(e) => {
        if (!(e.relatedTarget instanceof Node) || !e.currentTarget.contains(e.relatedTarget)) {
          setOver(null);
        }
      }}
      onDrop={drop}
    >
      <header data-testid={`companion-${c.user}`}>
        <span>
          <i className={c.connected && !c.revoked_at ? "led live" : "led"} aria-hidden="true" /> <strong>{c.hostname}</strong> <span className="muted">{own ? "yours" : c.user}</span>
          <div className={c.connected && !c.revoked_at ? "muted" : "warn"}>{state}</div>
          {!c.revoked_at && (
            <span className="fresh" data-testid={`fresh-${c.companion_id}`} data-share={(c.connected ? 1 : freshness(c.last_seen)).toFixed(2)} aria-hidden="true">
              <b className={c.connected ? "live" : freshness(c.last_seen) > 0 ? "fading" : "stale"} style={{ width: `${Math.round((c.connected ? 1 : freshness(c.last_seen)) * 100)}%` }} />
            </span>
          )}
        </span>
        {own && !c.revoked_at && (
          <button type="button" onClick={() => onRevoke(c.companion_id)}>
            revoke
          </button>
        )}
      </header>
      {over === "no" && <p className="warn" role="status">{c.revoked_at ? "This companion is revoked: it takes no issues." : `This is ${c.user}'s companion: you can queue issues only on your own.`}</p>}
      {over === "yes" && <p className="drop-hint">Drop to queue here</p>}

      <h4>Sessions</h4>
      {sessions.length === 0 ? (
        <p className="muted">No session reported.</p>
      ) : (
        <ul className="plain">
          {sessions.map((s, i) => {
            const status = shownStatus(s);
            return (
              <li key={s.session_id ?? i} className="lane-row" data-testid="lane-session">
                <span>
                  {s.issue ? issueKey(s.issue) : <span className="warn">no issue{s.branch ? ` (branch ${s.branch})` : ""}</span>}
                  <div className="muted">
                    seen {relative(s.last_seen)}
                    {s.spend_usd !== undefined && s.spend_usd !== null ? ` · $${s.spend_usd.toFixed(2)}` : ""}
                  </div>
                </span>
                <span className={`badge ${status}`} data-led={status === "working" ? "breathe" : undefined} title={status === "unknown" && s.status === "working" ? "It reported working, but no signal has arrived for a while" : undefined}>
                  {status}
                </span>
              </li>
            );
          })}
        </ul>
      )}

      <h4>Queue{queue.length ? ` (${queue.length})` : ""}</h4>
      {queue.length === 0 ? (
        <p className="muted">{takes ? "Nothing queued. Drag an issue here, or use Queue on… on an issue." : "Nothing queued."}</p>
      ) : (
        <ol className="plain">
          {queue.map((q, i) => {
            const key = issueKey(q.issue);
            const tracked = issues.find((x) => sameIssue(x.issue, q.issue));
            const pinned = (tracked?.pins.length ?? 0) > 0;
            const why = !pinned ? "pin first" : !c.connected ? "companion not connected" : null;
            return (
              <li
                key={key}
                className="lane-row queue-entry"
                data-testid={`queued-${q.issue.number}`}
                draggable={takes}
                onDragStart={(e) => {
                  e.dataTransfer.setData(ENTRY_MIME, String(i));
                  e.dataTransfer.effectAllowed = "move";
                }}
                onDragOver={(e) => {
                  if (takes && Array.from(e.dataTransfer.types).includes(ENTRY_MIME)) {
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "move";
                  }
                }}
                onDrop={(e) => {
                  const from = e.dataTransfer.getData(ENTRY_MIME);
                  if (takes && from !== "") {
                    e.preventDefault();
                    e.stopPropagation();
                    move(Number(from), i);
                  }
                }}
              >
                <span>
                  {key} <span className="muted">{tracked?.title ?? titles.get(key) ?? ""}</span>
                  <div className="muted">
                    queued by {q.queued_by} {relative(q.queued_at)}
                    {why ? ` · ${why}` : ""}
                  </div>
                  {q.last_start_error && <div className="warn">did not start: {q.last_start_error}</div>}
                </span>
                {takes && (
                  <span className="row-end">
                    <button type="button" aria-label={`Move ${key} up`} disabled={i === 0} onClick={() => move(i, i - 1)}>
                      ↑
                    </button>
                    <button type="button" aria-label={`Move ${key} down`} disabled={i === queue.length - 1} onClick={() => move(i, i + 1)}>
                      ↓
                    </button>
                    <button type="button" aria-label={`Remove ${key} from the queue`} onClick={() => onUnqueue(q.issue, c.companion_id)}>
                      Remove
                    </button>
                    <button type="button" className="primary" aria-label={`Start ${key}`} disabled={why !== null} title={why ?? `Start a session for ${key} on ${c.hostname}`} onClick={() => onStart(q.issue, c.companion_id)}>
                      Start
                    </button>
                  </span>
                )}
              </li>
            );
          })}
        </ol>
      )}
    </li>
  );
}

interface Props extends LaneActions {
  overview: Overview;
  me: Me | null;
}

/**
 * Each companion is a lane (design S13 to S16): who it belongs to, whether its signal flows,
 * the sessions it reports and the issues its owner intends to run there. Your own lanes come
 * first and take drops; everyone else's are read-only.
 */
export function Lanes({ overview, me, ...actions }: Props) {
  const lanes = orderLanes(overview.companions, me);
  const sessions = [...overview.issues.flatMap((i) => i.sessions ?? []), ...(overview.unpinned_sessions ?? [])];
  const titles = new Map((overview.backlog?.issues ?? []).map((i) => [issueKey(i.issue), i.title]));
  if (lanes.length === 0) {
    return <p className="muted">No companions paired yet.</p>;
  }
  return (
    <ul className="plain lanes" data-testid="lanes">
      {lanes.map((c) => (
        <Lane
          key={c.companion_id}
          companion={c}
          own={Boolean(me) && c.user === me?.login}
          sessions={sessions.filter((s) => s.companion_id === c.companion_id)}
          queue={(overview.queue ?? []).filter((q) => q.companion_id === c.companion_id).sort((a, b) => a.position - b.position)}
          issues={overview.issues}
          titles={titles}
          {...actions}
        />
      ))}
    </ul>
  );
}
