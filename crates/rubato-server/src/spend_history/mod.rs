//! Slice `spend_history`: each issue's spend over time (ADR 0037, design 103 §2–§4). Records the
//! spend a companion's `Sessions` snapshot carries and serves it, downsampled, to the overview.
//! Owns tables `spend_sessions` and `spend_points`. Names no other slice.

mod downsample;
mod store;

pub use downsample::{MAX_POINTS, downsample};
pub use store::{record, series};
