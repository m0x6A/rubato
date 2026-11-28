import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";

interface Props {
  /** The first button: what the person asks for, ending in "…" since a question follows. */
  ask: ReactNode;
  askLabel?: string;
  askTestId?: string;
  /** The button that does it, named for what it does ("Remove alice"), never "OK". */
  confirm: ReactNode;
  confirmTestId?: string;
  /** The way out, named for what stays ("Keep it"). */
  keep: ReactNode;
  /** What doing it means, shown with the two buttons. */
  explain?: ReactNode;
  /** Wraps the question: a block with its own test id where the caller had one. */
  wrap?: (question: ReactNode) => ReactNode;
  disabled?: boolean;
  onConfirm: () => void;
}

/**
 * Ask once, inline, before something that cannot be taken back (issue #357): the button gives
 * way to what it means and two buttons, with focus on the way out. Escape keeps things as they
 * are. No dialog: the question sits where the button was.
 */
export function Confirm({ ask, askLabel, askTestId, confirm, confirmTestId, keep, explain, wrap, disabled, onConfirm }: Props) {
  const [asking, setAsking] = useState(false);
  const keepButton = useRef<HTMLButtonElement>(null);
  const askButton = useRef<HTMLButtonElement>(null);
  const wasAsking = useRef(false);

  useEffect(() => {
    if (asking) {
      keepButton.current?.focus();
    } else if (wasAsking.current) {
      askButton.current?.focus();
    }
    wasAsking.current = asking;
  }, [asking]);

  if (!asking) {
    return (
      <button ref={askButton} type="button" disabled={disabled} aria-label={askLabel} data-testid={askTestId} onClick={() => setAsking(true)}>
        {ask}
      </button>
    );
  }
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setAsking(false);
    }
  };
  const question = (
    <>
      {explain}
      <span className="settings-actions" role="group" onKeyDown={onKeyDown}>
        <button
          type="button"
          disabled={disabled}
          data-testid={confirmTestId}
          onClick={() => {
            setAsking(false);
            onConfirm();
          }}
        >
          {confirm}
        </button>
        <button ref={keepButton} type="button" onClick={() => setAsking(false)}>
          {keep}
        </button>
      </span>
    </>
  );
  return <>{wrap ? wrap(question) : question}</>;
}
