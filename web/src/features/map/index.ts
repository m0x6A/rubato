/** Map: the system map drawn from the overview, painted by plans and badged with issues. */
export { buildGraph } from "./graph.ts";
export type { DraftMarkLike } from "./graph.ts";
export { MapView } from "./MapView.tsx";
export { mapApi } from "./api.ts";
export { Inspector, PlanInspector } from "./Inspector.tsx";
export { Discrepancies } from "./Discrepancies.tsx";
export type { Take } from "./territory.ts";
export { discrepancies } from "./territory.ts";
export { TrafficVitals, trafficVitals, unknownSentence } from "./traffic.tsx";
