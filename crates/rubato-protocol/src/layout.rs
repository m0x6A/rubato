//! Where each part sits on the shared map (ADR 0016). Presentation only: never an input to
//! progress, pins or plans. A part with no position here is placed by the automatic layout.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The top-left corner of a part on the map canvas, in the canvas's own units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// Positions by part id. In the overview it is the whole stored layout; as the body of
/// `PUT /api/layout` it holds only the parts that moved; as the answer of `DELETE /api/layout`
/// it is what was removed, so a `PUT` of it undoes the removal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MapLayout {
    #[serde(default)]
    pub positions: BTreeMap<String, Position>,
}

/// A coordinate further out than this is refused: no map is that large, and a runaway value
/// would put a part where nobody finds it.
pub const MAX_COORDINATE: f64 = 100_000.0;

impl Position {
    /// Finite and within `MAX_COORDINATE` of the origin on both axes.
    pub fn on_canvas(&self) -> bool {
        [self.x, self.y]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
    }
}

/// Route path for the layout: `PUT` the moved parts, `DELETE` clears it.
pub mod routes {
    pub const LAYOUT: &str = "/api/layout";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_must_be_finite_and_near() {
        let on = |x: f64, y: f64| Position { x, y }.on_canvas();
        assert!(on(0.0, 0.0));
        assert!(on(-MAX_COORDINATE, MAX_COORDINATE));
        assert!(!on(MAX_COORDINATE + 1.0, 0.0));
        assert!(!on(0.0, f64::NAN));
        assert!(!on(f64::INFINITY, 0.0));
    }

    #[test]
    fn an_overview_without_a_layout_reads_as_an_empty_one() {
        let l: MapLayout = serde_json::from_str("{}").unwrap();
        assert!(l.positions.is_empty());
    }
}
