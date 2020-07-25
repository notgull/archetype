//! Adapted from https://github.com/Logicalshift/flo_curves/blob/master/src/bezier/fit.rs

use super::BezierCurve;
use pathfinder_geometry::vector::Vector2F;

/// Maximum number of iterations to perform when trying to improve the curve fit
const MAX_ITERATIONS: usize = 4;

// How far out of the error bounds we can be (as a ratio of the maximum error) and still attempt to fit the curve
const FIT_ATTEMPT_RATIO: f32 = 4.0;

/// Maximum number of points to fit at once (curves with more points are divided before fitting)
const MAX_POINTS_TO_FIT: usize = 100;

///
/// Creates a bezier curve that fits a set of points with a particular error
///
/// Algorithm from Philip J. Schdeiner, Graphics Gems
///
/// There are a few modifications from the original algorithm:
///
///   * The 'small' error used to determine if we should use Newton-Raphson is now
///     just a multiplier of the max error
///   * We only try to fit a certain number of points at once as the algorithm runs
///     in quadratic time otherwise
///
pub fn fit_curve(points: &[Vector2F], max_error: f32) -> Option<Vec<BezierCurve>> {
    // Need at least 2 points to fit anything
    if points.len() < 2 {
        // Insufficient points for this curve
        None
    } else {
        let mut curves = vec![];

        // Divide up the points into blocks containing MAX_POINTS_TO_FIT items
        let num_blocks = ((points.len() - 1) / MAX_POINTS_TO_FIT) + 1;

        for point_block in 0..num_blocks {
            // Pick the set of points that will be in this block
            let start_point = point_block * MAX_POINTS_TO_FIT;
            let mut num_points = MAX_POINTS_TO_FIT;

            if start_point + num_points > points.len() {
                num_points = points.len() - start_point;
            }

            // Edge case: one point outside of a block (we ignore these blocks)
            if num_points < 2 {
                continue;
            }

            // Need the start and end tangents so we know how the curve continues
            let block_points = &points[start_point..start_point + num_points];

            let start_tangent = start_tangent(block_points);
            let end_tangent = end_tangent(block_points);

            let fit = fit_curve_cubic(block_points, &start_tangent, &end_tangent, max_error);
            curves.extend(fit);
        }

        Some(curves)
    }
}

///
/// Fits a bezier curve to a subset of points
///
pub fn fit_curve_cubic(
    points: &[Vector2F],
    start_tangent: &Vector2F,
    end_tangent: &Vector2F,
    max_error: f32,
) -> Vec<BezierCurve> {
    if points.len() <= 2 {
        // 2 points is a line (less than 2 points is an error here)
        fit_line(&points[0], &points[1])
    } else {
        // Find the initial set of chords (estimates for where the t values for each of the points are)
        let mut chords = chords_for_points(points);

        // Use the least-squares method to fit against the initial set of chords
        let mut curve: BezierCurve = generate_bezier(points, &chords, start_tangent, end_tangent);

        // Just use this curve if we got a good fit
        let (mut error, mut split_pos) = max_error_for_curve(points, &chords, &curve);

        // Try iterating to improve the fit if we're not too far out
        if error > max_error && error < max_error * FIT_ATTEMPT_RATIO {
            for _iteration in 0..MAX_ITERATIONS {
                // Recompute the chords and the curve
                chords = reparameterize(points, &chords, &curve);
                curve = generate_bezier(points, &chords, start_tangent, end_tangent);

                // Recompute the error
                let (new_error, new_split_pos) = max_error_for_curve(points, &chords, &curve);
                error = new_error;
                split_pos = new_split_pos;

                if error <= max_error {
                    break;
                }
            }
        }

        if error <= max_error {
            // We've generated a curve within the error bounds
            vec![curve]
        } else {
            // If error still too large, split the points and create two curves
            let center_tangent = tangent_between(
                &points[split_pos - 1],
                &points[split_pos],
                &points[split_pos + 1],
            );

            // Fit the two sides
            let lhs = fit_curve_cubic(
                &points[0..split_pos + 1],
                start_tangent,
                &center_tangent,
                max_error,
            );
            let rhs = fit_curve_cubic(
                &points[split_pos..points.len()],
                &(center_tangent * -1.0),
                end_tangent,
                max_error,
            );

            // Collect the result
            lhs.into_iter().chain(rhs.into_iter()).collect()
        }
    }
}

