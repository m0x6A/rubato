import { useState } from "react";
import { IssueList } from "../src/features/issues/IssueList.tsx";
import { IssuePanel as IssueDetail } from "../src/features/issues/IssuePanel.tsx";
import type { IssueRef, IssueSummary, MapRef, Me, Overview, PinRequest } from "../src/protocol.ts";
import type { IssueList as Which } from "../src/shared/route.ts";

interface Props {
  overview: Overview;
  me: Me | null;
  selected: IssueSummary | null;
  selection: MapRef | null;
  onSelect: (issue: IssueRef | null) => void;
  onClearSelection: () => void;
  onLaunch: (issue: IssueRef, branch?: string) => void;
  onPin: (issue: IssueRef, target: MapRef) => void;
  onUnpin: (req: PinRequest) => void;
  onQueue?: ((issue: IssueRef, companionId: string) => void) | undefined;
  onNotice?: ((text: string) => void) | undefined;
  onShowSeam?: ((seam: string) => void) | undefined;
  onLaunchLead?: ((issue: IssueRef, companionId: string) => void) | undefined;
  onPairCompanion?: (() => void) | undefined;
}

/**
 * For tests: the issue list and the issue detail as the shell puts them together, an issue's
 * detail in place of the list (issue #357 made them two components and the choice a route).
 */
export function IssuePanel({ selected, selection, onSelect, onClearSelection, onQueue, ...rest }: Props) {
  const [list, setList] = useState<Which>("open");
  if (selected) {
    return <IssueDetail {...rest} selected={selected} onClose={() => onSelect(null)} onOpenIssue={onSelect} />;
  }
  return <IssueList overview={rest.overview} me={rest.me} list={list} onList={setList} selection={selection} onClearSelection={onClearSelection} onOpen={onSelect} onPin={rest.onPin} onQueue={onQueue} />;
}
