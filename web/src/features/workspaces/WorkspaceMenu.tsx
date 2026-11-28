import type { Overview } from "../../protocol.ts";
import { Menu } from "../../shared/Menu.tsx";

interface Props {
  overview: Overview;
  onSwitch: (id: string) => void;
  onConfigure: () => void;
  onWorkspaces: () => void;
}

/**
 * Where the page is (issue #357): the button names the workspace, the application whose map is
 * shown. Its menu says which commit the map was read at, switches workspace when there are
 * several, and leads to this workspace's settings and to the list of workspaces. Drawn with one
 * workspace too, since its settings must always be reachable (ADR 0075).
 */
export function WorkspaceMenu({ overview, onSwitch, onConfigure, onWorkspaces }: Props) {
  const all = overview.workspaces ?? [];
  const name = all.find((w) => w.id === overview.workspace)?.name ?? overview.map.map.name;
  return (
    <span className="top-menu" data-testid="workspace-menu">
      <Menu
        button={
          <>
            {name} <span aria-hidden="true">▾</span>
          </>
        }
        buttonLabel={`Workspace ${name}`}
        menuLabel="Workspace"
      >
        {(close) => (
          <>
            <div className="menu-note" data-testid="map-version" title={`The map file as read at commit ${overview.map.version}; plans name parts of this version`}>
              map {overview.map.map.name} @ {overview.map.version.slice(0, 7)}
            </div>
            {all.length > 1 && (
              <div role="group" aria-label="Show another workspace">
                {all.map((w) => (
                  <button
                    key={w.id}
                    type="button"
                    role="menuitemradio"
                    aria-checked={w.id === overview.workspace}
                    tabIndex={-1}
                    data-testid={`switch-workspace-${w.id}`}
                    onClick={() => {
                      close();
                      if (w.id !== overview.workspace) {
                        onSwitch(w.id);
                      }
                    }}
                  >
                    <span>{w.name}</span>
                    <span className="muted">{w.id === overview.workspace ? "showing" : ""}</span>
                  </button>
                ))}
              </div>
            )}
            <div role="group" aria-label="Settings" className="menu-group">
              <button
                type="button"
                role="menuitem"
                tabIndex={-1}
                data-testid="configure-workspace"
                onClick={() => {
                  close();
                  onConfigure();
                }}
              >
                Configure workspace…
              </button>
              <button
                type="button"
                role="menuitem"
                tabIndex={-1}
                data-testid="open-workspaces"
                onClick={() => {
                  close();
                  onWorkspaces();
                }}
              >
                Workspaces…
              </button>
            </div>
          </>
        )}
      </Menu>
    </span>
  );
}
