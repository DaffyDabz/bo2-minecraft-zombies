//! Mob of the Dead (zm_prison) for the autoplay test player: every player
//! starts in Afterlife and goes back to it when downed, and only walking to
//! his own body and holding Use brings him back. Without this the test
//! player would stand as a ghost forever.

use bevy_ecs::prelude::World;
use std::sync::atomic::{AtomicI64, Ordering};

use gsc_t6::{ObjKind, Value};
use playerstate_iw4::buttons;

use super::{Zm, with_vm};
use crate::world::ClientId;

/// Where the player's own body lies while he is in Afterlife
/// (`self.afterlife` and `self.e_afterlife_corpse`, _zm_afterlife).
pub(super) fn afterlife_corpse(world: &mut World, client: ClientId) -> Option<[f32; 3]> {
    let corpse = with_vm(world, |vm, world| {
        let p = world.resource::<Zm>().players.get(&client.0)?.obj;
        let f = vm.intern("afterlife");
        if !gsc_t6::truthy(&vm.get_field(world, p, f)) {
            return None;
        }
        let f = vm.intern("e_afterlife_corpse");
        match vm.get_field(world, p, f) {
            Value::Object(o) => match vm.kind(o) {
                Some(ObjKind::Entity(n)) => Some(n),
                _ => None,
            },
            _ => None,
        }
    })??;
    world.resource::<Zm>().ents.get(&corpse).map(|e| e.origin)
}

/// The keys that walk to the body and hold Use on it:
/// (buttons, forward, right, view angles).
pub(super) fn revive_keys(
    origin: [f32; 3],
    eye: [f32; 3],
    corpse: [f32; 3],
) -> (u32, i8, i8, [f32; 3]) {
    let view = super::autoplay::look(eye, corpse);
    let d = ((corpse[0] - origin[0]).powi(2) + (corpse[1] - origin[1]).powi(2)).sqrt();
    if d > 40.0 {
        (0, 127, 0, view)
    } else {
        (buttons::USE, 0, 0, view)
    }
}

