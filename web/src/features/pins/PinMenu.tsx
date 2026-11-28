import type { IssueRef, MapRef, SystemMap } from "../../protocol.ts";
import { liveMap } from "../../shared/map.ts";
import { Menu } from "../../shared/Menu.tsx";

interface Props {
  map: SystemMap;
  issue: IssueRef;
  /** Where the issue is pinned already; those entries are shown as pinned and cannot be chosen again. */
  pinned?: MapRef[];
  onPin: (issue: IssueRef, target: MapRef) => void;
}

/**
 * "Pin to…": the keyboard twin of dragging an issue onto the map (design S3, N16). A button that
 * opens a menu of the map's parts and seams. Arrow keys, Home and End move; Enter or Space picks;
 * Escape or leaving the menu closes it.
 */
export function PinMenu({ map, issue, pinned = [], onPin }: Props) {
  const key = `${issue.repo}#${issue.number}`;
  // Only live parts are offered (ADR 0077).
  const live = liveMap(map);
  const isPinned = (kind: MapRef["kind"], id: string) => pinned.some((p) => p.kind === kind && p.id === id);

  const entry = (close: () => void, kind: MapRef["kind"], id: string, detail?: string) => {
    const already = isPinned(kind, id);
    return (
      <button key={`${kind}:${id}`} type="button" role="menuitem" tabIndex={-1} aria-disabled={already || undefined} onClick={() => {
        if (!already) {
          close();
          onPin(issue, { kind, id });
        }
      }}>
        <span className="id">{id}</span>
        <span className="muted">{already ? "pinned here" : (detail ?? "")}</span>
      </button>
    );
  };

  return (
    <Menu button="Pin to…" buttonLabel={`Pin ${key} to…`} menuLabel={`Pin ${key} to a part or a seam`}>
      {(close) => (
        <>
          <div role="group" aria-label="Parts">
            <div className="pin-menu-heading" aria-hidden="true">
              Parts
            </div>
            {live.nodes.map((n) => entry(close, "node", n.id, n.kind))}
          </div>
          {(live.seams ?? []).length > 0 && (
            <div role="group" aria-label="Seams">
              <div className="pin-menu-heading" aria-hidden="true">
                Seams
              </div>
              {(live.seams ?? []).map((s) => entry(close, "seam", s.id, `${s.from} → ${s.to}`))}
            </div>
          )}
        </>
      )}
    </Menu>
  );
}
