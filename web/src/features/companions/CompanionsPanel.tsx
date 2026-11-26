import { type FormEvent, useState } from "react";
import type { Me, Overview } from "../../protocol.ts";
import { companionsApi } from "./api.ts";
import { type LaneActions, Lanes } from "./Lanes.tsx";

interface Props extends Omit<LaneActions, "onRevoke"> {
  overview: Overview;
  me: Me | null;
  onChanged: () => void;
}

export function CompanionsPanel({ overview, me, onChanged, ...queueActions }: Props) {
  const [code, setCode] = useState("");
  const [msg, setMsg] = useState<string | null>(null);

  const approve = async (e: FormEvent) => {
    e.preventDefault();
    try {
      const p = await companionsApi.approvePairing({ user_code: code.trim().toUpperCase() });
      setMsg(`Paired ${p.hostname} as ${p.user}.`);
      setCode("");
      onChanged();
    } catch (err) {
      setMsg(err instanceof Error ? err.message : String(err));
    }
  };

  const revoke = async (id: string) => {
    try {
      await companionsApi.revokePairing(id);
      onChanged();
    } catch (err) {
      setMsg(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <section data-testid="companions">
      <h3>Pair a companion</h3>
      {/* The companion runs your sessions on your own machine; Rubato never reaches into it. */}
      <ol className="pair-steps">
        <li>
          On your machine, run:
          <code>rubato-companion pair --server {typeof window === "undefined" ? "<this Rubato's URL>" : window.location.origin}</code>
        </li>
        <li>Type the code it shows here and approve it as yourself.</li>
        <li>
          Start it; its lane shows below, and pinned issues get Start session:
          <code>rubato-companion run</code>
        </li>
      </ol>
      <form className="stack" onSubmit={approve}>
        <label htmlFor="pair-code">Code from rubato-companion pair</label>
        <input id="pair-code" data-testid="pair-code" value={code} onChange={(e) => setCode(e.target.value)} placeholder="ABCD-1234" autoComplete="off" spellCheck={false} />
        <button type="submit" disabled={code.trim().length < 8} title={code.trim().length < 8 ? "Type the whole code first" : undefined}>
          Approve
        </button>
      </form>
      {msg && <p className="muted">{msg}</p>}
      <h3 style={{ marginTop: 12 }}>Companions</h3>
      <Lanes overview={overview} me={me} onRevoke={(id) => void revoke(id)} {...queueActions} />
    </section>
  );
}