/// IW4L_T6_SHOCK=<script_string> (e.g. `revive_on`; `list` only logs them):
/// the shock boxes (`afterlife_interact`; an Afterlife door's box by its
/// targetname). At 1 s every box is logged. Once
/// the player holds the lightning hands (the Afterlife start, 10 to 40 s in)
/// he is put in front of the box that powers that, looking at it, and taps
/// fire for 20 s, his weapon and clip logged each second; then the boxes are
/// logged again (`p6_zm_al_shock_box_on` once zapped). For the next 10 s he
/// is put by his body and holds Use, so he is alive again; then IW4L_T6_PERK
/// runs (its 42 s step comes 32 s after the hands).
pub(crate) fn shock_test(world: &mut World, now: i64) {
    use std::sync::OnceLock;
    static WANT: OnceLock<Option<String>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| std::env::var("IW4L_T6_SHOCK").ok())
        .clone()
    else {
        return;
    };
    let tick = i64::from(crate::MATCH_TICK_MS);
    // When he first held the hands (and was put at the box).
    static START: AtomicI64 = AtomicI64::new(i64::MIN);
    let start = START.load(Ordering::Relaxed);
    let rel = if start == i64::MIN { -1 } else { now - start };
    let at = |t: i64| (t..t + tick).contains(&now);
    let at_rel = |t: i64| (t..t + tick).contains(&rel);
    if (21000 + tick..31000).contains(&rel) {
        revive_at_body(world, rel, 21000);
        return;
    }
    if at_rel(32000) {
        super::autoplay::PERK_BASE.store(now - 42000, Ordering::Relaxed);
        return;
    }
    let placing = start == i64::MIN && (5000..120000).contains(&now);
    if !(at(1000) || placing || (0..20000).contains(&rel) || at_rel(21000)) {
        return;
    }
    // (entnum, script_string, origin, model)
    let boxes: Vec<(u32, String, [f32; 3], String)> = with_vm(world, |vm, world| {
        let (tn, ss) = (vm.intern("targetname"), vm.intern("script_string"));
        world
            .resource::<Zm>()
            .ents
            .iter()
            .filter_map(|(n, e)| {
                let o = e.obj?;
                let name = vm.to_text(&vm.raw_field(o, tn));
                // An Afterlife door's box (zm_prison alcatraz_afterlife_doors)
                // goes by its own targetname.
                let label = if name == "afterlife_interact" {
                    vm.to_text(&vm.raw_field(o, ss))
                } else if e.model.starts_with("p6_zm_al_shock_box") && !name.is_empty() {
                    name
                } else {
                    return None;
                };
                Some((*n, label, e.origin, e.model.clone()))
            })
            .collect()
    })
    .unwrap_or_default();
    if at(1000) || at_rel(21000) {
        for (n, s, o, m) in &boxes {
            diag::info!(
                Sim,
                "bo2zm t6 shock box ent{n} {s:?} {m} at ({:.0} {:.0} {:.0})",
                o[0],
                o[1],
                o[2]
            );
        }
        // The perk machines: a powered one has its "_on" model.
        for (n, e) in &world.resource::<Zm>().ents {
            if e.model.contains("vending") {
                diag::info!(Sim, "bo2zm t6 shock perk machine ent{n} {}", e.model);
            }
        }
        return;
    }
    let Some(e) = boxes
        .iter()
        .find(|b| b.1 == want)
        .and_then(|b| world.resource::<Zm>().ents.get(&b.0).cloned())
    else {
        return;
    };
    let client = ClientId(0);
    let target = super::brushes::model_box(world, &e).map_or(e.origin, |(lo, hi)| {
        std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5)
    });
    // Placed once he holds the lightning hands: the Afterlife start moves
    // him to its own spot first (its time varies with the load).
    let held = super::frame(world).player(client).map_or(0, |p| p.weapon);
    let hands = held != 0 && super::weapon_text(world, held) == "lightning_hands_zm";
    if start == i64::MIN {
        if !hands {
            return;
        }
        START.store(now, Ordering::Relaxed);
        // The open side: the first spot on its four sides (80 to 160 units
        // out), else its corners, where he stands clear and his eye sees
        // the box.
        let (fw, rt, _) = gsc_t6::math::angle_vectors([0.0, e.angles[1], 0.0]);
        let sides = [
            [-rt[0], -rt[1], 0.0],
            fw,
            [rt[0], rt[1], 0.0],
            [-fw[0], -fw[1], 0.0],
        ];
        let f = super::frame(world);
        let spot = |d: &[f32; 3], dist: f32| -> Option<[f32; 3]> {
            let stand: [f32; 3] = std::array::from_fn(|i| target[i] + d[i] * dist);
            let down = [stand[0], stand[1], stand[2] - 200.0];
            let t = f.trace_static_world(
                stand,
                down,
                crate::bullet_collision::PLAYER_MINS,
                crate::bullet_collision::PLAYER_MAXS,
                crate::bullet_collision::MASK_PLAYER_SOLID,
            );
            if t.startsolid != 0 || t.fraction >= 1.0 {
                return None;
            }
            let feet: [f32; 3] =
                std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
            let eye = [feet[0], feet[1], feet[2] + 60.0];
            let sight = f.trace_static_world(
                eye,
                target,
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            );
            (sight.startsolid == 0 && sight.fraction > 0.9).then_some(feet)
        };
        // Else the corners too, 40 to 240 out (the zap reaches 256).
        let corners: Vec<[f32; 3]> = (0..4)
            .map(|k| {
                let (a, b) = (sides[k], sides[(k + 1) % 4]);
                let h = std::f32::consts::FRAC_1_SQRT_2;
                [(a[0] + b[0]) * h, (a[1] + b[1]) * h, 0.0]
            })
            .collect();
        let feet = [80.0, 110.0, 140.0, 160.0]
            .iter()
            .find_map(|&dist| sides.iter().find_map(|d| spot(d, dist)))
            .or_else(|| {
                [40.0, 60.0, 80.0, 110.0, 140.0, 160.0, 200.0, 240.0]
                    .iter()
                    .find_map(|&dist| sides.iter().chain(&corners).find_map(|d| spot(d, dist)))
            });
        let Some(feet) = feet else {
            diag::warn!(Sim, "bo2zm t6 shock test: no open side at the {want} box");
            return;
        };
        drop(f);
        super::teleport_player(world, client, feet);
        diag::info!(
            Sim,
            "bo2zm t6 shock test: {want} box at ({:.0} {:.0} {:.0}), player at ({:.0} {:.0} {:.0})",
            target[0],
            target[1],
            target[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    // Look at it every tick of the zap (his eye is 60 up from his feet).
    let Some(origin) = super::frame(world).player(client).map(|p| p.origin) else {
        return;
    };
    let view = super::autoplay::look([origin[0], origin[1], origin[2] + 60.0], target);
    super::set_player_view(world, client, view);
    if now % 1000 < tick {
        let f = super::frame(world);
        if let Some(ps) = f.player(client) {
            let clip = crate::script_player::ammo_clip(&f, client, ps.weapon);
            let facts = f.combat_facts_for(ps.weapon).map(|c| {
                (
                    c.clip_size,
                    c.start_ammo,
                    c.max_ammo,
                    c.weap_type,
                    c.weap_class,
                    c.fire_type,
                )
            });
            diag::info!(
                Sim,
                "bo2zm t6 shock test: weapon {} state {} clip {clip} (clip size, start, max, type, class, fire) {facts:?}",
                ps.weapon,
                ps.weaponstate_primary
            );
        }
    }
    if now % 500 < 250 {
        let mut req = world.resource_mut::<crate::step::StepRequest>();
        for (id, cmd) in &mut req.input.cmds {
            if *id == client {
                cmd.buttons |= buttons::ATTACK;
            }
        }
    }
}

/// The shock test's way back from Afterlife: put him beside his body once,
/// then look at it and hold Use till he is up.
fn revive_at_body(world: &mut World, rel: i64, t0: i64) {
    let client = ClientId(0);
    let Some(corpse) = afterlife_corpse(world, client) else {
        return;
    };
    let tick = i64::from(crate::MATCH_TICK_MS);
    if (t0 + tick..t0 + 2 * tick).contains(&rel) {
        super::teleport_player(world, client, [corpse[0] + 30.0, corpse[1], corpse[2]]);
        diag::info!(
            Sim,
            "bo2zm t6 shock test: back to the body at ({:.0} {:.0} {:.0})",
            corpse[0],
            corpse[1],
            corpse[2]
        );
    }
    let Some(origin) = super::frame(world).player(client).map(|p| p.origin) else {
        return;
    };
    let eye = [origin[0], origin[1], origin[2] + 60.0];
    let (press, _, _, view) = revive_keys(origin, eye, corpse);
    super::set_player_view(world, client, view);
    let mut req = world.resource_mut::<crate::step::StepRequest>();
    for (id, cmd) in &mut req.input.cmds {
        if *id == client {
            cmd.buttons |= press;
        }
    }
}

/// One door or debris pile for the door sweep: (entnum, targetname,
/// script_flag, script_noteworthy, zombie_cost, where what it moves stands).
type Door = (u32, String, String, String, i32, Option<[f32; 3]>);

/// IW4L_T6_DOORS=1 (or a comma list of script_flags): every door and debris
/// pile bought by hand, one after the other. Once the player holds the
/// lightning hands he is put by his body and holds Use for 10 s (revived);
/// from 13 s after the hands, every 4 s: 10000 points, put in the next
/// door's use trigger looking at what it opens, Use held 0.5-1.5 s; at 3.5 s
/// its flag, `_door_open`, his points and the hint are logged. With
/// IW4L_T6_SHOCK it starts once that test has him up again. Then
/// IW4L_T6_BOX runs (its clock starts there).
pub(crate) fn door_sweep(world: &mut World, now: i64) {
    use std::sync::{Mutex, OnceLock};
    static WANT: OnceLock<Option<Vec<String>>> = OnceLock::new();
    let Some(want) = WANT
        .get_or_init(|| {
            std::env::var("IW4L_T6_DOORS").ok().map(|v| {
                if v == "1" {
                    Vec::new()
                } else {
                    v.split(',').map(str::to_owned).collect()
                }
            })
        })
        .clone()
    else {
        return;
    };
    static START: AtomicI64 = AtomicI64::new(i64::MIN);
    static DOORS: Mutex<Vec<Door>> = Mutex::new(Vec::new());
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let mut start = START.load(Ordering::Relaxed);
    // With the shock test it starts once that is done and he is up again.
    if start == i64::MIN && std::env::var_os("IW4L_T6_SHOCK").is_some() {
        if super::autoplay::PERK_BASE.load(Ordering::Relaxed) == i64::MIN {
            return;
        }
        start = now - 12000;
        START.store(start, Ordering::Relaxed);
    }
    if start == i64::MIN {
        let held = super::frame(world).player(client).map_or(0, |p| p.weapon);
        if held == 0 || super::weapon_text(world, held) != "lightning_hands_zm" {
            return;
        }
        start = now;
        START.store(now, Ordering::Relaxed);
    }
    let rel = now - start;
    if (1000 + tick..11000).contains(&rel) {
        revive_at_body(world, rel, 1000);
        return;
    }
    if rel < 13000 {
        return;
    }
    let mut doors = DOORS.lock().unwrap();
    if (13000..13000 + tick).contains(&rel) {
        *doors = with_vm(world, |vm, world| {
            let (tn, flag, note, cost, target) = (
                vm.intern("targetname"),
                vm.intern("script_flag"),
                vm.intern("script_noteworthy"),
                vm.intern("zombie_cost"),
                vm.intern("target"),
            );
            let zm = world.resource::<Zm>();
            let mut v: Vec<Door> = zm
                .ents
                .iter()
                .filter(|(_, e)| e.classname.starts_with("trigger_use"))
                .filter_map(|(n, e)| {
                    let o = e.obj?;
                    let name = vm.to_text(&vm.raw_field(o, tn));
                    if name != "zombie_door" && name != "zombie_debris" {
                        return None;
                    }
                    let fl = vm.to_text(&vm.raw_field(o, flag));
                    if !want.is_empty() && !want.contains(&fl) {
                        return None;
                    }
                    let t = vm.to_text(&vm.raw_field(o, target));
                    let aim = zm
                        .ents
                        .values()
                        .find(|e| {
                            e.obj
                                .is_some_and(|eo| vm.to_text(&vm.raw_field(eo, tn)) == t)
                        })
                        .map(|e| e.origin);
                    let c = match vm.raw_field(o, cost) {
                        Value::Int(c) => c as i32,
                        _ => -1,
                    };
                    Some((*n, name, fl, vm.to_text(&vm.raw_field(o, note)), c, aim))
                })
                .collect();
            v.sort_by_key(|d| d.0);
            v
        })
        .unwrap_or_default();
        for d in doors.iter() {
            diag::info!(
                Sim,
                "bo2zm t6 doors: ent{} {} flag {:?} {:?} cost {} opens {:?}",
                d.0,
                d.1,
                d.2,
                d.3,
                d.4,
                d.5.map(|a| a.map(f32::round))
            );
        }
    }
    let step = rel - 13000;
    let (i, t) = ((step / 4000) as usize, step % 4000);
    let Some(door) = doors.get(i).cloned() else {
        if t < tick && i == doors.len() {
            diag::info!(Sim, "bo2zm t6 doors: sweep done, {} doors", doors.len());
            super::autoplay::BOX_BASE.store(now - 11500, Ordering::Relaxed);
        }
        if t < tick && i >= doors.len() && (i - doors.len()) % 2 == 0 {
            drop(doors);
            box_zone_report(world);
        }
        return;
    };
    drop(doors);
    let trig = world.resource::<Zm>().ents.get(&door.0).cloned();
    if t < tick {
        let Some(trig) = trig else {
            diag::info!(Sim, "bo2zm t6 doors: ent{} gone before its turn", door.0);
            return;
        };
        super::autoplay::give_points(world, 10000);
        let c = super::triggers::center(world, &trig);
        let stand = [c[0], c[1], c[2] + 40.0];
        let down = [stand[0], stand[1], stand[2] - 300.0];
        let tr = super::frame(world).trace_static_world(
            stand,
            down,
            crate::bullet_collision::PLAYER_MINS,
            crate::bullet_collision::PLAYER_MAXS,
            crate::bullet_collision::MASK_PLAYER_SOLID,
        );
        let feet: [f32; 3] = std::array::from_fn(|k| stand[k] + (down[k] - stand[k]) * tr.fraction);
        super::teleport_player(world, client, feet);
        diag::info!(
            Sim,
            "bo2zm t6 doors: ent{} {:?} trigger at ({:.0} {:.0} {:.0}), player at ({:.0} {:.0} {:.0})",
            door.0,
            door.2,
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
        return;
    }
    if t < 1500 {
        let Some(trig) = trig else { return };
        let Some(origin) = super::frame(world).player(client).map(|p| p.origin) else {
            return;
        };
        let to = door
            .5
            .unwrap_or_else(|| super::triggers::center(world, &trig));
        let view = super::autoplay::look(
            [origin[0], origin[1], origin[2] + 60.0],
            [to[0], to[1], to[2] + 40.0],
        );
        super::set_player_view(world, client, view);
        if t >= 500 {
            let mut req = world.resource_mut::<crate::step::StepRequest>();
            for (id, cmd) in &mut req.input.cmds {
                if *id == client {
                    cmd.buttons |= buttons::USE;
                }
            }
        }
        return;
    }
    if (3500..3500 + tick).contains(&t) {
        let score = super::frame(world)
            .client_meta(client)
            .map_or(0, |m| m.score);
        let (flag_on, open) = with_vm(world, |vm, world| {
            let first = door.2.split(',').next().unwrap_or("").to_owned();
            let f = vm.intern("flag");
            let flag_on = match vm.raw_field(vm.level, f) {
                Value::Array(a) => a
                    .get(&gsc_t6::Key::Str(vm.intern(&first)))
                    .is_some_and(|v| gsc_t6::truthy(&v)),
                _ => false,
            };
            let f = vm.intern("_door_open");
            let open = world
                .resource::<Zm>()
                .ents
                .get(&door.0)
                .and_then(|e| e.obj)
                .map(|o| vm.to_text(&vm.raw_field(o, f)));
            (flag_on, open)
        })
        .unwrap_or((false, None));
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 doors: ent{} {:?} flag {flag_on}, _door_open {:?}, points {score}, hint {hint:?}",
            door.0,
            door.2,
            open.unwrap_or_else(|| "gone".into())
        );
    }
}

/// The magic box's use trigger only comes to a player in an active zone
/// (_zm_unitrigger): the showing box's place, its stub's zone and whether
/// that zone is enabled and active, and every active zone.
pub(super) fn box_zone_report(world: &mut World) {
    let line = with_vm(world, |vm, world| {
        let f = |vm: &mut gsc_t6::Vm<World>, o: gsc_t6::ObjRef, s: &str| {
            let k = vm.intern(s);
            vm.raw_field(o, k)
        };
        let t = |vm: &mut gsc_t6::Vm<World>, o: gsc_t6::ObjRef, s: &str| {
            let v = f(vm, o, s);
            vm.to_text(&v)
        };
        let lvl = vm.level;
        let zones = f(vm, lvl, "zones");
        let zone_flags = |vm: &mut gsc_t6::Vm<World>, name: &str| -> String {
            let Value::Array(a) = &zones else {
                return "no zones".into();
            };
            match a.get(&gsc_t6::Key::Str(vm.intern(name))) {
                Some(Value::Object(z)) => format!(
                    "enabled {} active {}",
                    t(vm, z, "is_enabled"),
                    t(vm, z, "is_active")
                ),
                _ => "unknown".into(),
            }
        };
        let zm = world.resource::<Zm>();
        let mut boxes = Vec::new();
        let showing: Vec<([f32; 3], gsc_t6::ObjRef)> = zm
            .ents
            .values()
            .filter(|e| e.zbarrier.is_some())
            .filter_map(|e| Some((e.origin, e.obj?)))
            .collect();
        for (o, z) in showing {
            let state = t(vm, z, "state");
            if matches!(state.as_str(), "" | "undefined" | "away") {
                continue;
            }
            let zone = match f(vm, z, "owner") {
                Value::Object(owner) => match f(vm, owner, "unitrigger_stub") {
                    Value::Object(stub) => t(vm, stub, "in_zone"),
                    _ => "no stub".into(),
                },
                _ => "no owner".into(),
            };
            let flags = zone_flags(vm, &zone);
            boxes.push(format!(
                "box {state} at ({:.0} {:.0} {:.0}) zone {zone:?} {flags}",
                o[0], o[1], o[2]
            ));
        }
        let active = match f(vm, lvl, "active_zone_names") {
            Value::Array(a) => a
                .snapshot()
                .values_in_order()
                .map(|v| vm.to_text(v))
                .collect::<Vec<_>>()
                .join(" "),
            _ => "none".into(),
        };
        let lives = match zm.players.get(&0).map(|p| p.obj) {
            Some(p) => format!(
                "lives {} afterlife {} afterliferound {} solo {} intermission {} start_over {}",
                t(vm, p, "lives"),
                t(vm, p, "afterlife"),
                t(vm, p, "afterliferound"),
                t(vm, lvl, "is_forever_solo_game"),
                t(vm, lvl, "intermission"),
                match f(vm, lvl, "flag") {
                    Value::Array(a) => a
                        .get(&gsc_t6::Key::Str(vm.intern("afterlife_start_over")))
                        .map_or("undefined".into(), |v| vm.to_text(&v)),
                    _ => "no flags".into(),
                }
            ),
            None => "no player".into(),
        };
        format!("{}; active zones: {active}; {lives}", boxes.join("; "))
    })
    .unwrap_or_default();
    diag::info!(Sim, "bo2zm t6 box zones: {line}");
}

/// IW4L_T6_LIVES=1: logs the player's Afterlife state each time it changes
/// (`self.lives`, `self.afterlife`, health, `sessionstate`, laststand), with
/// the match time, so the step that takes his lives away shows in the log.
pub(crate) fn lives_watch(world: &mut World, now: i64) {
    use std::sync::{Mutex, OnceLock};
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("IW4L_T6_LIVES").is_some()) {
        return;
    }
    static LAST: Mutex<String> = Mutex::new(String::new());
    let line = with_vm(world, |vm, world| {
        let p = world.resource::<Zm>().players.get(&0)?.obj;
        let mut out = Vec::new();
        for name in [
            "lives",
            "afterlife",
            "health",
            "sessionstate",
            "laststand",
            "afterlifedeaths",
        ] {
            let k = vm.intern(name);
            let v = vm.get_field(world, p, k);
            out.push(format!("{name} {}", vm.to_text(&v)));
        }
        Some(out.join(" "))
    })
    .flatten()
    .unwrap_or_default();
    let mut last = LAST.lock().unwrap();
    if *last != line {
        diag::info!(Sim, "bo2zm t6 lives at {:.1}s: {line}", now as f64 / 1000.0);
        *last = line;
    }
}

/// IW4L_T6_BRUTUS=<seconds>: at that match time `level notify("spawn_brutus",
/// 1)`, as Mob's own devgui switch does (`watch_devgui_brutus`); with any
/// value, every 2 s each living Brutus (`is_brutus`) is logged: place,
/// distance to the player, health, lockdown state, script and animation.
pub(crate) fn brutus_test(world: &mut World, now: i64) {
    use std::sync::OnceLock;
    static AT: OnceLock<Option<i64>> = OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        std::env::var("IW4L_T6_BRUTUS")
            .ok()
            .map(|v| v.parse::<i64>().unwrap_or(0) * 1000)
    }) else {
        return;
    };
    let tick = i64::from(crate::MATCH_TICK_MS);
    if at > 0 && (at..at + tick).contains(&now) {
        with_vm(world, |vm, world| {
            let level = vm.level;
            vm.notify_str(world, level, "spawn_brutus", &[Value::Int(1)]);
        });
        diag::info!(
            Sim,
            "bo2zm t6 brutus: spawn_brutus notified at {}s",
            now / 1000
        );
    }
    if now % 2000 >= tick {
        return;
    }
    let me = super::frame(world)
        .player(ClientId(0))
        .map_or([0.0; 3], |p| p.origin);
    let rows: Vec<String> = with_vm(world, |vm, world| {
        let objs: Vec<(u32, gsc_t6::ObjRef)> = {
            let zm = world.resource::<Zm>();
            zm.actors
                .iter()
                .filter(|(_, a)| a.alive)
                .filter_map(|(n, _)| Some((*n, zm.ents.get(n)?.obj?)))
                .collect()
        };
        let mut out = Vec::new();
        for (n, o) in objs {
            let k = vm.intern("is_brutus");
            if !gsc_t6::truthy(&vm.raw_field(o, k)) {
                continue;
            }
            let mut fields = Vec::new();
            for name in [
                "health",
                "has_helmet",
                "ai_state",
                "priority_item",
                "goal_pos",
            ] {
                let k = vm.intern(name);
                let mut v = vm.get_field(world, o, k);
                // An item (box, perk machine, table...): where it is.
                if let Value::Object(item) = v
                    && name == "priority_item"
                {
                    let k = vm.intern("origin");
                    v = vm.get_field(world, item, k);
                }
                fields.push(format!("{name} {}", vm.to_text(&v)));
            }
            let zm = world.resource::<Zm>();
            let (Some(e), Some(a)) = (zm.ents.get(&n), zm.actors.get(&n)) else {
                continue;
            };
            let d = ((e.origin[0] - me[0]).powi(2) + (e.origin[1] - me[1]).powi(2)).sqrt();
            out.push(format!(
                "ent{n} at ({:.0} {:.0} {:.0}) dist {d:.0} {} {} {}",
                e.origin[0],
                e.origin[1],
                e.origin[2],
                a.script,
                a.playing.as_ref().map_or("-", |p| p.anim.as_str()),
                fields.join(" ")
            ));
        }
        out
    })
    .unwrap_or_default();
    for r in rows {
        diag::info!(Sim, "bo2zm t6 brutus at {}s: {r}", now / 1000);
    }
}
