import { findingText } from "../graph.ts";
import type { Finding } from "../protocol.ts";

export function Findings({ findings }: { findings: Finding[] }) {
  if (findings.length === 0) {
    return null;
  }
  return (
    <div className="findings" data-testid="findings">
      <strong>
        {findings.length} finding{findings.length === 1 ? "" : "s"}
      </strong>
      <ul>
        {findings.map((f, i) => (
          <li key={i}>{findingText(f)}</li>
        ))}
      </ul>
    </div>
  );
}
