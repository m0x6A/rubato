import { type KeyboardEvent, type ReactNode, useEffect, useId, useRef, useState } from "react";

interface Props {
  /** What the button shows. */
  button: ReactNode;
  /** The button's accessible name, when what it shows is not enough ("Pin m0x6A/rubato#9 to…"). */
  buttonLabel?: string;
  /** What the open menu is, for a screen reader. */
  menuLabel: string;
  buttonTestId?: string;
  /**
   * The entries: buttons with `role="menuitem"` and `tabIndex={-1}`, grouped as the caller likes.
   * An entry calls `close` before it acts, so focus is back on the button when the action runs.
   */
  children: (close: () => void) => ReactNode;
}

/**
 * A button that opens a menu (issue #357): the one menu of the app, first written for "Pin to…"
 * (design S3, N16). Arrow keys, Home and End move; Enter or Space picks; Escape or leaving the
 * menu closes it. Escape stops here, so it never also closes the view behind the menu.
 */
export function Menu({ button, buttonLabel, menuLabel, buttonTestId, children }: Props) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const items = () => Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);

  useEffect(() => {
    if (open) {
      items()[0]?.focus();
    }
  }, [open]);

  const close = () => {
    setOpen(false);
    trigger.current?.focus();
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
        close();
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
      <button ref={trigger} type="button" aria-haspopup="menu" aria-expanded={open} aria-controls={open ? menuId : undefined} aria-label={buttonLabel} data-testid={buttonTestId} onClick={() => setOpen((o) => !o)}>
        {button}
      </button>
      {open && (
        <div ref={list} id={menuId} role="menu" aria-label={menuLabel} className="pin-menu-list" onKeyDown={onKeyDown} onMouseDown={(e) => e.preventDefault() /* keep focus in the menu: Safari does not focus a clicked button, and the blur would close the menu before the click lands */}>
          {children(close)}
        </div>
      )}
    </span>
  );
}
