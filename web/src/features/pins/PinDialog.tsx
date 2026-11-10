import { type FormEvent, useState } from "react";
import type { PinRequest, SystemMap } from "../../protocol.ts";
import { parseIssueKey } from "../../shared/format.ts";

interface Props {
  map: SystemMap;
  onSubmit: (req: PinRequest) => void;
  onClose: () => void;
}

export function PinDialog({ map, onSubmit, onClose }: Props) {
  const [issue, setIssue] = useState("");
  const [target, setTarget] = useState(`node:${map.nodes[0]?.id ?? ""}`);
  const [error, setError] = useState<string | null>(null);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const ref = parseIssueKey(issue);
    if (!ref) {
      setError("Issue must be owner/name#number");
      return;
    }
    const [kind, id] = target.split(":", 2) as ["node" | "seam", string];
    onSubmit({ issue: ref, target: { kind, id } });
  };

  return (
    <div className="dialog-backdrop" onClick={onClose}>
      <form className="dialog stack" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <h2>Pin an issue to the map</h2>
        <label>
          Issue
          <input data-testid="pin-issue" value={issue} onChange={(e) => setIssue(e.target.value)} placeholder="owner/name#number" autoFocus />
        </label>
        <label>
          Node or seam
          <select data-testid="pin-target" value={target} onChange={(e) => setTarget(e.target.value)}>
            <optgroup label="Nodes">
              {map.nodes.map((n) => (
                <option key={n.id} value={`node:${n.id}`}>
                  {n.id}
                </option>
              ))}
            </optgroup>
            <optgroup label="Seams">
              {(map.seams ?? []).map((s) => (
                <option key={s.id} value={`seam:${s.id}`}>
                  {s.id}
                </option>
              ))}
            </optgroup>
          </select>
        </label>
        {error && <p className="warn">{error}</p>}
        <div className="actions">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="primary" data-testid="pin-submit">
            Pin
          </button>
        </div>
      </form>
    </div>
  );
}
