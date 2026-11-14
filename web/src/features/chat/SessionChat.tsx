import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { Me, SessionState } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import { relative } from "../../shared/format.ts";
import { onWebEvent } from "../live/index.ts";
import { chatApi, failureWords } from "./api.ts";
import { stateText } from "./states.ts";
import type { LaunchMessages, SessionMessageView } from "./types.ts";

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
 * Messages to one launched session (ADR 0051). The whole team reads them; only the launch's
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
        <span aria-hidden="true">{open ? "▾" : "▸"}</span> Messages
      </button>
      <div id={panel}>{open && <Conversation launchId={session.launch_id} hooksLost={session.hooks_lost === true} me={me} />}</div>
    </div>
  );
}

function Conversation({ launchId, hooksLost, me }: { launchId: string; hooksLost: boolean; me: Me | null }) {
  const [data, setData] = useState<LaunchMessages | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [said, setSaid] = useState<{ text: string; failed: boolean } | null>(null);
  // Only the newest read lands, so a slow answer never overwrites a newer one.
  const latest = useRef(0);
  // One send at a time, also for two key presses inside one render.
  const busy = useRef(false);

  const load = useCallback(async () => {
    const n = ++latest.current;
    try {
      const d = await chatApi.messages(launchId);
      if (n === latest.current) {
        setData(d);
        setLoadError(null);
      }
    } catch (e) {
      if (n === latest.current && !needsLogin(e)) {
        setLoadError(failureWords(e));
      }
    }
  }, [launchId]);

  useEffect(() => {
    void load();
    return () => {
      latest.current += 1;
    };
  }, [load]);

  // The server says `updated` whenever a message of this launch is added or changes state.
  useEffect(
    () =>
      onWebEvent((ev) => {
        if (ev.type === "updated" && ev.resource.kind === "launch_messages" && ev.resource.launch_id === launchId) {
          void load();
        }
      }),
    [launchId, load],
  );

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

  const isOwner = me !== null && data !== null && sameLogin(me.login, data.owner);
  const cannot = hooksLost ? HOOKS_LOST : data && !data.companion_can_receive ? UPGRADE : null;
  const canSend = isOwner && cannot === null;

  return (
    <div className="chat" data-testid={`chat-${launchId}`}>
      {loadError && <p className="warn">Messages could not be read: {loadError}</p>}
      {data === null ? (
        !loadError && <p className="muted">Loading messages…</p>
      ) : data.messages.length === 0 ? (
        <p className="muted">No messages yet.</p>
      ) : (
        <ol className="plain chat-log" aria-label="Messages to this session">
          {data.messages.map((m) => (
            <Message key={m.id} m={m} live={data.companion_live} onSendAgain={canSend ? () => void send(m.text) : undefined} />
          ))}
        </ol>
      )}
      {data && !isOwner && <p className="muted">Only {data.owner} can message this session.</p>}
      {isOwner && cannot && <p className="warn">{cannot}</p>}
      {canSend && <Compose sending={sending} onSend={send} />}
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

function Message({ m, live, onSendAgain }: { m: SessionMessageView; live: boolean; onSendAgain: (() => void) | undefined }) {
  return (
    <li className="chat-msg" data-testid={`message-${m.id}`}>
      <div className="chat-meta">
        <strong>{m.author}</strong>
        <time className="muted" dateTime={m.created_at} title={new Date(m.created_at).toLocaleString()}>
          {relative(m.created_at)}
        </time>
        <span className={`msg-state ${m.state}`}>{stateText(m, live)}</span>
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
