//! Origins (zm_tomb) test aid: the tank ride.
//!
//! IW4L_T6_TANK=[<seconds>] (30 s unless given): the player, given 2000
//! points, is put on the tank's deck (`level.vh_tank.e_roof`, the
//! `vol_on_tank_watch` volume) and uses its trigger (`t_use`, 500 points),
//! a press a second until `tank_moving` is set. Every 2 s for three minutes
//! the tank's place and flags, its stop (`str_location_current`), the
//! player's place, points and whether he rides it are logged.

use std::sync::OnceLock;

use bevy_ecs::prelude::World;
use gsc_t6::{Value, Vm};

use super::{entnum, frame, with_vm};
use crate::world::ClientId;

fn want() -> Option<i64> {
    static AT: OnceLock<Option<i64>> = OnceLock::new();
    *AT.get_or_init(|| {
        let v = std::env::var("IW4L_T6_TANK").ok()?;
        Some(
            v.trim()
                .parse::<f32>()
                .map_or(30_000, |s| (s * 1000.0) as i64),
        )
    })
}

/// The tank and the entity numbers of its use trigger and deck volume.
fn tank(
    vm: &mut Vm<World>,
    world: &mut World,
) -> Option<(gsc_t6::ObjRef, u32, Option<u32>, Option<u32>)> {
    let f = vm.intern("vh_tank");
    let Value::Object(t) = vm.get_field(world, vm.level, f) else {
        return None;
    };
    let n = entnum(vm, &Value::Object(t))?;
    let (u, r) = (vm.intern("t_use"), vm.intern("e_roof"));
    let use_trig = vm.get_field(world, t, u);
    let roof = vm.get_field(world, t, r);
    Some((t, n, entnum(vm, &use_trig), entnum(vm, &roof)))
}

/// An entity's place, or its brush's middle for a brush entity (the
/// brush's bounds are about the entity's origin).
fn middle(world: &mut World, n: u32) -> Option<[f32; 3]> {
    let e = world.resource::<super::Zm>().ents.get(&n)?.clone();
    let Some(b) = super::triggers::brush_index(&e) else {
        return Some(e.origin);
    };
    let f = frame(world);
    let m = f.clip_cmodels().models.get(b as usize)?;
    Some(std::array::from_fn(|i| {
        (m.mins[i] + m.maxs[i]) * 0.5 + e.origin[i]
    }))
}

pub(crate) fn tank_test(world: &mut World, now: i64) {
    let Some(at) = want() else {
        return;
    };
    let tick = i64::from(crate::MATCH_TICK_MS);
    let client = ClientId(0);
    let t = now - at;
    if !(0..180_000).contains(&t) {
        return;
    }
    let Some((obj, n, use_trig, roof)) = with_vm(world, tank).flatten() else {
        if t < tick {
            diag::warn!(Sim, "bo2zm t6 tank test: no level.vh_tank");
        }
        return;
    };
    let use_at = use_trig.and_then(|u| middle(world, u));
    if t < tick {
        with_vm(world, |vm, world| {
            if let Some(p) = world.resource::<super::Zm>().players.get(&0).map(|p| p.obj) {
                let f = vm.intern("score");
                vm.set_field(world, p, f, Value::Int(2000));
            }
        });
        let e = world
            .resource::<super::Zm>()
            .ents
            .get(&n)
            .cloned()
            .unwrap_or_default();
        let bounds = frame(world)
            .model_capability(&e.model)
            .flatten()
            .and_then(|c| c.bounds);
        let deck = roof.and_then(|r| middle(world, r));
        diag::info!(
            Sim,
            "bo2zm t6 tank test: tank ent{n} {} at {:?} angles {:?} bounds {bounds:?} platform {}; use ent{use_trig:?} at {use_at:?}; deck ent{roof:?} at {deck:?}",
            e.model,
            e.origin,
            e.angles,
            e.platform
        );
        if let Some(d) = deck {
            super::teleport_player(world, client, [d[0], d[1], d[2] + 8.0]);
        }
    }
    let moving = with_vm(world, |vm, world| {
        super::tomb_test::ent_flag(vm, world, obj, "tank_moving")
    })
    .unwrap_or_default();
    if (1000..12_000).contains(&t) && t % 1000 < 300 && moving != "1" {
        if let Some(u) = use_at {
            super::tomb_test::look_and_use(world, client, u);
        }
    }
    if t % 2000 < tick {
        let line = with_vm(world, |vm, world| {
            let loc = vm.intern("str_location_current");
            let loc = vm.get_field(world, obj, loc);
            format!(
                "moving {moving} activated {} cooldown {} at {}",
                super::tomb_test::ent_flag(vm, world, obj, "tank_activated"),
                super::tomb_test::ent_flag(vm, world, obj, "tank_cooldown"),
                vm.to_text(&loc)
            )
        })
        .unwrap_or_default();
        let zm = world.resource::<super::Zm>();
        let o = zm.ents.get(&n).map_or([0.0; 3], |e| e.origin);
        let rides = zm.riders.on.get(&0) == Some(&n);
        let score = zm.players.get(&0).map(|p| p.obj);
        let score = score.and_then(|p| {
            with_vm(world, |vm, world| {
                let f = vm.intern("score");
                let v = vm.get_field(world, p, f);
                vm.to_text(&v)
            })
        });
        let p = frame(world).player(client).map_or([0.0; 3], |ps| ps.origin);
        diag::info!(
            Sim,
            "bo2zm t6 tank test {}s: tank ({:.0} {:.0} {:.0}) {line}; player ({:.0} {:.0} {:.0}) rides {rides} score {}",
            t / 1000,
            o[0],
            o[1],
            o[2],
            p[0],
            p[1],
            p[2],
            score.unwrap_or_default()
        );
    }
}
