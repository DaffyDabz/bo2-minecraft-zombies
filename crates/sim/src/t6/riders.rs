//! Black Ops II's moving platforms (`setmovingplatformenabled(1)`:
//! Tranzit's bus and the clip linked to it): who stands on one, and
//! carrying them as it moves and turns (BO2's engine moves a player with
//! the platform under his feet). `getmoverent` names the platform: the bus
//! scripts count riders by it (`busupdateplayers`).

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;

use super::{Ent, Zm, frame};
use crate::world::ClientId;

/// Riders and the platforms' poses last tick.
#[derive(Clone, Debug, Default)]
pub(crate) struct Riders {
    /// Platform -> (origin, angles) at the end of last tick.
    pub poses: BTreeMap<u32, ([f32; 3], [f32; 3])>,
    /// Player -> the platform he stands on (the root of its links).
    pub on: BTreeMap<u32, u32>,
}

/// A platform's box in its own space (mins, maxs): a brush entity's
/// submodel, a model's bounds.
fn local_box(world: &mut World, e: &Ent) -> Option<([f32; 3], [f32; 3])> {
    if let Some(n) = super::triggers::brush_index(e) {
        let f = frame(world);
        let m = f.clip_cmodels().models.get(n as usize)?;
        return Some((m.mins, m.maxs));
    }
    if e.model.is_empty() {
        return None;
    }
    let (mid, half) = frame(world).model_capability(&e.model).flatten()?.bounds?;
    Some((
        std::array::from_fn(|i| mid[i] - half[i]),
        std::array::from_fn(|i| mid[i] + half[i]),
    ))
}

/// A world point in an entity's own space (IW axes: forward, left, up).
fn to_local(p: [f32; 3], origin: [f32; 3], angles: [f32; 3]) -> [f32; 3] {
    let (f, r, u) = gsc_t6::math::angle_vectors(angles);
    let d = gsc_t6::math::sub(p, origin);
    [
        gsc_t6::math::dot(d, f),
        -gsc_t6::math::dot(d, r),
        gsc_t6::math::dot(d, u),
    ]
}

/// The scripted solid under a point (`getgroundent`): the smallest
/// script brush model or moving platform whose box holds it (its top a few
/// units under it at most). Brush model hits come back from the traces as
/// the world, so Die Rise's elevators are found this way
/// (`object_is_on_elevator`: a piece or equipment placed in one).
pub(crate) fn ground_ent(world: &mut World, p: [f32; 3]) -> Option<u32> {
    let cands: Vec<(u32, Ent)> = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter(|(_, e)| !e.hidden && (e.platform || e.classname == "script_brushmodel"))
        .map(|(n, e)| (*n, e.clone()))
        .collect();
    let mut best: Option<(u32, f32)> = None;
    for (n, e) in cands {
        let Some((lo, hi)) = local_box(world, &e) else {
            continue;
        };
        let l = to_local(p, e.origin, e.angles);
        let inside = (0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i])
            && l[2] >= lo[2] - 2.0
            && l[2] <= hi[2] + 4.0;
        let vol = (hi[0] - lo[0]) * (hi[1] - lo[1]) * (hi[2] - lo[2]).max(1.0);
        if inside && best.is_none_or(|(_, v)| vol < v) {
            best = Some((n, vol));
        }
    }
    best.map(|(n, _)| n)
}

/// The platform an entity is linked under (its link chain's root).
fn root(zm: &Zm, mut n: u32) -> u32 {
    for _ in 0..8 {
        match zm.ents.get(&n).and_then(|e| e.link) {
            Some((p, _, _)) if zm.ents.get(&p).is_some_and(|e| e.platform) => n = p,
            _ => break,
        }
    }
    n
}

