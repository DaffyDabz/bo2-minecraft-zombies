//! Origins (zm_tomb) test aids.
//!
//! IW4L_T6_GEN=<generator>[@<seconds>] (generator_start_bunker,
//! generator_tank_trench, generator_mid_trench, generator_nml_left,
//! generator_nml_right, generator_church): at that time (20 s unless given)
//! the player is put beside that generator, looking at it, and holds use
//! for half a second, again every 2 s until the capture starts (200
//! points). From then on the zone's state is logged every 5 s for two
//! minutes. Several generators, comma separated, are taken one after
//! another, 40 s apart (run with IW4L_T6_OPENALL=buy and IW4L_T6_GOD=1).
//!
//! IW4L_T6_GEN_PAP=1 (with IW4L_T6_GEN): 40 s after the last generator the
//! player, given 10000 points, stands at Pack-a-Punch and uses it with his
//! gun, takes it back 10 s later; `all_zones_captured`, his weapons and
//! points are logged.
//!
//! IW4L_T6_GEN_PERKS=1 (with IW4L_T6_GEN): 25 s after Pack-a-Punch's time
//! the player visits each perk machine (Quick Revive, Juggernog, Speed Cola,
//! Stamin-Up, Mule Kick), 12 s each, with 10000 points, uses it and logs
//! his perks, points and the machine's hint.
//!
//! IW4L_T6_DIG=[<seconds>] (30 s unless given): the player picks up the
//! nearest shovel, then digs the nearest dig mound every 10 s, five times;
//! his shovel, weapons, points, the mounds left and the power-ups on the
//! ground are logged after each.

use std::sync::OnceLock;

use bevy_ecs::prelude::World;
use gsc_t6::{Key, Value, Vm};

use super::{frame, with_vm};
use crate::world::ClientId;

/// Time between generators in a list.
const SPAN: i64 = 40_000;

fn want() -> Option<&'static (Vec<String>, i64)> {
    static WANT: OnceLock<Option<(Vec<String>, i64)>> = OnceLock::new();
    WANT.get_or_init(|| {
        let v = std::env::var("IW4L_T6_GEN").ok()?;
        let (names, at) = match v.split_once('@') {
            Some((n, s)) => (n, s.parse::<f32>().map_or(20_000, |s| (s * 1000.0) as i64)),
            None => (v.as_str(), 20_000),
        };
        Some((names.split(',').map(str::to_owned).collect(), at))
    })
    .as_ref()
}

/// `level.zone_capture.zones[name]`.
fn zone(vm: &mut Vm<World>, host: &mut World, name: &str) -> Option<gsc_t6::ObjRef> {
    let zc = vm.intern("zone_capture");
    let Value::Object(zc) = vm.get_field(host, vm.level, zc) else {
        return None;
    };
    let zones = vm.intern("zones");
    let Value::Array(a) = vm.get_field(host, zc, zones) else {
        return None;
    };
    match a.get(&Key::Str(vm.intern(name))) {
        Some(Value::Object(o)) => Some(o),
        _ => None,
    }
}

fn vec_field(vm: &mut Vm<World>, host: &mut World, o: gsc_t6::ObjRef, f: &str) -> [f32; 3] {
    let f = vm.intern(f);
    match vm.get_field(host, o, f) {
        Value::Vec3(v) => v,
        _ => [0.0; 3],
    }
}

/// `self.ent_flag[name]` as 0/1, or "-" when unset.
pub(super) fn ent_flag(vm: &mut Vm<World>, host: &mut World, o: gsc_t6::ObjRef, name: &str) -> String {
    let f = vm.intern("ent_flag");
    match vm.get_field(host, o, f) {
        Value::Array(a) => a
            .get(&Key::Str(vm.intern(name)))
            .map_or("-".to_owned(), |v| u8::from(gsc_t6::truthy(&v)).to_string()),
        _ => "-".to_owned(),
    }
}

fn level_flag(vm: &mut Vm<World>, name: &str) -> bool {
    let f = vm.intern("flag");
    match vm.raw_field(vm.level, f) {
        Value::Array(a) => a
            .get(&Key::Str(vm.intern(name)))
            .is_some_and(|v| gsc_t6::truthy(&v)),
        _ => false,
    }
}

