// Must fail: a span name that is not in crates/rubato-protocol/src/otel.rs. Read as text by
// otel-names.test.ts, never imported or run.
import { Recorder } from "../../scripts/otel-emit.ts";

new Recorder("rubato").span("rubato.skill.bogus", {}, () => 0);
