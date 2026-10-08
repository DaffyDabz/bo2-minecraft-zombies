//! Die Rise (zm_highrise): its elevator cars carry players.
//!
//! A car is a script model (`p6_anim_zm_hr_elevator_common`, the freight
//! car `..._freight`) made a moving platform by `init_elevator`
//! (`setmovingplatformenabled`). Script models do not block player
//! movement here, so a car's floor and roof are kept as thin turned boxes
//! (presence's blockers) and `riders` carries whoever stands on one.

/// A car model's solid slabs in its own space: (bottom, top) heights; the
/// box's other sides are the model's bounds. The floor's top is 8 over the
/// origin (a perk machine linked to a car stands there, e.g. Jugger-Nog at
/// z 1296 in the car at 1288); the roof's top is the model's top.
pub(crate) fn platform_slabs(model: &str) -> &'static [(f32, f32)] {
    match model {
        "p6_anim_zm_hr_elevator_common" => &[(-8.0, 8.0), (130.0, 138.0)],
        "p6_anim_zm_hr_elevator_freight" => &[(-8.0, 8.0), (213.0, 221.0)],
        _ => &[],
    }
}

/// Inside a moving car: a point kept within its walls (its box in its own
/// space, a player's radius in from each side), given the car's pose.
pub(crate) fn keep_inside(
    p: [f32; 3],
    origin: [f32; 3],
    angles: [f32; 3],
    (lo, hi): ([f32; 3], [f32; 3]),
) -> [f32; 3] {
    const RADIUS: f32 = 16.0;
    let (f, r, _) = gsc_t6::math::angle_vectors(angles);
    let d = gsc_t6::math::sub(p, origin);
    let l = [gsc_t6::math::dot(d, f), -gsc_t6::math::dot(d, r)];
    let mut m = l;
    for i in 0..2 {
        let (a, b) = (lo[i] + RADIUS, hi[i] - RADIUS);
        if a < b {
            m[i] = l[i].clamp(a, b);
        }
    }
    let (dx, dy) = (m[0] - l[0], m[1] - l[1]);
    [
        p[0] + f[0] * dx - r[0] * dy,
        p[1] + f[1] * dx - r[1] * dy,
        p[2] + f[2] * dx - r[2] * dy,
    ]
}
