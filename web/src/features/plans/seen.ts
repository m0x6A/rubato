/**
 * The revision of each draft you last looked at, in this browser's `localStorage` (ADR 0053):
 * per device, never sent anywhere. Storage may be missing or refuse (a private window, blocked
 * site data); then nothing counts as seen and the page works the same, without a diff.
 */

export const seenKey = (draftId: string) => `rubato.draft-seen.${draftId}`;

export function readSeen(draftId: string): number | null {
  try {
    const v = localStorage.getItem(seenKey(draftId));
    if (v === null || !/^[1-9]\d*$/.test(v)) {
      return null;
    }
    return Number(v);
  } catch {
    return null;
  }
}

export function writeSeen(draftId: string, rev: number): void {
  try {
    localStorage.setItem(seenKey(draftId), String(rev));
  } catch {
    // No storage: next time nothing counts as seen.
  }
}
