import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import type { IssueRef, MapRef, SystemMap } from "../../protocol.ts";

interface Props {
  map: SystemMap;
  issue: IssueRef;
  /** Where the issue is pinned already; those entries are shown as pinned and cannot be chosen again. */
  pinned?: MapRef[];
  onPin: (issue: IssueRef, target: MapRef) => void;
}

/**
 * "Pin to…": the keyboard twin of dragging an issue onto the map (design S3, N16). A button that
 * opens a menu of the map's parts and seams. Arrow keys, Home and End move; Enter or Space picks;
 * Escape or leaving the menu closes it.
 */
export function PinMenu({ map, issue, pinned = [], onPin }: Props) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const key = `${issue.repo}#${issue.number}`;
  const isPinned = (kind: MapRef["kind"], id: string) => pinned.some((p) => p.kind === kind && p.id === id);

  const items = () => Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);

  useEffect(() => {
    if (open) {
      items()[0]?.focus();
    }
  }, [open]);

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

  const pick = (target: MapRef) => {
    close(true);
    onPin(issue, target);
  };

  const entry = (kind: MapRef["kind"], id: string, detail?: string) => {
    const already = isPinned(kind, id);
    return (
      <button key={`${kind}:${id}`} type="button" role="menuitem" tabIndex={-1} aria-disabled={already || undefined} onClick={() => !already && pick({ kind, id })}>
        <span className="id">{id}</span>
        <span className="muted">{already ? "pinned here" : (detail ?? "")}</span>
      </button>
    );
  };

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
      <button ref={trigger} type="button" aria-haspopup="menu" aria-expanded={open} aria-controls={open ? menuId : undefined} aria-label={`Pin ${key} to…`} onClick={() => setOpen((o) => !o)}>
        Pin to…
      </button>
      {open && (
        <div ref={list} id={menuId} role="menu" aria-label={`Pin ${key} to a part or a seam`} className="pin-menu-list" onKeyDown={onKeyDown} onMouseDown={(e) => e.preventDefault() /* keep focus in the menu: Safari does not focus a clicked button, and the blur would close the menu before the click lands */}>
          <div role="group" aria-label="Parts">
            <div className="pin-menu-heading" aria-hidden="true">
              Parts
            </div>
            {map.nodes.map((n) => entry("node", n.id, n.kind))}
          </div>
          {(map.seams ?? []).length > 0 && (
            <div role="group" aria-label="Seams">
              <div className="pin-menu-heading" aria-hidden="true">
                Seams
              </div>
              {(map.seams ?? []).map((s) => entry("seam", s.id, `${s.from} → ${s.to}`))}
            </div>
          )}
        </div>
      )}
    </span>
  );
}
