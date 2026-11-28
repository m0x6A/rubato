import { type FormEvent, useState } from "react";
import type { Me, Overview } from "../../protocol.ts";
import { companionsApi } from "./api.ts";
import { type LaneActions, Lanes } from "./Lanes.tsx";

interface Props extends Omit<LaneActions, "onRevoke"> {
  overview: Overview;
  me: Me | null;
  onChanged: () => void;
  /** Pairing is the view (its own address), not the lanes. */
  pairing?: boolean;
  /** Open the pairing view. */
  onPair?: (() => void) | undefined;
  /** Leave the pairing view: back to the lanes. `paired` says what was paired, when something was. */
  onPairDone?: ((paired: string | null) => void) | undefined;
}

/** Whether the person has a companion of their own: paired and not revoked. */
function hasOwn(overview: Overview, me: Me | null): boolean {
  return me !== null && overview.companions.some((c) => c.user.toLowerCase() === me.login.toLowerCase() && !c.revoked_at);
}

/**
 * Pair a companion: what to run on your own machine, where its code goes, and how to start it.
 * Done once per machine, so it has a view of its own (issue #357) and is not above the lanes.
 */
export function PairCompanion({ onPaired }: { onPaired: (text: string) => void }) {
  const [code, setCode] = useState("");
  const [msg, setMsg] = useState<string | null>(null);

  const approve = async (e: FormEvent) => {
    e.preventDefault();
    try {
      const p = await companionsApi.approvePairing({ user_code: code.trim().toUpperCase() });
      setCode("");
      setMsg(`Paired ${p.hostname} as ${p.user}.`);
      onPaired(`Paired ${p.hostname} as ${p.user}.`);
    } catch (err) {
      setMsg(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="pair" data-testid="pair-companion">
      <h3>Pair a companion</h3>
      {/* The companion runs your sessions on your own machine; Rubato never reaches into it. */}
      <ol className="pair-steps">
        <li>
          On your machine, run:
          <code>rubato-companion pair --server {typeof window === "undefined" ? "<this Rubato's URL>" : window.location.origin}</code>
        </li>
        <li>Type the code it shows here and approve it as yourself.</li>
        <li>
          Start it; its lane shows under Companions, and pinned issues get Start session:
          <code>rubato-companion run</code>
        </li>
      </ol>
      <form className="stack" onSubmit={approve}>
        <label htmlFor="pair-code">Code from rubato-companion pair</label>
        <input id="pair-code" data-testid="pair-code" value={code} onChange={(e) => setCode(e.target.value)} placeholder="ABCD-1234" autoComplete="off" spellCheck={false} />
        <button type="submit" className="primary" disabled={code.trim().length < 8} title={code.trim().length < 8 ? "Type the whole code first" : undefined}>
          Approve
        </button>
      </form>
      {msg && (
        <p className="muted" role="status">
          {msg}
        </p>
      )}
    </div>
  );
}

/**
 * Companions (issue #357): who runs what, and the queue on your own lanes. Pairing a machine is
 * a view of its own; it shows here only for someone who has no companion yet, since for them it
 * is the next step.
 */
export function CompanionsPanel({ overview, me, onChanged, pairing = false, onPair, onPairDone, ...queueActions }: Props) {
  const [msg, setMsg] = useState<string | null>(null);

  const revoke = async (id: string) => {
    try {
      await companionsApi.revokePairing(id);
      onChanged();
    } catch (err) {
      setMsg(err instanceof Error ? err.message : String(err));
    }
  };

  if (pairing) {
    return (
      <section data-testid="companions">
        <p>
          <button type="button" onClick={() => onPairDone?.(null)}>
            ← all companions
          </button>
        </p>
        <PairCompanion
          onPaired={(text) => {
            onChanged();
            onPairDone?.(text);
          }}
        />
      </section>
    );
  }
  const own = hasOwn(overview, me);
  return (
    <section data-testid="companions">
      {!own && me && <PairCompanion onPaired={() => onChanged()} />}
      {msg && <p className="warn">{msg}</p>}
      <Lanes overview={overview} me={me} onRevoke={(id) => void revoke(id)} {...queueActions} />
      {own && onPair && (
        <p className="pair-another">
          <button type="button" className="link" data-testid="pair-another" onClick={onPair}>
            Pair another machine
          </button>
        </p>
      )}
    </section>
  );
}