/// Each tick, after the vehicles and linked entities moved: each player
/// on the ground inside a platform's box (as it stood last tick, where his
/// move was made against it) rides it: he is moved and turned as it moved
/// and turned since. Riders are kept for `getmoverent`.
pub(crate) fn advance(world: &mut World) {
    ride_test(world);
    let plats: Vec<(u32, Ent)> = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter(|(_, e)| e.platform && !e.hidden)
        .map(|(n, e)| (*n, e.clone()))
        .collect();
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let prev = std::mem::take(&mut world.resource_mut::<Zm>().riders.poses);
    let mut on = BTreeMap::new();
    let log = std::env::var_os("IW4L_T6_RIDELOG").is_some();
    // Boxes once per tick.
    let boxes: Vec<(u32, ([f32; 3], [f32; 3]), Ent)> = plats
        .into_iter()
        .filter_map(|(n, e)| local_box(world, &e).map(|b| (n, b, e)))
        .collect();
    if log {
        for (n, (lo, hi), e) in boxes.iter().filter(|(n, _, _)| !prev.contains_key(n)) {
            diag::info!(
                Sim,
                "bo2zm t6 platform ent{n} {} {} at ({:.0} {:.0} {:.0}) box ({:.0} {:.0} {:.0})..({:.0} {:.0} {:.0})",
                e.classname,
                if e.model.is_empty() {
                    e.brush.as_deref().unwrap_or("")
                } else {
                    &e.model
                },
                e.origin[0],
                e.origin[1],
                e.origin[2],
                lo[0],
                lo[1],
                lo[2],
                hi[0],
                hi[1],
                hi[2]
            );
        }
    }
    let was = world.resource::<Zm>().riders.on.clone();
    for p in players {
        let client = ClientId(p);
        let Some(ps) = frame(world).player(client).copied() else {
            continue;
        };
        let grounded = ps.ground_entity_num != playerstate_iw4::ENTITYNUM_NONE
            || ps.pm_flags & playerstate_iw4::pm_flags::LADDER != 0;
        // The smallest box he stands in or on wins (the clip on the bus
        // over the bus's own model). In the air, only a vehicle's box holds
        // him (`floor_z` keeps him on its floor). One already riding a
        // platform stays on it while in its box, in the air too: a Die Rise
        // elevator going down drops away under his feet each tick, and he
        // falls back onto its floor.
        let mut best: Option<(u32, f32, ([f32; 3], [f32; 3]))> = None;
        for (n, (lo, hi), e) in &boxes {
            if !grounded
                && e.classname != "script_vehicle"
                && was.get(&p) != Some(&root(&world.resource::<Zm>(), *n))
            {
                continue;
            }
            let (o, a) = prev.get(n).copied().unwrap_or((e.origin, e.angles));
            let l = to_local(ps.origin, o, a);
            let inside = (0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i])
                && l[2] >= lo[2] - 2.0
                && l[2] <= hi[2] + 4.0;
            if !inside {
                continue;
            }
            // A Die Rise car holds only who stands on its floor or roof
            // (one in the doorway as it comes up from below is not lifted);
            // the trigger linked to it adds nothing.
            let zm = world.resource::<Zm>();
            let car = |m: u32| {
                zm.ents
                    .get(&m)
                    .map_or(&[][..], |r| super::highrise::platform_slabs(&r.model))
            };
            if !car(root(&zm, *n)).is_empty() {
                let slabs = car(*n);
                if !slabs
                    .iter()
                    .any(|&(_, top)| l[2] >= top - 6.0 && l[2] <= top + 60.0)
                {
                    continue;
                }
            }
            let vol = (hi[0] - lo[0]) * (hi[1] - lo[1]) * (hi[2] - lo[2]).max(1.0);
            if best.is_none_or(|(_, v, _)| vol < v) {
                best = Some((*n, vol, (*lo, *hi)));
            }
        }
        let Some((plat, _, plat_box)) = best else {
            if log && let Some(c) = was.get(&p) {
                let at = world.resource::<Zm>().ents.get(c).map(|e| e.origin);
                diag::info!(
                    Sim,
                    "bo2zm t6 rider {p} off ent{c} (at {at:?}): at ({:.0} {:.0} {:.0}) grounded {grounded}",
                    ps.origin[0],
                    ps.origin[1],
                    ps.origin[2]
                );
            }
            continue;
        };
        let carrier = root(&world.resource::<Zm>(), plat);
        on.insert(p, carrier);
        hold_on_floor(world, client, carrier);
        let Some(ps) = frame(world).player(client).copied() else {
            continue;
        };
        // Carried by the root's move (all its linked pieces move with it).
        let Some(&(o0, a0)) = prev.get(&carrier) else {
            continue;
        };
        let Some((o1, a1)) = world
            .resource::<Zm>()
            .ents
            .get(&carrier)
            .map(|e| (e.origin, e.angles))
        else {
            continue;
        };
        let dyaw = math_iw4::angle_subtract(a1[1], a0[1]);
        if o0 == o1 && dyaw == 0.0 {
            if log && world.resource::<Zm>().now_ms % 500 < crate::MATCH_TICK_MS as i64 {
                let l = to_local(ps.origin, o1, a1);
                diag::info!(
                    Sim,
                    "bo2zm t6 rider {p} on ent{carrier} (bus still) local ({:.0} {:.0} {:.0})",
                    l[0],
                    l[1],
                    l[2]
                );
            }
            continue;
        }
        // A Die Rise car's doors are shut while it moves: its rider stays
        // inside its walls (the model is not solid to him here).
        let mut from = ps.origin;
        let is_car = world
            .resource::<Zm>()
            .ents
            .get(&plat)
            .is_some_and(|e| !super::highrise::platform_slabs(&e.model).is_empty());
        if is_car && o0 != o1 {
            from = super::highrise::keep_inside(from, o0, a0, plat_box);
        }
        let (s, c) = dyaw.to_radians().sin_cos();
        let d = gsc_t6::math::sub(from, o0);
        let to = [
            o1[0] + d[0] * c - d[1] * s,
            o1[1] + d[0] * s + d[1] * c,
            o1[2] + d[2] + 0.25,
        ];
        let mut f = frame(world);
        if let Some(ps) = f.player_mut(client) {
            ps.origin = to;
            ps.delta_angles[1] += dyaw;
            ps.viewangles[1] += dyaw;
        }
        f.translate_player_area(client, gsc_t6::math::sub(to, ps.origin));
        if log && world.resource::<Zm>().now_ms % 500 < crate::MATCH_TICK_MS as i64 {
            let l = to_local(to, o1, a1);
            diag::info!(
                Sim,
                "bo2zm t6 rider {p} on ent{carrier} (via ent{plat}) at ({:.0} {:.0} {:.0}) local ({:.0} {:.0} {:.0})",
                to[0],
                to[1],
                to[2],
                l[0],
                l[1],
                l[2]
            );
        }
    }
    let poses = world
        .resource::<Zm>()
        .ents
        .iter()
        .filter(|(_, e)| e.platform)
        .map(|(n, e)| (*n, (e.origin, e.angles)))
        .collect();
    let mut zm = world.resource_mut::<Zm>();
    if log && zm.riders.on != on {
        diag::info!(Sim, "bo2zm t6 riders now {:?} (player -> platform)", on);
    }
    zm.riders.poses = poses;
    zm.riders.on = on;
}