/// The first of 8 headings (from `yaw0`) `dist` out from `c` where the
/// player fits on a floor near `c`'s height.
pub(super) fn place_near(world: &mut World, c: [f32; 3], yaw0: f32, dist: f32) -> Option<[f32; 3]> {
    (0..8).find_map(|k| {
        let yaw = (yaw0 + 45.0 * k as f32).to_radians();
        let p = [
            c[0] + dist * yaw.cos(),
            c[1] + dist * yaw.sin(),
            c[2] + 40.0,
        ];
        let down = [p[0], p[1], p[2] - 200.0];
        let t = frame(world).trace_static_world(
            p,
            down,
            crate::bullet_collision::PLAYER_MINS,
            crate::bullet_collision::PLAYER_MAXS,
            crate::bullet_collision::MASK_PLAYER_SOLID,
        );
        let f: [f32; 3] = std::array::from_fn(|i| p[i] + (down[i] - p[i]) * t.fraction);
        (t.startsolid == 0 && t.fraction < 1.0 && (f[2] - c[2]).abs() < 48.0).then_some(f)
    })
}

/// The player looks at `at` and holds use this tick (added to the
/// autoplay's own keys, which ran first; no shooting meanwhile).
pub(super) fn look_and_use(world: &mut World, client: ClientId, at: [f32; 3]) {
    let eye = frame(world)
        .player_mut(client)
        .map(|p| [p.origin[0], p.origin[1], p.origin[2] + 60.0]);
    if let Some(eye) = eye {
        let d = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]];
        let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let view = [
            -d[2].atan2(flat.max(1.0)).to_degrees(),
            d[1].atan2(d[0]).to_degrees(),
            0.0,
        ];
        super::set_player_view(world, client, view);
    }
    let mut req = world.resource_mut::<crate::step::StepRequest>();
    for (id, cmd) in &mut req.input.cmds {
        if *id == client {
            cmd.buttons |= playerstate_iw4::buttons::USE;
            cmd.buttons &= !playerstate_iw4::buttons::ATTACK;
        }
    }
}

