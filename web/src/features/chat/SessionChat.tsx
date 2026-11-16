import { useCallback, useId, useMemo, useRef, useState } from "react";
import type { LaunchLog, Me, SessionLogEntry, SessionState } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import { relative } from "../../shared/format.ts";
import { chatApi, failureWords } from "./api.ts";
import { stateText } from "./states.ts";
import { KIND_LABEL, type Turn, entryDomId, messageDomId, turnsByMessage } from "./thread.ts";
import type { SessionMessageView } from "./types.ts";
import { useLaunchResource } from "./useLaunchResource.ts";

const sameLogin = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
/** A 401 raised needs-login in `request`; the login page follows, so nothing more is said here. */
const needsLogin = (e: unknown) => e instanceof ApiError && e.status === 401;

const HOOKS_LOST = "The companion restarted and lost this session's hooks: relaunch to chat";
const UPGRADE = "Upgrade the companion to chat with this session";

interface Props {
  /** One of an issue's sessions. Nothing shows for a session Rubato did not launch. */
  session: SessionState;
  me: Me | null;
}

/**
 * One launched session's conversation: its prompts and replies (the session log, ADR 0052) and
 * the messages sent to it from Rubato (ADR 0051). The whole team reads both; only the launch's
 * owner writes, and the server and the companion check that again. Your own session opens
 * expanded; someone else's starts collapsed, so opening an issue reads nothing extra.
 */
export function SessionChat({ session, me }: Props) {
  const [open, setOpen] = useState(me !== null && sameLogin(me.login, session.user));
  const panel = useId();
  if (!session.launch_id) {
    return null;
  }
  return (
    <div className="session-chat">
      <button type="button" className="chat-toggle" aria-expanded={open} aria-controls={panel} onClick={() => setOpen((o) => !o)}>
        <span aria-hidden="true">{open ? "▾" : "▸"}</span> Conversation
      </button>
      <div id={panel}>{open && <Conversation launchId={session.launch_id} hooksLost={session.hooks_lost === true} me={me} />}</div>
    </div>
  );
}

function Conversation({ launchId, hooksLost, me }: { launchId: string; hooksLost: boolean; me: Me | null }) {
  // The server says `updated` when a message of this launch is added or changes state, and
  // when an entry of its log is stored; each is read again on its own.
  const { data, error: loadError, reload: load } = useLaunchResource("launch_messages", launchId, chatApi.messages);
  const log = useLaunchResource("launch_log", launchId, chatApi.log);
  const [sending, setSending] = useState(false);
  const [said, setSaid] = useState<{ text: string; failed: boolean } | null>(null);
  // One send at a time, also for two key presses inside one render.
  const busy = useRef(false);
  const turns = useMemo(() => turnsByMessage(log.data?.entries ?? []), [log.data]);
  const authors = useMemo(() => new Map((data?.messages ?? []).map((m) => [m.id, m.author])), [data]);

  const live = data?.companion_live ?? true;
  /** Posts `text` as a new message. True when the server took it. */
  const send = useCallback(
    async (text: string): Promise<boolean> => {
      if (busy.current) {
        return false;
      }
      busy.current = true;
      setSending(true);
      setSaid(null);
      try {
        const added = await chatApi.send(launchId, text);
        setSaid({ text: `Message added: ${stateText(added, live)}`, failed: false });
        void load();
        return true;
      } catch (e) {
        if (!needsLogin(e)) {
          setSaid({ text: `Not sent: ${failureWords(e)}`, failed: true });
        }
        return false;
      } finally {
        busy.current = false;
        setSending(false);
      }
    },
    [launchId, live, load],
  );

  /** Stops the session (ADR 0062): asks once more first, since a stop cannot be taken back. */
  const [confirmStop, setConfirmStop] = useState(false);
  const stop = useCallback(async () => {
    if (!confirmStop) {
      setConfirmStop(true);
      return;
    }
    setConfirmStop(false);
    setSaid(null);
    try {
      await chatApi.stop(launchId);
      setSaid({ text: "Stop sent: the running turn is killed and the session ends", failed: false });
      void load();
    } catch (e) {
      if (!needsLogin(e)) {
        setSaid({ text: `Not stopped: ${failureWords(e)}`, failed: true });
      }
    }
  }, [confirmStop, launchId, load]);

  const isOwner = me !== null && data !== null && sameLogin(me.login, data.owner);
  const cannot = hooksLost ? HOOKS_LOST : data && !data.companion_can_receive ? UPGRADE : null;
  const canSend = isOwner && cannot === null;

  return (
    <div className="chat" data-testid={`chat-${launchId}`}>
      <h4>Prompts and replies</h4>
      <Thread launchId={launchId} log={log.data} error={log.error} authors={authors} />
      <h4>Messages from Rubato</h4>
      {loadError && <p className="warn">Messages could not be read: {loadError}</p>}
      {data === null ? (
        !loadError && <p className="muted">Loading messages…</p>
      ) : data.messages.length === 0 ? (
        <p className="muted">No messages yet.</p>
      ) : (
        <ol className="plain chat-log" aria-label="Messages to this session">
          {data.messages.map((m) => (
            <Message key={m.id} launchId={launchId} m={m} live={data.companion_live} turn={turns.get(m.id)} onSendAgain={canSend ? () => void send(m.text) : undefined} />
          ))}
        </ol>
      )}
      {data && !isOwner && <p className="muted">Only {data.owner} can message this session.</p>}
      {isOwner && cannot && <p className="warn">{cannot}</p>}
      {canSend && <Compose sending={sending} onSend={send} />}
      {isOwner && (
        <button type="button" className={confirmStop ? "danger" : undefined} aria-label={confirmStop ? "Really stop this session" : "Stop this session"} disabled={!data?.companion_live} title={data?.companion_live ? "Kill the running turn and end the session" : "The companion is not connected"} onClick={() => void stop()}>
          {confirmStop ? "Really stop?" : "Stop session"}
        </button>
      )}
      {isOwner && (
        <p className="muted chat-said" role="status">
          {said && !said.failed ? said.text : ""}
        </p>
      )}
      {said?.failed && (
        <p className="warn" role="alert">
          {said.text}
        </p>
      )}
    </div>
  );
}

