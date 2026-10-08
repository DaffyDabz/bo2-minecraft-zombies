//! Minecraft Zombies: a dead mob falls as a ragdoll, as the mob ragdoll
//! mods have it. Its body is a box that tumbles under gravity, lands on the
//! ground its server body stands on, bounces and slides to rest; its legs,
//! arms, tails and wings swing loose toward the ground (never into it) and
//! shake on each landing. Vanilla's 20-tick tip over is kept wherever the
//! dead stay vanilla's 20 ticks.

use glam::{DVec3, Quat, Vec3};
use std::cell::Cell;

/// `LivingEntity` gravity and air drag, blocks a tick.
const GRAVITY: f32 = 0.08;
const DRAG: f32 = 0.98;
/// How much of a landing bounces back, and the ground's grip.
const RESTITUTION: f32 = 0.25;
const FRICTION: f32 = 0.6;
const SUBSTEPS: usize = 4;
/// How far a limb swings loose from its pose, at most.
const LIMB_REACH: f32 = 75.0;

/// Ragdolls show while the dead stay longer than vanilla's 20 ticks.
pub fn enabled() -> bool {
    minecraftoss_entities::health::corpse_ticks() > 20
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// A number in -1..1 from a seed and a salt.
fn signed(seed: u32, salt: u32) -> f32 {
    (hash(seed ^ salt.wrapping_mul(0x9e37_79b9)) as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// One dead mob's falling body.
#[derive(Clone, Copy, Debug)]
pub struct Ragdoll {
    /// Half its box: a little slimmer than the hitbox, as the models are.
    half: Vec3,
    /// The inverse of its box's inertia, about its own axes.
    inv_inertia: Vec3,
    centre: DVec3,
    centre_o: DVec3,
    rot: Quat,
    rot_o: Quat,
    vel: Vec3,
    spin: Vec3,
    ground: f64,
    /// Its server body's height last tick.
    server_y: f64,
    /// Ticks its server body has not moved up or down.
    server_rest: i32,
    ticks: i32,
    still: i32,
    /// The last landings: tick and strength (0..1).
    impacts: [(f32, f32); 4],
    seed: u32,
}

/// What a renderer needs of a ragdoll at a partial tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RagdollPose {
    /// The body's turn, model to world, as `body_rotation` gives it.
    pub rot: Quat,
    /// Ticks since death.
    pub t: f32,
    pub impacts: [(f32, f32); 4],
    pub seed: u32,
    pub ground: f64,
}

impl Ragdoll {
    /// At death: standing at `feet`, moving `motion` a tick (the killing
    /// hit's knockback), facing `body_rot`.
    pub fn new(feet: DVec3, motion: DVec3, body_rot: f32, size: (f32, f32), seed: u32) -> Self {
        let (w, h) = (size.0.max(0.2), size.1.max(0.2));
        // A four-legged mob's body is long, nose to tail.
        let four_legged = w >= h * 0.5;
        let half = if four_legged { Vec3::new(w * 0.35, h * 0.45, w * 0.6) } else { Vec3::new(w * 0.35, h * 0.5, w * 0.35) };
        let (x, y, z) = (half.x * 2.0, half.y * 2.0, half.z * 2.0);
        let inertia = Vec3::new((y * y + z * z) / 12.0, (x * x + z * z) / 12.0, (x * x + y * y) / 12.0);
        let rot = Quat::from_rotation_y((180.0 - body_rot).to_radians());
        let mut vel = motion.as_vec3();
        if vel.length() > 0.6 {
            vel = vel.normalize() * 0.6;
        }
        vel.y += 0.05;
        // It tips over the way the hit pushed it, else a way of its own.
        let flat = Vec3::new(vel.x, 0.0, vel.z);
        let dir = if flat.length() > 0.01 {
            flat.normalize()
        } else {
            let a = signed(seed, 1) * std::f32::consts::PI;
            Vec3::new(a.cos(), 0.0, a.sin())
        };
        // A four-legged one rolls onto its side, not up on its tail.
        let dir = if four_legged {
            let side = rot * Vec3::X;
            let toward = dir.dot(side);
            side * if toward.abs() > 0.05 { toward.signum() } else { signed(seed, 6).signum() }
        } else {
            dir
        };
        let wobble = if four_legged { 0.1 } else { 0.25 };
        let axis = (Vec3::Y.cross(dir) + Vec3::new(signed(seed, 2), 0.0, signed(seed, 3)) * wobble).normalize();
        let speed = (0.2 + flat.length() * 0.6) * (1.0 + signed(seed, 4) * 0.2);
        let spin = axis * speed + Vec3::Y * signed(seed, 5) * 0.06;
        // Already leaning most of the way to its tipping point, so even a
        // still, squat body goes over.
        let side = if four_legged { half.x } else { half.x.max(half.z) };
        let rot = Quat::from_axis_angle(axis, ((side / half.y).atan() * 0.8).max(0.3)) * rot;
        let mut centre = feet + (rot * Vec3::new(0.0, half.y, 0.0)).as_dvec3();
        let lowest = (0..8)
            .map(|i| {
                let s = Vec3::new(
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                );
                centre.y + f64::from((rot * (half * s)).y)
            })
            .fold(f64::MAX, f64::min);
        centre.y += feet.y - lowest;
        Self {
            half,
            inv_inertia: inertia.recip(),
            centre,
            centre_o: centre,
            rot,
            rot_o: rot,
            vel,
            spin,
            ground: feet.y,
            server_y: feet.y,
            server_rest: 0,
            ticks: 0,
            still: 0,
            impacts: [(-100.0, 0.0); 4],
            seed,
        }
    }

    fn inv_world(&self, x: Vec3) -> Vec3 {
        self.rot * (self.inv_inertia * (self.rot.inverse() * x))
    }

    fn apply(&mut self, impulse: Vec3, r: Vec3) {
        self.vel += impulse;
        self.spin += self.inv_world(r.cross(impulse));
    }

    fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            let s = Vec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            self.rot * (self.half * s)
        })
    }

    /// One client tick. `feet` is where its server body lies, and the body
    /// never strays far from it. The ground is where that body comes to rest,
    /// or follows it down a real fall (off a ledge, or killed in the air); a
    /// knockback hop or a slow sink as it dies leaves the ground where it was.
    pub fn tick(&mut self, feet: DVec3) {
        self.centre_o = self.centre;
        self.rot_o = self.rot;
        self.ticks += 1;
        let drop = self.server_y - feet.y;
        self.server_y = feet.y;
        self.server_rest = if drop.abs() < 0.005 { self.server_rest + 1 } else { 0 };
        if self.server_rest >= 2 || drop > 0.2 {
            self.ground = feet.y;
        }
        let lowest = self.corners().iter().map(|r| self.centre.y + f64::from(r.y)).fold(f64::MAX, f64::min);
        if lowest > self.ground + 0.02 {
            self.still = 0;
        }
        if self.still >= 5 {
            return;
        }
        let dt = 1.0 / SUBSTEPS as f32;
        let mut hit = 0.0f32;
        let mut touching = false;
        for _ in 0..SUBSTEPS {
            self.vel.y -= GRAVITY * dt;
            self.centre += (self.vel * dt).as_dvec3();
            let turn = self.spin.length() * dt;
            if turn > 1e-6 {
                self.rot = (Quat::from_axis_angle(self.spin.normalize(), turn) * self.rot).normalize();
            }
            let mut deepest = 0.0f32;
            for r in self.corners() {
                let depth = (self.ground - (self.centre.y + f64::from(r.y))) as f32;
                if depth <= 0.0 {
                    continue;
                }
                touching = true;
                deepest = deepest.max(depth);
                let vp = self.vel + self.spin.cross(r);
                if vp.y >= 0.0 {
                    continue;
                }
                let (rot, inv) = (self.rot, self.inv_inertia);
                let k = move |d: Vec3| 1.0 + d.dot((rot * (inv * (rot.inverse() * r.cross(d)))).cross(r));
                let j = -(1.0 + RESTITUTION) * vp.y / k(Vec3::Y);
                self.apply(Vec3::Y * j, r);
                hit = hit.max(j);
                let vp = self.vel + self.spin.cross(r);
                let slide = Vec3::new(vp.x, 0.0, vp.z);
                let speed = slide.length();
                if speed > 1e-5 {
                    let t = slide / speed;
                    let jt = (speed / k(t)).min(FRICTION * j);
                    self.apply(-t * jt, r);
                }
            }
            if deepest > 0.0 {
                self.centre.y += f64::from(deepest) * 0.8;
            }
        }
        self.vel *= DRAG;
        self.spin *= if touching { 0.9 } else { 0.98 };
        // Its server body is where it really lies.
        let off = DVec3::new(self.centre.x - feet.x, 0.0, self.centre.z - feet.z);
        if off.length() > 0.75 {
            let keep = feet + off.normalize() * 0.75;
            self.centre.x = keep.x;
            self.centre.z = keep.z;
        }
        let last = self.impacts[0].0;
        if hit > 0.03 && self.ticks as f32 - last > 3.0 {
            self.impacts.rotate_right(1);
            self.impacts[0] = (self.ticks as f32, (hit * 4.0).min(1.0));
        }
        if touching && self.vel.length() < 0.01 && self.spin.length() < 0.01 {
            self.still += 1;
        } else {
            self.still = 0;
        }
    }

    /// Its feet and pose at a partial tick.
    pub fn pose(&self, partial: f32) -> (DVec3, RagdollPose) {
        let centre = self.centre_o.lerp(self.centre, f64::from(partial));
        let rot = self.rot_o.slerp(self.rot, partial);
        let feet = centre - (rot * Vec3::new(0.0, self.half.y, 0.0)).as_dvec3();
        (feet, RagdollPose { rot, t: self.ticks as f32 + partial, impacts: self.impacts, seed: self.seed, ground: self.ground })
    }
}

