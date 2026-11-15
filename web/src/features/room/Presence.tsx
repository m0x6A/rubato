import { useEffect, useId, useRef, useState } from "react";
import type { LeftReason, Me, RoomAgent, RoomRequest, SessionState } from "../../protocol.ts";
import { relative } from "../../shared/format.ts";
import { type RoomKey, roomApi } from "./api.ts";

const same = (a: string | null | undefined, b: string | null | undefined) => (a ?? "").toLowerCase() === (b ?? "").toLowerCase() && a != null;

/** `Limits.room_agent_idle_secs`'s default: an agent not addressed this long leaves. */
const IDLE_MINUTES = 30;

const LEFT: Record<LeftReason, string> = {
  dismissed: "dismissed",
  timed_out: `not addressed for ${IDLE_MINUTES} minutes`,
  session_ended: "its session ended",
};

const STATE: Record<RoomRequest["state"], string> = {
  pending: "pending",
  accepted: "accepted",
  declined: "declined",
  withdrawn: "withdrawn",
};

function Since({ at }: { at: string }) {
  return (
    <time dateTime={at} title={new Date(at).toLocaleString()}>
      {relative(at)}
    </time>
  );
}

export interface PresenceProps {
  room: RoomKey;
  me: Me | null;
  agents: RoomAgent[];
  requests: RoomRequest[];
  /** This issue's sessions launched from Rubato on the room's branch. */
  sessions: SessionState[];
  /** The plan is on the branch: a turn runs in a read-only fork, not in the working session. */
  planOnBranch: boolean;
  acting: boolean;
  /** Run one action; the room says what it did, or why not, and is read again. */
  act: (run: () => Promise<string>) => Promise<boolean>;
}

/**
 * Who is in the room (ADR 0057): the agents called in, by whom and since when, who left and why,
 * the requests to call one in, and the call-in itself. An agent is never in a room by default:
 * its owner calls it in, and anyone else asks the owner.
 */