///
/// Creates a curve representing a line between two points
///
fn fit_line(p1: &Vector2F, p2: &Vector2F) -> Vec<BezierCurve> {
    // Any bezier curve where the control points line up forms a straight line; we use points around 1/3rd of the way along in our generation here
    let direction = *p2 - *p1;
    let cp1 = *p1 + (direction * 0.33);
    let cp2 = *p1 + (direction * 0.66);

    vec![BezierCurve::from_points([*p1, cp1, cp2, *p2])]
}

///
/// Chord-length parameterizes a set of points
///
/// This is an estimate of the 't' value for these points on the final curve.
///
fn chords_for_points(points: &[Vector2F]) -> Vec<f32> {
    let mut distances = vec![];
    let mut total_distance = 0.0;

    // Compute the distances for each point
    distances.push(total_distance);
    for p in 1..points.len() {
        total_distance += distance(&points[p - 1], &points[p]);
        distances.push(total_distance);
    }

    // Normalize to the range 0..1
    for p in 0..points.len() {
        distances[p] /= total_distance;
    }

    distances
}

///
/// Generates a bezier curve using the least-squares method
///
fn generate_bezier(
    points: &[Vector2F],
    chords: &[f32],
    start_tangent: &Vector2F,
    end_tangent: &Vector2F,
) -> BezierCurve {
    // Precompute the RHS as 'a'
    let a: Vec<_> = chords
        .iter()
        .map(|chord| {
            let inverse_chord = 1.0 - chord;

            let b1 = 3.0 * chord * (inverse_chord * inverse_chord);
            let b2 = 3.0 * chord * chord * inverse_chord;

            (*start_tangent * b1, *end_tangent * b2)
        })
        .collect();

    // Create the 'C' and 'X' matrices
    let mut c = [[0.0, 0.0], [0.0, 0.0]];
    let mut x = [0.0, 0.0];

    let last_point = points[points.len() - 1];

    for point in 0..points.len() {
        c[0][0] += a[point].0.dot(a[point].0);
        c[0][1] += a[point].0.dot(a[point].1);
        c[1][0] = c[0][1];
        c[1][1] += a[point].1.dot(a[point].1);

        let chord = chords[point];
        let inverse_chord = 1.0 - chord;
        let b0 = inverse_chord * inverse_chord * inverse_chord;
        let b1 = 3.0 * chord * (inverse_chord * inverse_chord);
        let b2 = 3.0 * chord * chord * inverse_chord;
        let b3 = chord * chord * chord;

        let tmp = points[point]
            - ((points[0] * b0) + (points[0] * b1) + (last_point * b2) + (last_point * b3));

        x[0] += a[point].0.dot(tmp);
        x[1] += a[point].1.dot(tmp);
    }

    // Compute their determinants
    let det_c0_c1 = c[0][0] * c[1][1] - c[1][0] * c[0][1];
    let det_c0_x = c[0][0] * x[1] - c[1][0] * x[0];
    let det_x_c1 = x[0] * c[1][1] - x[1] * c[0][1];

    // Derive alpha values
    let alpha_l = if f32::abs(det_c0_c1) < 1.0e-4 {
        0.0
    } else {
        det_x_c1 / det_c0_c1
    };
    let alpha_r = if f32::abs(det_c0_c1) < 1.0e-4 {
        0.0
    } else {
        det_c0_x / det_c0_c1
    };

    // Use the Wu/Barsky heuristic if alpha-negative
    let seg_length = distance(&points[0], &last_point);
    let epsilon = 1.0e-6 * seg_length;

    if alpha_l < epsilon || alpha_r < epsilon {
        // Much less accurate means of estimating a curve
        let dist = seg_length / 3.0;
        BezierCurve::from_points([
            points[0],
            points[0] + (*start_tangent * dist),
            last_point + (*end_tangent * dist),
            last_point,
        ])
    } else {
        // The control points are positioned an alpha distance out along the tangent vectors
        BezierCurve::from_points([
            points[0],
            points[0] + (*start_tangent * alpha_l),
            last_point + (*end_tangent * alpha_r),
            last_point,
        ])
    }
}

