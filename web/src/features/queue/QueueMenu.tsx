import type { IssueRef, PairingInfo, QueuedIssue } from "../../protocol.ts";
import { Menu } from "../../shared/Menu.tsx";

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
  const key = `${issue.repo}#${issue.number}`;
  if (lanes.length === 0) {
    return null;
  }
  const queuedOn = (companionId: string) => queue.some((q) => q.companion_id === companionId && q.issue.repo === issue.repo && q.issue.number === issue.number);

  return (
    <Menu button="Queue on…" buttonLabel={`Queue ${key} on…`} menuLabel={`Queue ${key} on one of your companions`}>
      {(close) =>
        lanes.map((c) => {
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
                  close();
                  onQueue(issue, c.companion_id);
                }
              }}
            >
              <span className="id">{c.hostname}</span>
              <span className="muted">{already ? "queued here" : c.connected ? "connected" : "not connected"}</span>
            </button>
          );
        })
      }
    </Menu>
  );
}