/// A vehicle's floor height in its own space (IW4L_T6_BUSFLOOR, default
/// 40: the Tranzit bus's deck over its origin at the road).
fn floor_z() -> f32 {
    static Z: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *Z.get_or_init(|| {
        std::env::var("IW4L_T6_BUSFLOOR")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(super::transit::BUS_FLOOR)
    })
}

/// The bus's floor is a blocker (`transit`); one inside it is also held up
/// at its floor (falling stops there), in case a carry step left him in it.
fn hold_on_floor(world: &mut World, client: ClientId, carrier: u32) {
    let Some((o, a)) = world
        .resource::<Zm>()
        .ents
        .get(&carrier)
        .filter(|e| e.classname == "script_vehicle")
        .map(|e| (e.origin, e.angles))
    else {
        return;
    };
    let mut f = frame(world);
    let Some(ps) = f.player_mut(client) else {
        return;
    };
    let l = to_local(ps.origin, o, a);
    // Only between its walls: one beside it (in its box) stays on the road.
    let (lo, hi) = super::transit::BUS_INSIDE;
    if !(0..2).all(|i| l[i] >= lo[i] && l[i] <= hi[i]) {
        return;
    }
    let lift = floor_z() - l[2];
    if lift <= 0.0 {
        return;
    }
    ps.origin[2] += lift;
    ps.velocity[2] = ps.velocity[2].max(0.0);
    f.translate_player_area(client, [0.0, 0.0, lift]);
}

/// IW4L_T6_RIDE="x y z" (the bus's own space; default "0 0 80"): at 15 s
/// player 0 is put there in the first script vehicle that is a platform
/// (Tranzit's bus), once: a test of riding it.
fn ride_test(world: &mut World) {
    static AT: std::sync::OnceLock<Option<[f32; 3]>> = std::sync::OnceLock::new();
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(off) = *AT.get_or_init(|| {
        let v = std::env::var("IW4L_T6_RIDE").ok()?;
        let n: Vec<f32> = v
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        Some(if n.len() == 3 {
            [n[0], n[1], n[2]]
        } else {
            [0.0, 0.0, 80.0]
        })
    }) else {
        return;
    };
    let zm = world.resource::<Zm>();
    // IW4L_T6_RIDE_AT=<seconds>: when (default 15).
    let at = std::env::var("IW4L_T6_RIDE_AT")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map_or(15_000, |s| (s * 1000.0) as i64);
    if zm.now_ms < at || DONE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some((o, a)) = zm
        .ents
        .values()
        .find(|e| e.platform && e.classname == "script_vehicle")
        .map(|e| (e.origin, e.angles))
    else {
        return;
    };
    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (f, r, u) = gsc_t6::math::angle_vectors(a);
    let to: [f32; 3] =
        std::array::from_fn(|i| o[i] + f[i] * off[0] - r[i] * off[1] + u[i] * off[2]);
    diag::info!(
        Sim,
        "bo2zm t6 ride test: player 0 put in the bus at ({:.0} {:.0} {:.0})",
        to[0],
        to[1],
        to[2]
    );
    super::teleport_player(world, ClientId(0), to);
    // IW4L_T6_RIDE_YAW=<degrees>: he faces that way from the bus's front.
    let yaw: f32 = std::env::var("IW4L_T6_RIDE_YAW")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0.0);
    super::set_player_view(world, ClientId(0), [0.0, a[1] + yaw, 0.0]);
}
