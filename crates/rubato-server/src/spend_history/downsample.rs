//! Downsampling a spend series for the overview (design 103 §4). Pure.

use rubato_protocol::SpendPoint;

/// At most this many points per issue reach the overview (ADR 0037).
pub const MAX_POINTS: usize = 24;

/// A series of `max` or fewer points comes back as is. A longer one keeps its first and last
/// point and, between them, the last point of each of `max - 2` equal time spans that has one.
/// The result is a subsequence, so a non-decreasing series stays non-decreasing.
pub fn downsample(points: Vec<SpendPoint>, max: usize) -> Vec<SpendPoint> {
    if points.len() <= max || max < 2 {
        return points;
    }
    let spans = (max - 2) as i64;
    let (first, last) = (points[0].at, points[points.len() - 1].at);
    let width = (last - first).num_milliseconds().max(1);
    let mut out = vec![points[0].clone()];
    let mut kept: Option<(i64, SpendPoint)> = None;
    for p in &points[1..points.len() - 1] {
        let span = ((p.at - first).num_milliseconds() * spans / width).clamp(0, spans - 1);
        if let Some((s, prev)) = kept.take()
            && s != span
        {
            out.push(prev);
        }
        kept = Some((span, p.clone()));
    }
    if let Some((_, p)) = kept {
        out.push(p);
    }
    out.push(points[points.len() - 1].clone());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, TimeZone, Utc};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 8, 0, 0).unwrap()
    }

    fn series(n: usize) -> Vec<SpendPoint> {
        (0..n)
            .map(|i| SpendPoint {
                at: t0() + Duration::minutes(15 * i as i64),
                usd: i as f64 * 0.5,
            })
            .collect()
    }

    #[test]
    fn a_long_series_becomes_24_points_with_first_and_last_kept() {
        let long = series(100);
        let out = downsample(long.clone(), MAX_POINTS);
        assert_eq!(out.len(), 24, "{} points", out.len());
        assert_eq!(out.first(), long.first());
        assert_eq!(out.last(), long.last());
        assert!(
            out.windows(2)
                .all(|w| w[0].usd <= w[1].usd && w[0].at < w[1].at),
            "non-decreasing and in time order: {out:?}"
        );
    }

    #[test]
    fn a_short_series_comes_back_unchanged() {
        for n in [0, 1, 2, 23, 24] {
            assert_eq!(downsample(series(n), MAX_POINTS), series(n), "{n} points");
        }
    }

    #[test]
    fn uneven_gaps_keep_the_last_point_of_each_span() {
        // Twenty points in the first hour, then one a day later: the busy hour falls into the
        // first span, so it is kept by its last point only.
        let mut pts: Vec<SpendPoint> = (0..30)
            .map(|i| SpendPoint {
                at: t0() + Duration::minutes(2 * i),
                usd: i as f64,
            })
            .collect();
        pts.push(SpendPoint {
            at: t0() + Duration::days(1),
            usd: 100.0,
        });
        let out = downsample(pts.clone(), MAX_POINTS);
        assert!(out.len() <= MAX_POINTS);
        assert_eq!(out.first(), pts.first());
        assert_eq!(out.last(), pts.last());
        assert_eq!(
            out.len(),
            3,
            "first, the last of the busy span, last: {out:?}"
        );
        assert_eq!(out[1].usd, 29.0);
    }
}
