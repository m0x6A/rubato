import { useCallback, useId, useMemo, useRef, useState } from "react";
import type { IssueRef, IssueSummary, Limits, Me, Overview, RoomMessage, SeamNotice } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import { issueKey, relative } from "../../shared/format.ts";
import { useWebResource } from "../live/index.ts";
import { type RoomKey, roomApi, roomFailureText } from "./api.ts";

/** `Limits.room_message_max_chars`'s default, for an overview that does not carry the limits. */
export const ROOM_MAX_CHARS = 8000;

/** The room of an issue's plan (ADR 0057): its draft's branch, else its plan's; none without either. */
export function roomFor(issue: IssueSummary): RoomKey | null {
  const branch = issue.draft?.branch ?? issue.plan?.git_ref ?? null;
  return branch ? { repo: issue.issue.repo, number: issue.issue.number, branch } : null;
}

/** The longest message the room takes: the overview's limit when it carries one, else 8000. */
export function roomLimit(overview: Overview): number {
  const n = (overview as Overview & { limits?: Partial<Limits> | null }).limits?.room_message_max_chars;
  return typeof n === "number" && n > 0 ? n : ROOM_MAX_CHARS;
}

/** Characters as the server counts them (Unicode scalar values), not UTF-16 units. */
const chars = (s: string) => [...s].length;
const count = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

interface Props {
  issue: IssueSummary;
  me: Me | null;
  maxChars: number;
  /** Open another issue on this page; offered for the issues `canOpen` accepts. */
  onOpenIssue?: ((issue: IssueRef) => void) | undefined;
  canOpen?: ((issue: IssueRef) => boolean) | undefined;
  /** Show a seam on the map. */
  onShowSeam?: ((seam: string) => void) | undefined;
}

/**
 * The plan's room (ADR 0057): the team discusses the plan here, and Rubato posts a notice when
 * another plan touches one of its seams. Everyone reads it; anyone logged in posts. Agents come
 * with call-in (PR 11); their messages already read as theirs.
 */
export function RoomSection(props: Props) {
  const room = roomFor(props.issue);
  // Keyed by the room, so another issue's room starts with its own list and box.
  return room ? <Room key={`${room.repo}#${room.number}:${room.branch}`} room={room} {...props} /> : null;
}

function Room({ room, me, maxChars, onOpenIssue, canOpen, onShowSeam }: Props & { room: RoomKey }) {
  const heading = useId();
  // The server says `updated` for this room when a message is added.
  const { data, error, reload } = useWebResource({ kind: "room", ...room }, () => roomApi.get(room), roomFailureText);
  const [posted, setPosted] = useState<RoomMessage[]>([]);
  const [sending, setSending] = useState(false);
  const [said, setSaid] = useState<{ text: string; failed: boolean } | null>(null);
  // One post at a time, also for two key presses inside one render.
  const busy = useRef(false);

  // A post shows at once; the next read of the room carries it too, so it is kept once.
  const messages = useMemo(() => {
    const read = data?.messages ?? [];
    const have = new Set(read.map((m) => m.id));
    return [...read, ...posted.filter((m) => !have.has(m.id))];
  }, [data, posted]);

  const send = useCallback(
    async (text: string): Promise<boolean> => {
      if (busy.current) {
        return false;
      }
      busy.current = true;
      setSending(true);
      setSaid(null);
      try {
        const added = await roomApi.post({ ...room, text });
        if (added) {
          setPosted((p) => [...p, added]);
        }
        setSaid({ text: "Posted to the room.", failed: false });
        void reload();
        return true;
      } catch (e) {
        // A 401 raised needs-login in `request`; the login page follows.
        if (!(e instanceof ApiError && e.status === 401)) {
          setSaid({ text: `Not posted: ${roomFailureText(e)}`, failed: true });
        }
        return false;
      } finally {
        busy.current = false;
        setSending(false);
      }
    },
    [room, reload],
  );

  return (
    <section className="room" aria-labelledby={heading} data-testid="room">
      <h3 id={heading} style={{ marginTop: 12 }}>
        Room · <code>{room.branch}</code>
      </h3>
      <p className="muted">The plan's room: the whole team reads it.</p>
      {error && <p className="warn">The room could not be read: {error}</p>}
      {data === null && messages.length === 0 ? (
        !error && <p className="muted">Loading the room…</p>
      ) : messages.length === 0 ? (
        <p className="muted">No messages yet.</p>
      ) : (
        <ol className="plain room-log" aria-label="Messages in the room">
          {messages.map((m) => (
            <Message key={m.id} m={m} onOpenIssue={onOpenIssue} canOpen={canOpen} onShowSeam={onShowSeam} />
          ))}
        </ol>
      )}
      {me ? <Compose sending={sending} maxChars={maxChars} onSend={send} /> : <p className="muted">Log in to post in the room.</p>}
      <p className="muted room-said" role="status">
        {said && !said.failed ? said.text : ""}
      </p>
      {said?.failed && (
        <p className="warn" role="alert">
          {said.text}
        </p>
      )}
    </section>
  );
}