/** The prompts and replies of the launch, oldest first: the conversation as the team reads it. */
function Thread({ launchId, log, error, authors }: { launchId: string; log: LaunchLog | null; error: string | null; authors: Map<string, string> }) {
  if (error) {
    return <p className="warn">The conversation could not be read: {error}</p>;
  }
  if (log === null) {
    return <p className="muted">Loading the conversation…</p>;
  }
  if (log.entries.length === 0) {
    return <p className="muted">Nothing has been said in this session yet.</p>;
  }
  return (
    <ol className="plain chat-log session-log" aria-label="Prompts and replies in this session">
      {log.entries.map((e, i) => (
        <Entry key={entryDomId(launchId, e)} launchId={launchId} e={e} newSession={i > 0 && log.entries[i - 1]?.session_id !== e.session_id} author={e.message_id ? authors.get(e.message_id) : undefined} />
      ))}
    </ol>
  );
}

function Entry({ launchId, e, newSession, author }: { launchId: string; e: SessionLogEntry; newSession: boolean; author: string | undefined }) {
  const fromRubato = e.role === "user" && e.message_id;
  return (
    // A target of "see the reply": it takes focus, so following the link moves the reader here.
    <li className={`log-entry ${e.role} ${e.kind}`} id={entryDomId(launchId, e)} tabIndex={-1}>
      {newSession && <p className="muted log-new-session">A new Claude Code session continues this launch.</p>}
      <div className="chat-meta">
        {e.role === "user" ? (
          <>
            <strong>Prompt</strong>
            {fromRubato ? (
              <span className="log-source">
                {author ? `from Rubato, sent by ${author}` : "from Rubato"}
                {author && e.message_id && (
                  <>
                    {" · "}
                    <a href={`#${messageDomId(launchId, e.message_id)}`}>the message</a>
                  </>
                )}
              </span>
            ) : (
              <span className="log-source">from the terminal</span>
            )}
          </>
        ) : (
          <>
            <strong>Claude</strong>
            <span className="log-kind">{KIND_LABEL[e.kind]}</span>
          </>
        )}
        <time className="muted" dateTime={e.at} title={new Date(e.at).toLocaleString()}>
          {relative(e.at)}
        </time>
      </div>
      <p className="log-text">{e.text}</p>
      {e.truncated && <p className="muted log-cut">(cut)</p>}
      {e.mixed && <p className="warn log-mixed">typed together with text already in the terminal</p>}
    </li>
  );
}

function Message({ launchId, m, live, turn, onSendAgain }: { launchId: string; m: SessionMessageView; live: boolean; turn: Turn | undefined; onSendAgain: (() => void) | undefined }) {
  return (
    // A target of "the message": it takes focus, so following the link moves the reader here.
    <li className="chat-msg" id={messageDomId(launchId, m.id)} tabIndex={-1} data-testid={`message-${m.id}`}>
      <div className="chat-meta">
        <strong>{m.author}</strong>
        {/* Sent for its owner by the owner's own lead agent (ADR 0072). */}
        {m.via_launch && (
          <span className="badge" data-testid={`via-lead-${m.id}`} title={`Sent by ${m.author}'s lead agent (launch ${m.via_launch.slice(0, 8)})`}>
            via lead agent
          </span>
        )}
        <time className="muted" dateTime={m.created_at} title={new Date(m.created_at).toLocaleString()}>
          {relative(m.created_at)}
        </time>
        <span className={`msg-state ${m.state}`}>{stateText(m, live)}</span>
        {turn?.reply ? (
          <>
            <span className="msg-answered">answered</span>
            <a href={`#${entryDomId(launchId, turn.reply)}`}>see the reply</a>
          </>
        ) : (
          turn && <a href={`#${entryDomId(launchId, turn.prompt)}`}>see it in the conversation</a>
        )}
      </div>
      <p className="chat-text">{m.text}</p>
      {m.state === "delivery_unknown" && onSendAgain && (
        <p className="muted chat-unknown">
          The companion never said whether it was typed, so it is not typed again.{" "}
          <button type="button" onClick={onSendAgain}>
            Send again
          </button>
        </p>
      )}
    </li>
  );
}

function Compose({ sending, onSend }: { sending: boolean; onSend: (text: string) => Promise<boolean> }) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  const id = useId();
  const blank = text.trim() === "";
  const submit = async () => {
    if (sending || blank) {
      return;
    }
    if (await onSend(text)) {
      setText("");
    }
    // The Send button was disabled while sending; put the person back where they type.
    box.current?.focus();
  };
  return (
    <form
      className="chat-compose"
      aria-busy={sending}
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <label htmlFor={`${id}-text`}>Message to your session</label>
      <textarea
        id={`${id}-text`}
        ref={box}
        rows={3}
        value={text}
        readOnly={sending}
        aria-describedby={`${id}-hint`}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void submit();
          }
        }}
      />
      <div className="chat-compose-row">
        <span id={`${id}-hint`} className="muted">
          Typed into the session when it next waits for input · Ctrl+Enter sends
        </span>
        <button type="submit" className="primary" disabled={sending || blank}>
          {sending ? "Sending…" : "Send"}
        </button>
      </div>
    </form>
  );
}