///
/// Computes the maximum error for a curve fit against a given set of points
///
/// The chords indicate the estimated t-values corresponding to the points.
///
/// Returns the maximum error and the index of the point with that error.
///
fn max_error_for_curve(points: &[Vector2F], chords: &[f32], curve: &BezierCurve) -> (f32, usize) {
    let errors = points.iter().zip(chords.iter()).map(|(point, chord)| {
        // Get the actual position of this point and the offset
        let actual = curve.eval(*chord);
        let offset = *point - actual;

        // The dot product of an item with itself is the square of the distance
        offset.dot(offset)
    });

    // Search the errors for the biggest one
    let mut biggest_error_squared = 0.0;
    let mut biggest_error_offset = 0;

    for (current_point, error_squared) in errors.enumerate() {
        if error_squared > biggest_error_squared {
            biggest_error_squared = error_squared;
            biggest_error_offset = current_point;
        }
    }

    // Indicate the biggest error and where it was
    (f32::sqrt(biggest_error_squared), biggest_error_offset)
}

///
/// Returns the unit tangent at the start of the curve
///
fn start_tangent(points: &[Vector2F]) -> Vector2F {
    (points[1] - points[0]).normalize()
}

///
/// Returns the unit tangent at the end of the curve
///
fn end_tangent(points: &[Vector2F]) -> Vector2F {
    (points[points.len() - 2] - points[points.len() - 1]).normalize()
}

///
/// Estimates the tangent between three points
///
fn tangent_between(p1: &Vector2F, p2: &Vector2F, p3: &Vector2F) -> Vector2F {
    let v1 = *p1 - *p2;
    let v2 = *p2 - *p3;

    ((v1 + v2) * 0.5).normalize()
}

///
/// Applies the newton-raphson method in order to improve the t values of a curve
///
fn reparameterize(points: &[Vector2F], chords: &[f32], curve: &BezierCurve) -> Vec<f32> {
    points
        .iter()
        .zip(chords.iter())
        .map(|(point, chord)| newton_raphson_root_find(curve, point, *chord))
        .collect()
}

///
/// Uses newton-raphson to find a root for a curve
///
fn newton_raphson_root_find(curve: &BezierCurve, point: &Vector2F, estimated_t: f32) -> f32 {
    let [start, cp1, cp2, end] = curve.clone().into_points();

    // Compute Q(t) (where Q is our curve)
    let qt = curve.eval(estimated_t);

    // Generate control vertices
    let qn1 = (cp1 - start) * 3.0;
    let qn2 = (cp2 - cp1) * 3.0;
    let qn3 = (end - cp2) * 3.0;

    let qnn1 = (qn2 - qn1) * 2.0;
    let qnn2 = (qn3 - qn2) * 2.0;

    // Compute Q'(t) and Q''(t)
    let qnt = de_casteljau3(estimated_t, qn1, qn2, qn3);
    let qnnt = de_casteljau2(estimated_t, qnn1, qnn2);

    // Compute f(u)/f'(u)
    let numerator = (qt - *point).dot(qnt);
    let denominator = qnt.dot(qnt) + (qt - *point).dot(qnnt);

    // u = u - f(u)/f'(u)
    if denominator == 0.0 {
        estimated_t
    } else {
        estimated_t - (numerator / denominator)
    }
}

///
/// de Casteljau's algorithm for cubic bezier curves
///
#[inline]
pub fn de_casteljau4(t: f32, w1: Vector2F, w2: Vector2F, w3: Vector2F, w4: Vector2F) -> Vector2F {
    let wn1 = w1 * (1.0 - t) + w2 * t;
    let wn2 = w2 * (1.0 - t) + w3 * t;
    let wn3 = w3 * (1.0 - t) + w4 * t;

    de_casteljau3(t, wn1, wn2, wn3)
}

///
/// de Casteljau's algorithm for quadratic bezier curves
///
#[inline]
pub fn de_casteljau3(t: f32, w1: Vector2F, w2: Vector2F, w3: Vector2F) -> Vector2F {
    let wn1 = w1 * (1.0 - t) + w2 * t;
    let wn2 = w2 * (1.0 - t) + w3 * t;

    de_casteljau2(t, wn1, wn2)
}

///
/// de Casteljau's algorithm for lines
///
#[inline]
pub fn de_casteljau2(t: f32, w1: Vector2F, w2: Vector2F) -> Vector2F {
    w1 * (1.0 - t) + w2 * t
}

#[inline]
fn distance(v1: &Vector2F, v2: &Vector2F) -> f32 {
    let a = (v1.x() - v2.x()).powi(2) + (v1.y() - v2.y()).powi(2);
    a.sqrt()
}
