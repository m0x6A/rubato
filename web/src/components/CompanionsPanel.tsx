import { type FormEvent, useState } from "react";
import { api } from "../api.ts";
import { relative } from "../graph.ts";
import type { Me, PairingInfo } from "../protocol.ts";

interface Props {
  companions: PairingInfo[];
  me: Me | null;
  onChanged: () => void;
}

export function CompanionsPanel({ companions, me, onChanged }: Props) {
  const [code, setCode] = useState("");
  const [msg, setMsg] = useState<string | null>(null);

  const approve = async (e: FormEvent) => {
    e.preventDefault();
    try {
      const p = await api.approvePairing({ user_code: code.trim().toUpperCase() });
      setMsg(`Paired ${p.hostname} as ${p.user}.`);
      setCode("");
      onChanged();
    } catch (err) {
      setMsg(err instanceof Error ? err.message : String(err));
    }
  };

  const revoke = async (id: string) => {
    try {
      await api.revokePairing(id);
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
      <ul className="plain">
        {companions.map((c) => (
          <li key={c.companion_id} className="issue-row" data-testid={`companion-${c.user}`}>
            <span>
              <strong>{c.user}</strong> <span className="muted">{c.hostname}</span>
              <div className="muted">
                {c.revoked_at ? "revoked" : c.connected ? "connected" : c.last_seen ? `last seen ${relative(c.last_seen)}` : "never connected"}
              </div>
            </span>
            {me && c.user === me.login && !c.revoked_at && <button onClick={() => revoke(c.companion_id)}>revoke</button>}
          </li>
        ))}
        {companions.length === 0 && <li className="muted">No companions paired yet.</li>}
      </ul>
    </section>
  );
}