thread_local! {
    /// The ragdoll the renderer is drawing, by its feet.
    static LIMP: Cell<Option<(DVec3, RagdollPose)>> = const { Cell::new(None) };
}

/// The mob about to be drawn (`MobPose::body_rotation`).
pub(crate) fn set_drawing(feet: DVec3, pose: Option<RagdollPose>) {
    LIMP.set(pose.map(|p| (feet, p)));
}

/// `ModelPart` space (1/16 block, Y down, feet at 24) to the mob's own.
fn model_to_entity(p: Vec3, scale: Vec3) -> Vec3 {
    Vec3::new(-p.x, 24.016 - p.y, p.z) * scale / 16.0
}

/// A cube's extra turn about its pivot while its mob is a ragdoll: a limb
/// (a long thin cube hanging from its pivot) swings toward the ground,
/// never through it, and shakes on landings. Others keep their pose.
pub(crate) fn limb_turn(feet: DVec3, rotation: Quat, scale: Vec3, from: [f32; 3], to: [f32; 3], pivot: [f32; 3], part: Quat) -> Quat {
    let Some((at, pose)) = LIMP.get() else {
        return Quat::IDENTITY;
    };
    if at.distance_squared(feet) > 1e-8 {
        return Quat::IDENTITY;
    }
    let size = Vec3::from_array(to) - Vec3::from_array(from);
    let a = if size.x >= size.y && size.x >= size.z {
        0
    } else if size.y >= size.z {
        1
    } else {
        2
    };
    let long = size[a];
    let mut rest = [size.x, size.y, size.z];
    rest.sort_by(f32::total_cmp);
    let mid = rest[1];
    let volume = size.x * size.y * size.z;
    if !(20.0..=800.0).contains(&volume) || long < mid * 1.45 || (mid > 5.0 && long < mid * 3.0) {
        return Quat::IDENTITY;
    }
    let (lo, hi) = (from[a], to[a]);
    let reach = if hi.abs() >= lo.abs() { hi } else { lo };
    // It hangs from its pivot (down, or out to a side), unlike a head.
    if reach.abs() < long * 0.7 || (a == 1 && reach < 0.0) {
        return Quat::IDENTITY;
    }
    let mut axis = Vec3::ZERO;
    axis[a] = reach.signum();
    let dir = part * axis;
    let length = reach.abs();
    // Down, in model space.
    let e = rotation.inverse() * Vec3::NEG_Y;
    let down = Vec3::new(-e.x, -e.y, e.z);
    let key = hash(pose.seed ^ (pivot[0].to_bits().rotate_left(7) ^ pivot[1].to_bits().rotate_left(13) ^ pivot[2].to_bits()));
    let t = pose.t;
    // Settling in with a little overshoot.
    let settle = 1.0 - (-t / 5.0).exp() * (0.8 * t).cos();
    let toward = Quat::from_rotation_arc(dir, down);
    let (toward_axis, angle) = toward.to_axis_angle();
    let angle = angle.min(LIMB_REACH.to_radians()) * 0.85;
    let sprawl = Quat::from_euler(glam::EulerRot::XZY, signed(key, 1) * 0.35, signed(key, 2) * 0.35, 0.0);
    let ground_y = pose.ground;
    let pivot_v = Vec3::from_array(pivot);
    let tip_y = |q: Quat| feet.y + f64::from((rotation * model_to_entity(pivot_v + q * dir * length, scale)).y);
    // The furthest it swings with its tip above the ground.
    let mut amount = settle.clamp(0.0, 1.2);
    let turn = |f: f32| Quat::from_axis_angle(toward_axis, angle * f) * Quat::IDENTITY.slerp(sprawl, f.min(1.0));
    if tip_y(turn(amount)) < ground_y + 0.02 {
        let (mut ok, mut bad) = (0.0f32, amount);
        if tip_y(turn(0.0)) >= ground_y + 0.02 {
            for _ in 0..6 {
                let m = (ok + bad) * 0.5;
                if tip_y(turn(m)) >= ground_y + 0.02 {
                    ok = m;
                } else {
                    bad = m;
                }
            }
        }
        amount = ok;
    }
    // Each landing shakes it about an axis across it.
    let mut shake = 0.0;
    for (at, strength) in pose.impacts {
        let s = t - at;
        if s >= 0.0 && s < 30.0 {
            shake += strength * (-s / 4.0).exp() * (1.2 * s).sin() * 0.6;
        }
    }
    let across = dir.cross(Vec3::new(signed(key, 3), signed(key, 4), signed(key, 5))).normalize_or(Vec3::X);
    Quat::from_axis_angle(across, shake) * turn(amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_and_rests_by_its_feet() {
        for (size, motion) in [((0.6, 1.99), DVec3::new(0.3, 0.1, 0.0)), ((0.9, 1.4), DVec3::ZERO), ((0.4, 0.7), DVec3::new(0.0, 0.0, -0.2)), ((0.9, 0.9), DVec3::new(0.1, 0.1, 0.4)), ((1.4, 0.9), DVec3::new(-0.3, 0.0, 0.1)), ((0.9, 1.3), DVec3::new(0.0, 0.0, 0.5))] {
            let feet = DVec3::new(10.5, 64.0, -3.5);
            let mut r = Ragdoll::new(feet, motion, 37.0, size, 12345);
            for t in 0..100 {
                r.tick(feet);
                let (f, p) = r.pose(0.5);
                if t % 10 == 0 || !f.is_finite() || !p.rot.is_finite() {
                    eprintln!("{size:?} t{t} feet {f:.2} rot {:.2} centre {:.2}", p.rot, r.centre);
                }
                assert!(f.is_finite() && p.rot.is_finite(), "nan at {t}");
                assert!((r.centre - feet).length() < 2.0, "flew off at {t}: {:?}", r.centre);
            }
            // It lies down: its up leans more than 60 degrees.
            let up = r.rot * Vec3::Y;
            assert!(up.y < 0.5, "{size:?} still standing: up {up:?}");
            // A four-legged one on its side: nose to tail stays level.
            if size.0 >= size.1 * 0.5 {
                let long = r.rot * Vec3::Z;
                assert!(long.y.abs() < 0.5, "{size:?} up on an end: {long:?}");
            }
            let lowest = r.corners().iter().map(|c| r.centre.y + f64::from(c.y)).fold(f64::MAX, f64::min);
            assert!((lowest - feet.y).abs() < 0.1, "{size:?} not on the ground: {lowest}");
        }
    }

    #[test]
    fn every_mob_lies_down_however_it_died() {
        let mut failed = Vec::new();
        for size in [(0.6, 1.95), (0.9, 0.9), (0.9, 1.4), (0.4, 0.7), (1.4, 0.9), (0.6, 1.7), (0.6, 0.85), (0.9, 1.3)] {
            for seed in 0..60u32 {
                let a = seed as f64 * 0.7;
                let motion = if seed % 3 == 0 { DVec3::ZERO } else { DVec3::new(a.cos(), 0.3, a.sin()) * (0.05 * f64::from(seed % 7)) };
                let feet = DVec3::new(0.5, 64.0, 0.5);
                let mut r = Ragdoll::new(feet, motion, seed as f32 * 37.0, size, hash(seed));
                for _ in 0..100 {
                    r.tick(feet);
                }
                let (up, long) = (r.rot * Vec3::Y, r.rot * Vec3::Z);
                if up.y > 0.5 || (size.0 >= size.1 * 0.5 && long.y.abs() > 0.5) {
                    failed.push((size, seed, up.y, long.y));
                }
            }
        }
        assert!(failed.is_empty(), "{} stood up: {failed:?}", failed.len());
    }

    #[test]
    fn a_sinking_server_body_keeps_it_on_the_floor() {
        let feet = DVec3::new(0.5, 64.0, 0.5);
        let mut r = Ragdoll::new(feet, DVec3::ZERO, 0.0, (0.6, 1.99), 7);
        for t in 0..100 {
            r.tick(feet - DVec3::new(0.0, 0.055 * f64::from(t), 0.0));
        }
        assert_eq!(r.ground, 64.0);
        // A real fall is followed down.
        let mut r = Ragdoll::new(feet, DVec3::ZERO, 0.0, (0.6, 1.99), 7);
        let mut y = 64.0;
        let mut v = 0.0;
        for _ in 0..40 {
            v = (v + 0.08) * 0.98;
            y = f64::max(y - v, 60.0);
            r.tick(DVec3::new(0.5, y, 0.5));
        }
        assert!(r.ground < 60.5, "ground {}", r.ground);
        // A knockback hop lands back where it was.
        let mut r = Ragdoll::new(feet, DVec3::ZERO, 0.0, (0.6, 1.99), 7);
        let (mut y, mut v) = (64.0, 0.4);
        for _ in 0..40 {
            y = f64::max(y + v, 64.0);
            v = (v - 0.08) * 0.98;
            r.tick(DVec3::new(0.5, y, 0.5));
        }
        assert_eq!(r.ground, 64.0);
    }
}
