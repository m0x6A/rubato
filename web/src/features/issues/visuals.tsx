/**
 * Pictures of an issue's state (issue #81, design §6): a ring on each row, a stage rail and a
 * spend gauge on the selected issue, and the team's issues as one segmented track in the topbar.
 * The text beside them still says the state; these lead, they do not replace it.
 */
import type { IssueSummary, ProgressState } from "../../protocol.ts";
import { STATE_LABEL } from "../../shared/format.ts";

/** The progress model's stations, in order. Blocked and unknown are off the rail. */
export const RAIL: { state: ProgressState; label: string }[] = [
  { state: "planned", label: "Planned" },
  { state: "started", label: "Started" },
  { state: "designing", label: "Designing" },
  { state: "designed", label: "Designed" },
  { state: "building", label: "Building" },
  { state: "in_review", label: "Review" },
  { state: "done", label: "Done" },
];

const station = (state: ProgressState) => RAIL.findIndex((r) => r.state === state);

const R = 11;
const CIRC = 2 * Math.PI * R;

/** A ring: how far along the rail the issue is. Blocked is a red ring with a gap; unknown a dashed grey ring. */
export function ProgressRing({ state }: { state: ProgressState }) {
  const at = station(state);
  const share = at >= 0 ? `${at + 1}/${RAIL.length}` : state === "blocked" ? "gap" : "dashed";
  const label = `${STATE_LABEL[state]}${at >= 0 ? `, ${at + 1} of ${RAIL.length}` : ""}`;
  return (
    <svg className={`ring ${state}`} data-share={share} width="28" height="28" viewBox="0 0 30 30" role="img" aria-label={label}>
      {state === "unknown" ? (
        <circle cx="15" cy="15" r={R} className="ring-dashed" />
      ) : (
        <>
          <circle cx="15" cy="15" r={R} className="ring-track" />
          <circle cx="15" cy="15" r={R} className="ring-arc" transform="rotate(-90 15 15)" strokeDasharray={state === "blocked" ? `${CIRC * 0.42} 5 ${CIRC} ${CIRC}` : `${(CIRC * (at + 1)) / RAIL.length} ${CIRC}`} />
        </>
      )}
    </svg>
  );
}

/** Seven stations, the current one lit; blocked is a notch on the rail, unknown lights nothing. */
export function StageRail({ state }: { state: ProgressState }) {
  const at = station(state);
  const label = at >= 0 ? `Progress: ${STATE_LABEL[state]}, station ${at + 1} of ${RAIL.length}` : `Progress: ${STATE_LABEL[state]}`;
  return (
    <div className={`stage-rail ${state}`} data-testid="stage-rail" role="img" aria-label={label}>
      {RAIL.map((r, i) => (
        <span key={r.state} className={["station", i < at ? "past" : i === at ? "now" : ""].filter(Boolean).join(" ")}>
          <i aria-hidden="true" />
          <span className="label">{r.label}</span>
        </span>
      ))}
      {state === "blocked" && <span className="notch" aria-hidden="true" />}
    </div>
  );
}

/** Spend against the plan's cap: calm below 80 %, amber from 80 %, red at the cap. Nothing without a cap. */
export function SpendGauge({ spend, cap }: { spend: number; cap: number | null | undefined }) {
  if (!cap || cap <= 0) {
    return null;
  }
  const share = Math.min(1, spend / cap);
  const tone = spend >= cap ? "over" : share >= 0.8 ? "near" : "calm";
  const arc = Math.PI * 52;
  return (
    <div className="gauge" data-tone={tone} role="img" aria-label={`spend $${spend.toFixed(2)} of a $${cap.toFixed(2)} cap`}>
      <svg width="132" height="80" viewBox="0 0 132 84" aria-hidden="true">
        <path d="M14 74 A52 52 0 0 1 118 74" className="gauge-track" />
        <path d="M14 74 A52 52 0 0 1 118 74" className="gauge-arc" strokeDasharray={`${arc * share} ${arc}`} />
        <path d="M103.2 47 L112.9 39.9" className="gauge-mark" />
      </svg>
      <span className="gauge-value" aria-hidden="true">
        ${spend.toFixed(2)}
      </span>
      <span className="gauge-cap" aria-hidden="true">
        of ${cap.toFixed(0)} cap
      </span>
    </div>
  );
}

const TRACK_ORDER: ProgressState[] = ["planned", "started", "designing", "designed", "building", "in_review", "done", "blocked", "unknown"];

/** The team's issues as one segmented track, a segment per state, as wide as its count. */
export function TeamTrack({ issues }: { issues: IssueSummary[] }) {
  const counts = new Map<ProgressState, number>();
  for (const i of issues) {
    counts.set(i.progress.state, (counts.get(i.progress.state) ?? 0) + 1);
  }
  const present = TRACK_ORDER.filter((s) => counts.get(s));
  const words = present.map((s) => `${counts.get(s)} ${STATE_LABEL[s].toLowerCase()}`).join(" · ");
  return (
    <div className="team-track" data-testid="team-track" title={words}>
      <div className="track-bar" aria-hidden="true">
        {present.map((s) => (
          <span key={s} className={`segment ${s}`} data-state={s} style={{ flexGrow: counts.get(s) }} />
        ))}
      </div>
      <span className="track-words">
        {issues.length} issue{issues.length === 1 ? "" : "s"}
        <span className="sr-only">: {words}</span>
      </span>
    </div>
  );
}