pub(crate) fn generator_test(world: &mut World, now: i64) {
    let Some((names, at0)) = want() else {
        return;
    };
    if std::env::var_os("IW4L_T6_GEN_PAP").is_some() {
        pap_steps(world, *at0 + SPAN * names.len() as i64, now);
    }
    if std::env::var_os("IW4L_T6_GEN_PERKS").is_some() {
        perk_steps(world, *at0 + SPAN * names.len() as i64 + 25_000, now);
    }
    let i = ((now - at0) / SPAN).clamp(0, names.len() as i64 - 1);
    let (name, at) = (names[i as usize].as_str(), at0 + SPAN * i);
    let last = i as usize + 1 == names.len();
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let spot = with_vm(world, |vm, world| {
        let z = zone(vm, world, name)?;
        Some((
            vec_field(vm, world, z, "origin"),
            vec_field(vm, world, z, "angles"),
        ))
    })
    .flatten();
    let Some((c, ang)) = spot else {
        if (at..at + tick).contains(&now) {
            diag::warn!(Sim, "bo2zm t6 gen test: no capture zone {name}");
        }
        return;
    };
    if (at..at + tick).contains(&now) {
        let feet = place_near(world, c, ang[1], 72.0);
        let feet = feet.unwrap_or([c[0], c[1], c[2] + 4.0]);
        super::teleport_player(world, client, feet);
        diag::info!(
            Sim,
            "bo2zm t6 gen test: {name} at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    // Use for half a second every 2 s until the capture starts (one press
    // sometimes lands before the generator's trigger is live).
    if (at + 1000..at + 9000).contains(&now) && (now - at) % 2000 < 500 {
        let started = with_vm(world, |vm, world| {
            let z = zone(vm, world, name);
            level_flag(vm, "zone_capture_in_progress")
                || z.is_some_and(|z| ent_flag(vm, world, z, "player_controlled") == "1")
        })
        .unwrap_or(false);
        if !started {
            look_and_use(world, client, [c[0], c[1], c[2] + 30.0]);
        }
    }
    if std::env::var_os("IW4L_T6_GEN_BOX").is_some() {
        box_steps(world, name, at, now);
    }
    if now >= at + 2000 && now < at + if last { 122_000 } else { SPAN } && (now - at) % 5000 < tick
    {
        let line = with_vm(world, |vm, world| {
            let z = zone(vm, world, name)?;
            let (cur, caps) = (vm.intern("n_current_progress"), vm.intern("capture_zombies"));
            let progress = vm.get_field(world, z, cur);
            let progress = vm.to_text(&progress);
            let zombies = match vm.get_field(world, z, caps) {
                Value::Array(a) => a.len().to_string(),
                _ => "-".to_owned(),
            };
            Some(format!(
                "in_progress {} contested {} player_controlled {} progress {progress} capture_zombies {zombies} power {}",
                u8::from(level_flag(vm, "zone_capture_in_progress")),
                ent_flag(vm, world, z, "zone_contested"),
                ent_flag(vm, world, z, "player_controlled"),
                u8::from(level_flag(vm, "power_on")),
            ))
        })
        .flatten()
        .unwrap_or_default();
        let score = frame(world)
            .player_mut(client)
            .map(|p| p.origin)
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 gen test {}s: {name} {line}; player ({:.0} {:.0} {:.0})",
            (now - at) / 1000,
            score[0],
            score[1],
            score[2]
        );
    }
}

/// IW4L_T6_GEN_BOX=1 (with IW4L_T6_GEN): 20 s after the capture started the
/// player stands at that zone's magic box with 10000 points and uses it,
/// takes the weapon 6.5 s later, and his weapons and points are logged.
fn box_steps(world: &mut World, name: &str, at: i64, now: i64) {
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let t = now - at;
    if !(20_000..30_000 + tick).contains(&t) {
        return;
    }
    let spot = with_vm(world, |vm, world| {
        let z = zone(vm, world, name)?;
        let f = vm.intern("mystery_boxes");
        let Value::Array(a) = vm.get_field(world, z, f) else {
            return None;
        };
        let Some(Value::Object(b)) = a.get(&Key::Int(0)) else {
            return None;
        };
        Some((
            vec_field(vm, world, b, "origin"),
            vec_field(vm, world, b, "angles"),
        ))
    })
    .flatten();
    let Some((c, ang)) = spot else {
        if t < tick {
            diag::warn!(Sim, "bo2zm t6 gen box test: {name} has no box");
        }
        return;
    };
    if t < 20_000 + tick {
        with_vm(world, |vm, world| {
            if let Some(p) = world.resource::<super::Zm>().players.get(&0).map(|p| p.obj) {
                let f = vm.intern("score");
                vm.set_field(world, p, f, Value::Int(10_000));
            }
        });
        let feet = place_near(world, c, ang[1] - 90.0, 56.0).unwrap_or([c[0], c[1], c[2] + 4.0]);
        super::teleport_player(world, client, feet);
        diag::info!(
            Sim,
            "bo2zm t6 gen box test: box at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (21_000..21_300).contains(&t) || (27_500..27_800).contains(&t) {
        look_and_use(world, client, [c[0], c[1], c[2] + 20.0]);
    }
    if (30_000..30_000 + tick).contains(&t) {
        let names: Vec<String> = {
            let f = frame(world);
            crate::script_player::weapons(&f, client, crate::script_player::WeaponList::All)
                .into_iter()
                .map(|w| crate::script_player::weapon_name(&f, w))
                .collect()
        };
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        let hint = world
            .resource::<super::Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 gen box test: weapons {names:?}, points {score}, hint {hint:?}"
        );
    }
}

/// IW4L_T6_GEN_PAP=1: see the module notes. `at` is when the player is
/// put at the machine.
fn pap_steps(world: &mut World, at: i64, now: i64) {
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let t = now - at;
    if !(0..20_000 + tick).contains(&t) {
        return;
    }
    let Some((_, c, ang)) =
        super::autoplay::ent_by(world, "script_noteworthy", "specialty_weapupgrade")
    else {
        if t < tick {
            diag::warn!(
                Sim,
                "bo2zm t6 gen pap test: no specialty_weapupgrade trigger"
            );
        }
        return;
    };
    if t < tick {
        with_vm(world, |vm, world| {
            if let Some(p) = world.resource::<super::Zm>().players.get(&0).map(|p| p.obj) {
                let f = vm.intern("score");
                vm.set_field(world, p, f, Value::Int(10_000));
            }
        });
        let feet = place_near(world, c, ang[1], 48.0).unwrap_or([c[0], c[1], c[2] + 4.0]);
        super::teleport_player(world, client, feet);
        diag::info!(
            Sim,
            "bo2zm t6 gen pap test: trigger at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (1_000..1_300).contains(&t) || (11_000..11_300).contains(&t) {
        look_and_use(world, client, [c[0], c[1], c[2] + 30.0]);
    }
    if [500, 2_000, 6_000, 12_000, 20_000]
        .iter()
        .any(|s| (*s..*s + tick).contains(&t))
    {
        let names: Vec<String> = {
            let f = frame(world);
            crate::script_player::weapons(&f, client, crate::script_player::WeaponList::All)
                .into_iter()
                .map(|w| crate::script_player::weapon_name(&f, w))
                .collect()
        };
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        let hint = world
            .resource::<super::Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        let flags = with_vm(world, |vm, _| {
            ["all_zones_captured", "power_on", "pack_machine_in_use"]
                .map(|f| format!("{f} {}", u8::from(level_flag(vm, f))))
                .join(", ")
        })
        .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 gen pap test {}s: {flags}; weapons {names:?}, points {score}, hint {hint:?}",
            t / 1000
        );
    }
}

/// The perks of Origins' fixed machines, in the order the perk test visits
/// them.
const PERKS: [&str; 5] = [
    "specialty_quickrevive",
    "specialty_armorvest",
    "specialty_fastreload",
    "specialty_longersprint",
    "specialty_additionalprimaryweapon",
];

/// IW4L_T6_GEN_PERKS=1: see the module notes. `at` is when the player is
/// put at the first machine.
fn perk_steps(world: &mut World, at: i64, now: i64) {
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let t = now - at;
    if t < 0 || t >= 12_000 * PERKS.len() as i64 {
        return;
    }
    let (perk, t) = (PERKS[(t / 12_000) as usize], t % 12_000);
    let Some((_, c, ang)) = super::autoplay::ent_by(world, "script_noteworthy", perk) else {
        if t < tick {
            diag::warn!(Sim, "bo2zm t6 gen perk test: no {perk} trigger");
        }
        return;
    };
    if t < tick {
        with_vm(world, |vm, world| {
            if let Some(p) = world.resource::<super::Zm>().players.get(&0).map(|p| p.obj) {
                let f = vm.intern("score");
                vm.set_field(world, p, f, Value::Int(10_000));
            }
        });
        let feet = place_near(world, c, ang[1], 48.0).unwrap_or([c[0], c[1], c[2] + 4.0]);
        super::teleport_player(world, client, feet);
        diag::info!(
            Sim,
            "bo2zm t6 gen perk test: {perk} trigger at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (1_000..1_300).contains(&t) {
        look_and_use(world, client, [c[0], c[1], c[2] + 30.0]);
    }
    if (800..800 + tick).contains(&t) || (10_000..10_000 + tick).contains(&t) {
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        let zm = world.resource::<super::Zm>();
        let hint = zm.hints.get(&0).cloned().unwrap_or_default();
        let perks: Vec<String> = zm
            .players
            .get(&0)
            .map(|p| p.perks.iter().cloned().collect())
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 gen perk test {}s: {perk}; perks {perks:?}, points {score}, hint {hint:?}",
            t / 1000
        );
    }
}

/// The entity numbers and origins of the script models showing `model`,
/// nearest to `from` first.
fn models_near(world: &World, model: &str, from: [f32; 3]) -> Vec<(u32, [f32; 3])> {
    let mut v: Vec<(u32, [f32; 3])> = world
        .resource::<super::Zm>()
        .ents
        .iter()
        .filter(|(_, e)| e.model == model)
        .map(|(n, e)| (*n, e.origin))
        .collect();
    let d = |o: &[f32; 3]| (0..3).map(|i| (o[i] - from[i]).powi(2)).sum::<f32>();
    v.sort_by(|a, b| d(&a.1).total_cmp(&d(&b.1)));
    v
}

/// IW4L_T6_DIG: see the module notes.
pub(crate) fn dig_test(world: &mut World, now: i64) {
    static AT: OnceLock<Option<i64>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        let v = std::env::var("IW4L_T6_DIG").ok()?;
        Some(v.parse::<f32>().map_or(30_000, |s| (s * 1000.0) as i64))
    }) else {
        return;
    };
    static SPOT: std::sync::Mutex<Option<[f32; 3]>> = std::sync::Mutex::new(None);
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let t = now - at;
    // Step 0 is the shovel, 1..=5 the mounds.
    if !(0..60_000 + tick).contains(&t) {
        return;
    }
    let (step, t) = (t / 10_000, t % 10_000);
    let me = frame(world)
        .player_mut(client)
        .map(|p| p.origin)
        .unwrap_or_default();
    if t < tick {
        let model = if step == 0 {
            "p6_zm_tm_shovel"
        } else {
            "p6_zm_tm_dig_mound"
        };
        let near = models_near(world, model, me);
        let Some(&(n, c)) = near.first() else {
            diag::warn!(Sim, "bo2zm t6 dig test {step}: no {model} left");
            *SPOT.lock().unwrap() = None;
            return;
        };
        if step == 0 {
            with_vm(world, |vm, world| {
                if let Some(p) = world.resource::<super::Zm>().players.get(&0).map(|p| p.obj) {
                    let f = vm.intern("score");
                    vm.set_field(world, p, f, Value::Int(10_000));
                }
            });
        }
        let feet = place_near(world, c, 0.0, 40.0).unwrap_or([c[0], c[1], c[2] + 4.0]);
        super::teleport_player(world, client, feet);
        *SPOT.lock().unwrap() = Some(c);
        diag::info!(
            Sim,
            "bo2zm t6 dig test {step}: {model} ent{n} at ({:.0} {:.0} {:.0}) of {}; player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            near.len(),
            feet[0],
            feet[1],
            feet[2]
        );
    }
    let spot = *SPOT.lock().unwrap();
    if let Some(c) = spot
        && (1_000..1_400).contains(&t)
    {
        look_and_use(world, client, [c[0], c[1], c[2] + 10.0]);
    }
    // A dug-up gun floats over the mound until it is used.
    if let Some(c) = spot
        && (5_000..5_400).contains(&t)
    {
        look_and_use(world, client, [c[0], c[1], c[2] + 40.0]);
    }
    if (800..800 + tick).contains(&t) || (8_000..8_000 + tick).contains(&t) {
        let names: Vec<String> = {
            let f = frame(world);
            crate::script_player::weapons(&f, client, crate::script_player::WeaponList::All)
                .into_iter()
                .map(|w| crate::script_player::weapon_name(&f, w))
                .collect()
        };
        let score = frame(world).client_meta(client).map_or(0, |m| m.score);
        let hint = world
            .resource::<super::Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        let dig = with_vm(world, |vm, world| {
            let p = world
                .resource::<super::Zm>()
                .players
                .get(&0)
                .map(|p| p.obj)?;
            let f = vm.intern("dig_vars");
            let Value::Array(a) = vm.get_field(world, p, f) else {
                return None;
            };
            Some(
                ["has_shovel", "n_spots_dug", "n_losing_streak"]
                    .map(|k| {
                        let v = a.get(&Key::Str(vm.intern(k))).unwrap_or_default();
                        format!("{k} {}", vm.to_text(&v))
                    })
                    .join(", "),
            )
        })
        .flatten()
        .unwrap_or_default();
        let mounds = models_near(world, "p6_zm_tm_dig_mound", me).len();
        let powerups: Vec<String> = world
            .resource::<super::Zm>()
            .ents
            .values()
            .filter(|e| {
                (e.model.starts_with("zombie_") && !e.model.contains("vending"))
                    || e.model.starts_with("t6_wpn")
                    || e.model.contains("powerup")
            })
            .map(|e| e.model.clone())
            .collect();
        diag::info!(
            Sim,
            "bo2zm t6 dig test {step} {}s: {dig}; weapons {names:?}, points {score}, mounds {mounds}, pickups {powerups:?}, hint {hint:?}",
            t / 1000
        );
    }
}
