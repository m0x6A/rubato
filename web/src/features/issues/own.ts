import type { Me, Overview } from "../../protocol.ts";

/** Companions the current user may queue issues on: their own, paired, not revoked (ADR 0020). */
export function ownLanes(overview: Overview, me: Me | null) {
  if (!me) {
    return [];
  }
  return overview.companions.filter((c) => c.user === me.login && !c.revoked_at);
}

/** Companions the current user may start sessions in: their own, paired, connected (ADR 0004). */
export function ownConnectedCompanions(overview: Overview, me: Me | null) {
  if (!me) {
    return [];
  }
  return overview.companions.filter((c) => c.user === me.login && c.connected && !c.revoked_at);
}
