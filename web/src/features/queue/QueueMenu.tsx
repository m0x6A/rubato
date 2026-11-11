import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import type { IssueRef, PairingInfo, QueuedIssue } from "../../protocol.ts";

interface Props {
  issue: IssueRef;
  /** The caller's own companions that take a queue: paired and not revoked. */
  lanes: PairingInfo[];
  queue: QueuedIssue[];
  onQueue: (issue: IssueRef, companionId: string) => void;
}

/**
 * "Queue on…": the keyboard twin of dropping an issue on one of your lanes (design S14). A
 * button that opens a menu of your own companions. Arrow keys, Home and End move; Enter or
 * Space picks; Escape or leaving the menu closes it. Not shown to someone with no companion.
 */
export function QueueMenu({ issue, lanes, queue, onQueue }: Props) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const key = `${issue.repo}#${issue.number}`;
  const items = () => Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);

  useEffect(() => {
    if (open) {
      items()[0]?.focus();
    }
  }, [open]);

  if (lanes.length === 0) {
    return null;
  }
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) {
      trigger.current?.focus();
    }
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const all = items();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => {
      e.preventDefault();
      all[(i + all.length) % all.length]?.focus();
    };
    switch (e.key) {
      case "Escape":
        e.preventDefault();
        e.stopPropagation();
        close(true);
        break;
      case "ArrowDown":
        go(at + 1);
        break;
      case "ArrowUp":
        go(at - 1);
        break;
      case "Home":
        go(0);
        break;
      case "End":
        go(all.length - 1);
        break;
    }
  };
  const queuedOn = (companionId: string) => queue.some((q) => q.companion_id === companionId && q.issue.repo === issue.repo && q.issue.number === issue.number);

  return (
    <span
      className="pin-menu"
      onClick={(e) => e.stopPropagation()}
      onBlur={(e) => {
        if (open && !e.currentTarget.contains(e.relatedTarget)) {
          setOpen(false);
        }
      }}
    >
      <button ref={trigger} type="button" aria-haspopup="menu" aria-expanded={open} aria-controls={open ? menuId : undefined} aria-label={`Queue ${key} on…`} onClick={() => setOpen((o) => !o)}>
        Queue on…
      </button>
      {open && (
        <div ref={list} id={menuId} role="menu" aria-label={`Queue ${key} on one of your companions`} className="pin-menu-list" onKeyDown={onKeyDown} onMouseDown={(e) => e.preventDefault()}>
          {lanes.map((c) => {
            const already = queuedOn(c.companion_id);
            return (
              <button
                key={c.companion_id}
                type="button"
                role="menuitem"
                tabIndex={-1}
                aria-disabled={already || undefined}
                onClick={() => {
                  if (!already) {
                    close(true);
                    onQueue(issue, c.companion_id);
                  }
                }}
              >
                <span className="id">{c.hostname}</span>
                <span className="muted">{already ? "queued here" : c.connected ? "connected" : "not connected"}</span>
              </button>
            );
          })}
        </div>
      )}
    </span>
  );
}
