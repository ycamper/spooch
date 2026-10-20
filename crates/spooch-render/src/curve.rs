//! Text along a curve: the arc a label's baseline bends into, measured by
//! its curvature (one over its radius) so the letters keep their spacing
//! however far it bends.

use std::f32::consts::{PI, TAU};

/// The most a line `w` long bends: into a full circle.
pub fn max_curvature(w: f32) -> f32 {
    TAU / w.max(1.0)
}

/// How far the middle of an arc `w` long with curvature `k` sits below its
/// ends: positive for a smile (`k` > 0, the centre above), negative for a
/// rainbow. A full circle's is its diameter.
pub fn sagitta(k: f32, w: f32) -> f32 {
    let a = k.abs().min(max_curvature(w));
    if a < 1e-6 {
        return 0.0;
    }
    let half = (a * w / 2.0).min(PI);
    k.signum() * (1.0 - half.cos()) / a
}

/// The curvature that gives an arc `w` long the sagitta `s`, up to a full
/// circle.
pub fn curvature_for(s: f32, w: f32) -> f32 {
    let target = s.abs();
    let most = max_curvature(w);
    if target <= 0.0 {
        return 0.0;
    }
    if target >= sagitta(most, w) {
        return s.signum() * most;
    }
    // The sagitta grows with the curvature up to a full circle.
    let (mut lo, mut hi) = (0.0, most);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if sagitta(mid, w) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    s.signum() * (lo + hi) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sagitta_and_curvature_agree() {
        let w = 200.0;
        assert_eq!(sagitta(0.0, w), 0.0);
        for s in [-50.0, -5.0, 3.0, 20.0, 60.0] {
            let k = curvature_for(s, w);
            assert!((sagitta(k, w) - s).abs() < 0.01, "{s}");
            assert_eq!(k.signum(), f32::signum(s));
        }
        // Past a full circle it stays one, as wide as w / pi.
        let k = curvature_for(500.0, w);
        assert!((k - max_curvature(w)).abs() < 1e-6);
        assert!((sagitta(k, w) - w / PI).abs() < 0.01);
    }
}