export function Presence({ room, me, agents, requests, sessions, planOnBranch, acting, act }: PresenceProps) {
  const heading = useId();
  const headingRef = useRef<HTMLHeadingElement>(null);
  const present = agents.filter((a) => !a.left_at);
  const left = agents.filter((a) => a.left_at);
  const inRoom = new Set(present.map((a) => a.launch_id));
  const pending = new Set(requests.filter((r) => r.state === "pending").map((r) => r.launch_id));
  // One call-in per launch on this branch whose agent is not in yet.
  const callable = me
    ? [...new Map(sessions.filter((s) => s.launch_id && !inRoom.has(s.launch_id)).map((s) => [s.launch_id as string, s])).values()].filter((s) => same(s.user, me.login) || !pending.has(s.launch_id as string))
    : [];
  // After an action its button may leave (joined, answered, dismissed): focus goes to the heading.
  const run: PresenceProps["act"] = async (fn) => {
    const ok = await act(fn);
    if (ok) {
      headingRef.current?.focus();
    }
    return ok;
  };

  return (
    <section className="room-presence" aria-labelledby={heading}>
      <h4 id={heading} ref={headingRef} tabIndex={-1}>
        Agents
      </h4>
      {present.length === 0 ? (
        <p className="muted">No agent is in the room. An agent joins only when its owner calls it in.</p>
      ) : (
        <ul className="plain room-agents" aria-label="In the room">
          {present.map((a) => (
            <li key={a.handle}>
              <strong>{a.handle}</strong>{" "}
              <span className="muted">
                called in by {a.called_by}, <Since at={a.joined_at} />
              </span>
              {me && (same(me.login, a.owner) || same(me.login, a.called_by)) && (
                <>
                  {" "}
                  <button type="button" aria-label={`Dismiss ${a.handle}`} disabled={acting} onClick={() => void run(async () => (await roomApi.dismiss({ ...room, handle: a.handle }), `${a.handle} left the room.`))}>
                    Dismiss
                  </button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      {left.length > 0 && (
        <ul className="plain room-agents left" aria-label="Left the room">
          {left.map((a) => (
            <li key={`${a.handle}-${a.left_at}`} className="muted">
              {a.handle} left <Since at={a.left_at as string} />: {a.left_reason ? LEFT[a.left_reason] : "left"}
            </li>
          ))}
        </ul>
      )}
      {requests.length > 0 && (
        <ul className="plain room-requests" aria-label="Requests to call an agent in">
          {requests.map((r) => (
            <Request key={r.id} r={r} me={me} acting={acting} act={run} />
          ))}
        </ul>
      )}
      {callable.map((s) => (
        <CallIn key={s.launch_id} room={room} me={me as Me} session={s} planOnBranch={planOnBranch} acting={acting} act={run} />
      ))}
    </section>
  );
}

function Request({ r, me, acting, act }: { r: RoomRequest; me: Me | null; acting: boolean; act: PresenceProps["act"] }) {
  const owner = me !== null && same(me.login, r.owner);
  const asker = me !== null && same(me.login, r.requested_by);
  const open = r.state === "pending";
  const answer = (a: "accept" | "decline" | "withdraw", said: string) => void act(async () => (await roomApi.answer(r.id, { answer: a }), said));
  return (
    <li data-testid={`room-request-${r.id}`} className={`room-request ${r.state}`}>
      {owner && open ? `${r.requested_by} asks you to call your agent in` : `${r.requested_by} asked ${r.owner} to call their agent in`}{" "}
      <span className="muted">
        · <Since at={r.answered_at ?? r.at} /> · <span className="room-request-state">{STATE[r.state]}</span>
      </span>
      {open && owner && (
        <>
          {" "}
          <button type="button" className="primary" disabled={acting} onClick={() => answer("accept", "Accepted: your agent joins the room.")}>
            Accept
          </button>{" "}
          <button type="button" disabled={acting} onClick={() => answer("decline", "Declined.")}>
            Decline
          </button>
        </>
      )}
      {open && asker && !owner && (
        <>
          {" "}
          <button type="button" disabled={acting} onClick={() => answer("withdraw", "Withdrawn.")}>
            Withdraw
          </button>
        </>
      )}
    </li>
  );
}

/** What calling this agent in costs, in the words of design §D8. */
function estimate(session: SessionState, mine: boolean, planOnBranch: boolean): string {
  const whose = mine ? "your" : `${session.user}'s`;
  if (!planOnBranch) {
    return `Its turns are typed into ${whose} working session when it next waits for input, like a message.`;
  }
  const spend = session.spend_usd;
  return typeof spend === "number"
    ? `A first turn re-reads the session uncached: about $${spend.toFixed(2)}, the session's spend so far, on ${whose} spend cap. Later turns cost less.`
    : `A first turn re-reads the session uncached; its spend so far is not known. It counts on ${whose} spend cap.`;
}

/** "Call my agent in" for the owner, "Ask <owner> …" for anyone else; both confirm after the estimate. */
function CallIn({ room, me, session, planOnBranch, acting, act }: { room: RoomKey; me: Me; session: SessionState; planOnBranch: boolean; acting: boolean; act: PresenceProps["act"] }) {
  const mine = same(me.login, session.user);
  const owner = session.user;
  const [open, setOpen] = useState(false);
  // Cancel puts focus back on the button that opened the estimate, once it is drawn again.
  const [refocus, setRefocus] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const confirm = useRef<HTMLButtonElement>(null);
  const heading = useId();
  useEffect(() => {
    if (open) {
      confirm.current?.focus();
    } else if (refocus) {
      trigger.current?.focus();
      setRefocus(false);
    }
  }, [open, refocus]);
  const label = mine ? "Call my agent in" : `Ask ${owner} to call their agent in`;
  const call = async () => {
    const ok = await act(async () => {
      const r = await roomApi.callIn({ ...room, launch_id: session.launch_id as string });
      return r.outcome === "joined" ? `${r.agent.handle} joined the room.` : `Asked ${owner}; the request is pending.`;
    });
    if (ok) {
      setOpen(false);
    }
  };
  if (!open) {
    return (
      <p className="room-callin-open">
        <button type="button" ref={trigger} disabled={acting} onClick={() => setOpen(true)}>
          {label}
        </button>
      </p>
    );
  }
  return (
    <div className="room-callin" role="group" aria-labelledby={heading}>
      <p id={heading}>
        <strong>{mine ? "Call your agent in?" : `Ask ${owner} to call their agent in?`}</strong>
      </p>
      {!mine && <p>{owner} decides. If they accept:</p>}
      <p>{estimate(session, mine, planOnBranch)}</p>
      <p className="muted">It reads only what the room says after it joins, as quoted discussion, and answers when a message names it.</p>
      <p>
        <button type="button" className="primary" ref={confirm} disabled={acting} onClick={() => void call()}>
          {mine ? "Call in" : "Ask"}
        </button>{" "}
        <button
          type="button"
          onClick={() => {
            setOpen(false);
            setRefocus(true);
          }}
        >
          Cancel
        </button>
      </p>
    </div>
  );
}
