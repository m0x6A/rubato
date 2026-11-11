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
      <form className="stack" onSubmit={approve}>
        <input data-testid="pair-code" value={code} onChange={(e) => setCode(e.target.value)} placeholder="code shown by rubato-companion pair" />
        <button type="submit" disabled={code.trim().length < 8}>
          Approve
        </button>
      </form>
      {msg && <p className="muted">{msg}</p>}
      <h3 style={{ marginTop: 12 }}>Companions</h3>
      <Lanes overview={overview} me={me} onRevoke={(id) => void revoke(id)} {...queueActions} />
    </section>
  );
}