type Links = Pick<Props, "onOpenIssue" | "canOpen" | "onShowSeam">;

function Message({ m, ...links }: { m: RoomMessage } & Links) {
  const who = m.author_kind === "system" ? "Rubato" : m.author;
  return (
    <li className={`room-msg ${m.author_kind}`} data-testid={`room-message-${m.id}`}>
      <div className="room-meta">
        <strong className="room-author">{who}</strong>
        {m.author_kind !== "person" && <span className="room-kind">{m.author_kind === "agent" ? "agent" : "notice"}</span>}
        <time className="muted" dateTime={m.at} title={new Date(m.at).toLocaleString()}>
          {relative(m.at)}
        </time>
      </div>
      <p className="room-text">{m.text}</p>
      {m.notice && <Notice n={m.notice} {...links} />}
    </li>
  );
}

/** What a seam notice is about: the seam both plans touch, and the other plan's issue and branch. */
function Notice({ n, onOpenIssue, canOpen, onShowSeam }: { n: SeamNotice } & Links) {
  const other = issueKey(n.other);
  return (
    <p className="room-notice">
      Both plans touch{" "}
      {onShowSeam ? (
        <button type="button" className="link" aria-label={`seam ${n.seam}`} onClick={() => onShowSeam(n.seam)}>
          {n.seam}
        </button>
      ) : (
        <code>{n.seam}</code>
      )}
      . The other plan:{" "}
      {onOpenIssue && canOpen?.(n.other) ? (
        <button type="button" className="link" onClick={() => onOpenIssue(n.other)}>
          {other}
        </button>
      ) : (
        <a href={`https://github.com/${n.other.repo}/issues/${n.other.number}`} target="_blank" rel="noreferrer">
          {other}
        </a>
      )}{" "}
      on <code>{n.other_branch}</code>.
    </p>
  );
}

function Compose({ sending, maxChars, onSend }: { sending: boolean; maxChars: number; onSend: (text: string) => Promise<boolean> }) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  const id = useId();
  const n = chars(text);
  const over = n - maxChars;
  const can = !sending && text.trim() !== "" && over <= 0;
  const submit = async () => {
    if (!can) {
      return;
    }
    if (await onSend(text)) {
      setText("");
    }
    // The button was disabled while posting; put the person back where they type.
    box.current?.focus();
  };
  return (
    <form
      className="room-compose"
      aria-busy={sending}
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <label htmlFor={`${id}-text`}>Message to the room</label>
      <textarea
        id={`${id}-text`}
        ref={box}
        rows={3}
        value={text}
        readOnly={sending}
        aria-describedby={`${id}-hint ${id}-count`}
        aria-invalid={over > 0 ? true : undefined}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void submit();
          }
        }}
      />
      <div className="room-compose-row">
        <span id={`${id}-hint`} className="muted">
          The whole team reads it · Ctrl+Enter posts
        </span>
        <button type="submit" className="primary" disabled={!can}>
          {sending ? "Posting…" : "Post"}
        </button>
      </div>
      <p id={`${id}-count`} className={over > 0 ? "warn" : "muted"}>
        {over > 0 ? `${count(over, "character")} over the limit of ${maxChars}` : n > maxChars * 0.9 ? `${n} of ${maxChars} characters` : ""}
      </p>
    </form>
  );
}
