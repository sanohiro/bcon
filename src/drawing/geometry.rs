//! Geometric primitives for anti-aliased rendering
//!
//! Functions for computing distances and alpha values used in
//! anti-aliased rendering of geometric shapes.

/// Smoothstep interpolation for anti-aliasing.
///
/// Returns smooth transition from 0 to 1 as t goes from 0 to 1.
/// Uses Hermite interpolation: 3t² - 2t³
///
/// # Properties
/// - smoothstep(0) = 0
/// - smoothstep(1) = 1
/// - First derivative is 0 at both endpoints (smooth)
#[inline]
pub fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Compute anti-aliased alpha from signed distance.
///
/// # Arguments
/// * `d` - Signed distance to shape boundary (positive = inside)
/// * `aa_width` - Width of the anti-aliasing transition zone
///
/// # Returns
/// * `d >= 0`: 1.0 (fully inside)
/// * `d < -aa_width`: 0.0 (fully outside)
/// * Otherwise: smooth transition using smoothstep
#[inline]
pub fn aa_alpha_from_distance(d: f32, aa_width: f32) -> f32 {
    if d >= 0.0 {
        1.0
    } else {
        let t = (d / aa_width + 1.0).clamp(0.0, 1.0);
        smoothstep(t)
    }
}

/// Compute approximate SDF (signed distance field) for an ellipse.
///
/// Returns signed distance: positive inside ellipse, negative outside.
/// Uses gradient-based approximation rather than exact Newton iteration
/// for better performance at acceptable visual quality.
///
/// # Algorithm
/// For a point P at normalized coordinates (nx, ny):
/// 1. len = sqrt(nx² + ny²) gives distance from center in normalized space
/// 2. In normalized space, ellipse has implicit equation: x² + y² = 1
/// 3. len - 1.0 gives approximate distance (exact for circles)
/// 4. Scale by gradient correction factor k to account for non-uniform stretch
///
/// The gradient correction k = (rx * ry) / (rx * |ny| + ry * |nx|)
/// approximates the local gradient magnitude of the implicit function.
///
/// # Arguments
/// * `nx`, `ny` - Normalized coordinates (point / radius)
/// * `rx`, `ry` - Ellipse radii (used for gradient correction)
/// * `len` - Length of normalized vector sqrt(nx² + ny²), precomputed for efficiency
///
/// # Note
/// This is an approximation. For exact ellipse SDF, iterative methods
/// like Newton's method are needed, but this is fast and visually sufficient.
#[inline]
pub fn ellipse_sdf(nx: f32, ny: f32, rx: f32, ry: f32, len: f32) -> f32 {
    if len <= 0.001 {
        // Point at center: inside by the smaller radius
        return -rx.min(ry);
    }
    // Gradient-based correction factor for non-circular ellipses
    let k = (rx * ry) / (rx * ny.abs() + ry * nx.abs()).max(0.001);
    // Distance in normalized space, scaled by gradient correction
    (len - 1.0) * k.min(rx.min(ry))
}

/// Calculate shortest distance from point P to line segment AB.
///
/// Used for anti-aliased outline rendering of powerline triangle shapes.
///
/// # Algorithm
/// Uses point-to-line projection to find the closest point on segment AB:
///
/// 1. Compute vectors: v = B - A (segment direction), w = P - A (point offset)
/// 2. Project P onto infinite line AB: t = dot(v, w) / dot(v, v)
/// 3. Clamp t to [0, 1] to stay within segment
/// 4. Return distance from P to the clamped projection point
///
/// # Geometric Cases
/// - t < 0 (c1 <= 0): Point projects before segment start A → return |PA|
/// - t > 1 (c2 <= c1): Point projects after segment end B → return |PB|
/// - 0 <= t <= 1: Point projects within segment → return perpendicular distance
///
/// # Arguments
/// * `px`, `py` - Point coordinates
/// * `ax`, `ay` - Segment start (A)
/// * `bx`, `by` - Segment end (B)
///
/// # Reference
/// <https://en.wikipedia.org/wiki/Distance_from_a_point_to_a_line>
#[inline]
pub fn distance_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax; // Segment direction vector
    let vy = by - ay;
    let wx = px - ax; // Vector from segment start to point
    let wy = py - ay;

    // c1 = dot(v, w): projection of w onto v (unnormalized)
    let c1 = vx * wx + vy * wy;
    if c1 <= 0.0 {
        // Point projects before segment start: closest point is A
        return (wx * wx + wy * wy).sqrt();
    }

    // c2 = dot(v, v) = |v|²: squared length of segment
    let c2 = vx * vx + vy * vy;
    if c2 <= c1 {
        // Point projects after segment end: closest point is B
        let dx = px - bx;
        let dy = py - by;
        return (dx * dx + dy * dy).sqrt();
    }

    // Point projects within segment: compute perpendicular distance
    // t = c1 / c2 is the normalized projection parameter (0 <= t <= 1)
    let t = c1 / c2;
    let proj_x = ax + t * vx;
    let proj_y = ay + t * vy;
    let dx = px - proj_x;
    let dy = py - proj_y;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smoothstep_boundaries() {
        assert!((smoothstep(0.0) - 0.0).abs() < 1e-6);
        assert!((smoothstep(1.0) - 1.0).abs() < 1e-6);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_distance_to_segment() {
        // Point directly on segment
        assert!((distance_to_segment(0.5, 0.0, 0.0, 0.0, 1.0, 0.0)).abs() < 1e-6);

        // Point above segment midpoint
        let d = distance_to_segment(0.5, 1.0, 0.0, 0.0, 1.0, 0.0);
        assert!((d - 1.0).abs() < 1e-6);

        // Point at segment start
        assert!((distance_to_segment(0.0, 0.0, 0.0, 0.0, 1.0, 0.0)).abs() < 1e-6);
    }
}
