//! IW4L_T6_USE=<targetname>[@<seconds>] (e.g. use_elec_switch, Buried's
//! power switch): a test aid for any use trigger the map names. At that
//! time (6 s unless given) the player stands in the trigger, looking at its
//! middle, and holds use for half a second; two seconds later the level
//! flag IW4L_T6_USE_FLAG (default power_on) and the trigger's hint are
//! logged, and again 35 s later (power events can take 30 s).
//! IW4L_T6_USE_STAY=1: he is not moved there.

use std::sync::{Mutex, OnceLock, PoisonError};

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, Value, Vm};

use super::{Zm, frame, with_vm};
use crate::world::ClientId;

fn want() -> Option<&'static (String, i64)> {
    static WANT: OnceLock<Option<(String, i64)>> = OnceLock::new();
    WANT.get_or_init(|| {
        let v = std::env::var("IW4L_T6_USE").ok()?;
        let (name, at) = match v.split_once('@') {
            Some((n, s)) => (
                n.to_owned(),
                s.parse::<f32>().map_or(6000, |s| (s * 1000.0) as i64),
            ),
            None => (v, 6000),
        };
        Some((name, at))
    })
    .as_ref()
}

/// IW4L_T6_TP="<x> <y> <z> [yaw]@<seconds>": at that time the player is put
/// there (the console `tp` refuses: a zombies player is never "Alive" to it),
/// e.g. Buried's mansion `2547 326 260 180@20` for the ghosts. His origin is
/// logged two seconds later.
fn tp_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Option<([f32; 4], i64)>> = OnceLock::new();
    let Some((pose, at)) = *WANT.get_or_init(|| {
        let v = std::env::var("IW4L_T6_TP").ok()?;
        let (p, s) = v.split_once('@').unwrap_or((v.as_str(), "20"));
        let n: Vec<f32> = p.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        let at = s.parse::<f32>().map_or(20000, |s| (s * 1000.0) as i64);
        (n.len() >= 3).then(|| ([n[0], n[1], n[2], n.get(3).copied().unwrap_or(0.0)], at))
    }) else {
        return;
    };
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    if (at..at + tick).contains(&now) {
        super::teleport_player(world, client, [pose[0], pose[1], pose[2]]);
        super::set_player_view(world, client, [0.0, pose[3], 0.0]);
    }
    if (at + 2000..at + 2000 + tick).contains(&now) {
        let o = frame(world).player(client).map(|p| p.origin).unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 tp test: player at ({:.0} {:.0} {:.0})",
            o[0],
            o[1],
            o[2]
        );
    }
}

pub(crate) fn use_test(world: &mut World, now: i64) {
    tp_test(world, now);
    let Some((name, at)) = want() else {
        return;
    };
    let (name, at) = (name.as_str(), *at);
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    if (at..at + tick).contains(&now) {
        let trig = with_vm(world, |vm, world| {
            let tn = vm.intern("targetname");
            world
                .resource::<Zm>()
                .ents
                .values()
                .find(|e| {
                    e.classname.starts_with("trigger")
                        && e.obj
                            .is_some_and(|o| vm.to_text(&vm.raw_field(o, tn)) == name)
                })
                .cloned()
        })
        .flatten();
        let Some(trig) = trig else {
            diag::warn!(Sim, "bo2zm t6 use test: no trigger named {name}");
            return;
        };
        let c = super::triggers::center(world, &trig);
        // IW4L_T6_USE_STAY=1: he presses use where he stands (in the bus at
        // its door: the trigger rides with it).
        if std::env::var_os("IW4L_T6_USE_STAY").is_some() {
            diag::info!(
                Sim,
                "bo2zm t6 use test: {name} at ({:.0} {:.0} {:.0}); player stays",
                c[0],
                c[1],
                c[2]
            );
            return;
        }
        let stand = [c[0], c[1], c[2] + 20.0];
        let down = [stand[0], stand[1], stand[2] - 300.0];
        let t = frame(world).trace_static_world(
            stand,
            down,
            crate::bullet_collision::PLAYER_MINS,
            crate::bullet_collision::PLAYER_MAXS,
            crate::bullet_collision::MASK_PLAYER_SOLID,
        );
        let feet: [f32; 3] = std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction);
        super::teleport_player(world, client, feet);
        let eye = [feet[0], feet[1], feet[2] + 60.0];
        let d = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
        let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let view = [
            -d[2].atan2(flat.max(1.0)).to_degrees(),
            d[1].atan2(d[0]).to_degrees(),
            0.0,
        ];
        super::set_player_view(world, client, view);
        diag::info!(
            Sim,
            "bo2zm t6 use test: {name} at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (at + 1000..at + 1500).contains(&now) {
        let mut req = world.resource_mut::<crate::step::StepRequest>();
        for (id, cmd) in &mut req.input.cmds {
            if *id == client {
                cmd.buttons |= playerstate_iw4::buttons::USE;
            }
        }
    }
    // again at +35 s: a power event can take 30 s (Tranzit's reactor)
    if (at + 3000..at + 3000 + tick).contains(&now)
        || (at + 35000..at + 35000 + tick).contains(&now)
    {
        let flag = std::env::var("IW4L_T6_USE_FLAG").unwrap_or_else(|_| "power_on".to_owned());
        let on = with_vm(world, |vm, _| {
            let f = vm.intern("flag");
            match vm.raw_field(vm.level, f) {
                gsc_t6::Value::Array(a) => a
                    .get(&gsc_t6::Key::Str(vm.intern(&flag)))
                    .is_some_and(|v| gsc_t6::truthy(&v)),
                _ => false,
            }
        })
        .unwrap_or(false);
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 use test: {name} used, flag {flag} {on}, hint {hint:?}"
        );
    }
}

/// IW4L_T6_BUILD=<buildable>[@<seconds>] (turbine, springpad_zm,
/// subwoofer_zm, headchopper_zm ...): a test aid for BO2's buildables.
/// From that time (15 s unless given) every 12 s: the player stands on the
/// next part still lying in the world (`level.buildable_stubs`, the stub's
/// `buildablezone.pieces`), presses use (pick up), then stands at the
/// stub's table looking at it and holds use 4 s (build). When every part is
/// built he presses use there once (take it). Every step is logged.
/// A comma list builds one after another, e.g. `powerswitch@20,pap@200`
/// (each takes up to 132 s).
pub(crate) fn build_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Vec<(String, i64)>> = OnceLock::new();
    static PART: Mutex<Option<ObjRef>> = Mutex::new(None);
    // A comma list runs one after another, each from its own time
    // (Buried: `keys_zm@15,sloth@80` opens Leroy's cell, then feeds him).
    let want = WANT.get_or_init(|| {
        let Ok(v) = std::env::var("IW4L_T6_BUILD") else {
            return Vec::new();
        };
        v.split(',')
            .map(|v| match v.split_once('@') {
                Some((n, s)) => (
                    n.to_owned(),
                    s.parse::<f32>().map_or(15000, |s| (s * 1000.0) as i64),
                ),
                None => (v.to_owned(), 15000),
            })
            .collect()
    });
    let Some((name, start)) = want.iter().rfind(|(_, s)| *s <= now) else {
        return;
    };
    let (name, start) = (name.as_str(), *start);
    // `<table>:<other>`: the parts come from the other buildable's stubs
    // (Buried: `sloth:booze` takes the bottle lying in the world, from the
    // `booze` buildable, and gives it to Leroy).
    let (name, from) = name.split_once(':').unwrap_or((name, name));
    if now < start {
        return;
    }
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let (k, t) = ((now - start) / 12000, (now - start) % 12000);
    if k > 10 {
        return;
    }
    let Some((stub_origin, stub_angles, parts)) =
        with_vm(world, |vm, world| find_stub(vm, world, name)).flatten()
    else {
        if t < tick {
            diag::warn!(Sim, "bo2zm t6 build test: no buildable stub {name}");
        }
        return;
    };
    if t < tick {
        if k == 0 {
            log_stubs(world);
        }
        // The next part still lying in the world.
        // Every stub of that buildable (Buried makes several `sloth` stubs).
        let source = with_vm(world, |vm, world| find_parts(vm, world, from)).unwrap_or_default();
        let next = source
            .iter()
            .find(|p| !p.built && p.model.is_some())
            .cloned();
        *PART.lock().unwrap_or_else(PoisonError::into_inner) = next.as_ref().map(|p| p.obj);
        let summary: Vec<String> = source
            .iter()
            .map(|p| {
                let state = if p.built {
                    "(built)"
                } else if p.model.is_none() {
                    "(away)"
                } else {
                    ""
                };
                format!("{}{state}", p.modelname)
            })
            .collect();
        diag::info!(
            Sim,
            "bo2zm t6 build test: {name} round {k}: parts {summary:?}"
        );
        match next {
            Some(p) => {
                let at = p.model.unwrap_or_default();
                let feet = ground(world, at);
                super::teleport_player(world, client, feet);
                super::set_player_view(
                    world,
                    client,
                    look_at([feet[0], feet[1], feet[2] + 60.0], at),
                );
                diag::info!(
                    Sim,
                    "bo2zm t6 build test: to part {} at ({:.0} {:.0} {:.0})",
                    p.modelname,
                    at[0],
                    at[1],
                    at[2]
                );
            }
            None => {
                stand_at(world, client, name, stub_origin, stub_angles);
                diag::info!(
                    Sim,
                    "bo2zm t6 build test: no part left; at the table to take it"
                );
            }
        }
    }
    let have_part = PART
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some();
    // Pick up (or take) once the prompt shows: press 0.5 s from then.
    static PICK_FROM: Mutex<Option<i64>> = Mutex::new(None);
    if (1000..2900).contains(&t) {
        let mut from = PICK_FROM.lock().unwrap_or_else(PoisonError::into_inner);
        if from.is_none_or(|f| f > now || f < now - (t - 1000)) {
            let shown = world
                .resource::<Zm>()
                .hints
                .get(&0)
                .is_some_and(|h| !h.is_empty());
            *from = shown.then_some(now);
        }
        if from.is_some_and(|f| now - f < 500) {
            press(world, client);
        }
    }
    if have_part && (900..900 + tick).contains(&t) {
        let part = *PART.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(p) = part {
            log_part_trigger(world, client, p);
        }
    }
    if have_part && (3000..3000 + tick).contains(&t) {
        let held = held_parts(world);
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(Sim, "bo2zm t6 build test: holding {held:?}, hint {hint:?}");
        stand_at(world, client, name, stub_origin, stub_angles);
    }
    if have_part && (4000..4000 + tick).contains(&t) {
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(Sim, "bo2zm t6 build test: at the table, hint {hint:?}");
        log_table_check(world, client, name);
    }
    // Hold use 4.3 s from when the table's prompt shows (the unitrigger
    // can take a few seconds to reach him where many stubs are near, and a
    // use already held when it appears does not count).
    static HOLD_FROM: Mutex<Option<i64>> = Mutex::new(None);
    if have_part && (4200..9000).contains(&t) {
        let mut from = HOLD_FROM.lock().unwrap_or_else(PoisonError::into_inner);
        if from.is_none_or(|f| f > now || f < now - (t - 4200)) {
            let shown = world
                .resource::<Zm>()
                .hints
                .get(&0)
                .is_some_and(|h| !h.is_empty());
            *from = shown.then_some(now);
        }
        if from.is_some_and(|f| now - f < 4300) {
            press(world, client);
        }
    }
    if (11000..11000 + tick).contains(&t) {
        let built = parts.iter().filter(|p| p.built).count();
        let names: Vec<String> = {
            let f = frame(world);
            crate::script_player::weapons(&f, client, crate::script_player::WeaponList::All)
                .into_iter()
                .map(|w| crate::script_player::weapon_name(&f, w))
                .collect()
        };
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(
            Sim,
            "bo2zm t6 build test: {name} {built}/{} built (as read at the round start); weapons {names:?}, hint {hint:?}, power_on {}",
            parts.len(),
            level_flag(world, "power_on")
        );
    }
}

/// `level.flag[name]` is set.
fn level_flag(world: &mut World, name: &str) -> bool {
    with_vm(world, |vm, _| {
        let f = vm.intern("flag");
        match vm.raw_field(vm.level, f) {
            Value::Array(a) => a
                .get(&gsc_t6::Key::Str(vm.intern(name)))
                .is_some_and(|v| gsc_t6::truthy(&v)),
            _ => false,
        }
    })
    .unwrap_or(false)
}

#[derive(Clone)]
struct Part {
    obj: ObjRef,
    modelname: String,
    built: bool,
    /// Where its model lies (none: carried, or not spawned).
    model: Option<[f32; 3]>,
}

fn buildable_stubs(vm: &mut Vm<World>) -> Vec<ObjRef> {
    let f = vm.intern("buildable_stubs");
    match vm.raw_field(vm.level, f) {
        Value::Array(a) => a
            .read()
            .values_in_order()
            .filter_map(Value::as_obj)
            .collect(),
        _ => Vec::new(),
    }
}

fn origin_or_field(vm: &mut Vm<World>, world: &mut World, o: ObjRef) -> [f32; 3] {
    let f = vm.intern("origin");
    super::origin_of(vm, world, &Value::Object(o))
        .or_else(|| vm.raw_field(o, f).as_vec3())
        .unwrap_or_default()
}

/// Every buildable stub: its buildable, place and whether it is built.
fn log_stubs(world: &mut World) {
    with_vm(world, |vm, world| {
        let (eq, built) = (vm.intern("equipname"), vm.intern("built"));
        for s in buildable_stubs(vm) {
            let o = origin_or_field(vm, world, s);
            diag::info!(
                Sim,
                "bo2zm t6 build test: stub {} at ({:.0} {:.0} {:.0}) built {}",
                vm.to_text(&vm.raw_field(s, eq)),
                o[0],
                o[1],
                o[2],
                vm.to_text(&vm.raw_field(s, built))
            );
        }
    });
}

/// The parts player 0 carries (`current_buildable_pieces`), by model.
fn held_parts(world: &mut World) -> Vec<String> {
    with_vm(world, |vm, world| {
        let p = world.resource::<Zm>().players.get(&0).map(|p| p.obj)?;
        let (cur, mn) = (
            vm.intern("current_buildable_pieces"),
            vm.intern("modelname"),
        );
        let Value::Array(a) = vm.raw_field(p, cur) else {
            return Some(Vec::new());
        };
        let vals: Vec<Value> = a.read().values_in_order().cloned().collect();
        Some(
            vals.iter()
                .filter_map(Value::as_obj)
                .map(|o| vm.to_text(&vm.raw_field(o, mn)))
                .collect(),
        )
    })
    .flatten()
    .unwrap_or_default()
}

/// The parts of every stub of that buildable.
fn find_parts(vm: &mut Vm<World>, world: &mut World, name: &str) -> Vec<Part> {
    let eq = vm.intern("equipname");
    let stubs: Vec<ObjRef> = buildable_stubs(vm)
        .into_iter()
        .filter(|&s| vm.to_text(&vm.raw_field(s, eq)) == name)
        .collect();
    stubs
        .into_iter()
        .flat_map(|s| stub_parts(vm, world, s))
        .collect()
}

/// The stub of that buildable: its table's place and angles, its parts.
fn find_stub(
    vm: &mut Vm<World>,
    world: &mut World,
    name: &str,
) -> Option<([f32; 3], [f32; 3], Vec<Part>)> {
    let (eq, zone, angles) = (
        vm.intern("equipname"),
        vm.intern("buildablezone"),
        vm.intern("angles"),
    );
    let stub = buildable_stubs(vm)
        .into_iter()
        .find(|&s| vm.to_text(&vm.raw_field(s, eq)) == name)?;
    let origin = origin_or_field(vm, world, stub);
    let ang = vm.raw_field(stub, angles).as_vec3().unwrap_or_default();
    vm.raw_field(stub, zone).as_obj()?;
    Some((origin, ang, stub_parts(vm, world, stub)))
}

fn stub_parts(vm: &mut Vm<World>, world: &mut World, stub: ObjRef) -> Vec<Part> {
    let (zone, pieces, mn, built, model) = (
        vm.intern("buildablezone"),
        vm.intern("pieces"),
        vm.intern("modelname"),
        vm.intern("built"),
        vm.intern("model"),
    );
    let Some(z) = vm.raw_field(stub, zone).as_obj() else {
        return Vec::new();
    };
    let Value::Array(a) = vm.raw_field(z, pieces) else {
        return Vec::new();
    };
    let objs: Vec<ObjRef> = a
        .read()
        .values_in_order()
        .filter_map(Value::as_obj)
        .collect();
    let parts = objs
        .into_iter()
        .map(|p| {
            let m = vm.raw_field(p, model);
            Part {
                obj: p,
                modelname: vm.to_text(&vm.raw_field(p, mn)),
                built: gsc_t6::truthy(&vm.raw_field(p, built)),
                model: super::origin_of(vm, world, &m),
            }
        })
        .collect();
    parts
}

/// Why holding use at a table may stop at once: the checks of BO2's
/// `player_continue_building` on the table's trigger for this player
/// (touching it, looking at its origin within 0.4, a clear line from it
/// to the eye).
fn log_table_check(world: &mut World, client: ClientId, name: &str) {
    let trig = with_vm(world, |vm, _| {
        let (eq, pt) = (vm.intern("equipname"), vm.intern("playertrigger"));
        let stub = buildable_stubs(vm)
            .into_iter()
            .find(|&s| vm.to_text(&vm.raw_field(s, eq)) == name)?;
        let Value::Array(a) = vm.raw_field(stub, pt) else {
            return None;
        };
        let v = a.read().values_in_order().next().cloned()?;
        super::entnum(vm, &v)
    })
    .flatten();
    let Some(e) = trig.and_then(|n| world.resource::<Zm>().ents.get(&n).cloned()) else {
        log_stub_zone(world, name);
        return;
    };
    let touching = super::triggers::player_box(world, client.0)
        .is_some_and(|(lo, hi)| super::triggers::touches(world, &e, lo, hi));
    let Some((eye, angles)) = frame(world).player(client).map(|ps| {
        let o = ps.origin;
        ([o[0], o[1], o[2] + ps.view_height_current], ps.viewangles)
    }) else {
        return;
    };
    let (fw, _, _) = gsc_t6::math::angle_vectors(angles);
    let d = gsc_t6::math::sub(e.origin, eye);
    let len = gsc_t6::math::dot(d, d).sqrt().max(1e-3);
    let dot = gsc_t6::math::dot(d, fw) / len;
    let t = frame(world).trace_static_world(
        e.origin,
        eye,
        [0.0; 3],
        [0.0; 3],
        crate::bullet_collision::MASK_SHOT,
    );
    diag::info!(
        Sim,
        "bo2zm t6 build test: table check: trigger at ({:.0} {:.0} {:.0}) box {:?} angles {:?}; touching {touching}; eye ({:.0} {:.0} {:.0}) look dot {dot:.2}; trace to eye fraction {:.2} startsolid {}",
        e.origin[0],
        e.origin[1],
        e.origin[2],
        e.box_dims,
        e.angles,
        eye[0],
        eye[1],
        eye[2],
        t.fraction,
        t.startsolid
    );
}

/// No trigger built for the table: the unitrigger manager only offers stubs
/// whose zone (`in_zone`) is in `level.active_zone_names`, plus the dynamic
/// ones. Log the stub's zone, whether it is registered, and the active zones.
fn log_stub_zone(world: &mut World, name: &str) {
    with_vm(world, |vm, _| {
        let (eq, iz, reg, azn) = (
            vm.intern("equipname"),
            vm.intern("in_zone"),
            vm.intern("registered"),
            vm.intern("active_zone_names"),
        );
        let stub = buildable_stubs(vm)
            .into_iter()
            .find(|&s| vm.to_text(&vm.raw_field(s, eq)) == name);
        let (zone, registered) = stub
            .map(|s| {
                (
                    vm.to_text(&vm.raw_field(s, iz)),
                    vm.to_text(&vm.raw_field(s, reg)),
                )
            })
            .unwrap_or_default();
        let active: Vec<String> = match vm.raw_field(vm.level, azn) {
            Value::Array(a) => {
                let vals: Vec<Value> = a.read().values_in_order().cloned().collect();
                vals.iter().map(|v| vm.to_text(v)).collect()
            }
            _ => Vec::new(),
        };
        diag::info!(
            Sim,
            "bo2zm t6 build test: table check: no player trigger; stub in_zone {zone:?} registered {registered:?}; active zones {active:?}"
        );
    });
}

/// The part's unitrigger stub as the manager sees it: zone, place, size,
/// whether this player has a trigger on it, and where the player stands.
fn log_part_trigger(world: &mut World, client: ClientId, part: ObjRef) {
    let player = frame(world).player(client).map(|ps| ps.origin);
    with_vm(world, |vm, world| {
        let f = |vm: &mut Vm<World>, n: &str| vm.intern(n);
        let ut = f(vm, "unitrigger");
        let Some(stub) = vm.raw_field(part, ut).as_obj() else {
            diag::info!(Sim, "bo2zm t6 build test: part has no unitrigger");
            return;
        };
        let fields = [
            "in_zone",
            "registered",
            "radius",
            "script_height",
            "require_look_at",
        ];
        let vals: Vec<String> = fields
            .iter()
            .map(|n| {
                let k = f(vm, n);
                format!("{n} {}", vm.to_text(&vm.raw_field(stub, k)))
            })
            .collect();
        let o = origin_or_field(vm, world, stub);
        let (ot, pt) = (f(vm, "trigger"), f(vm, "playertrigger"));
        let has = !matches!(vm.raw_field(stub, ot), Value::Undefined)
            || matches!(vm.raw_field(stub, pt), Value::Array(_));
        diag::info!(
            Sim,
            "bo2zm t6 build test: part trigger at ({:.0} {:.0} {:.0}) {}; trigger built {has}; player at {player:?}",
            o[0],
            o[1],
            o[2],
            vals.join(", ")
        );
    });
}

fn ground(world: &mut World, at: [f32; 3]) -> [f32; 3] {
    let stand = [at[0], at[1], at[2] + 20.0];
    let down = [stand[0], stand[1], stand[2] - 300.0];
    let t = frame(world).trace_static_world(
        stand,
        down,
        crate::bullet_collision::PLAYER_MINS,
        crate::bullet_collision::PLAYER_MAXS,
        crate::bullet_collision::MASK_PLAYER_SOLID,
    );
    std::array::from_fn(|i| stand[i] + (down[i] - stand[i]) * t.fraction)
}

fn look_at(eye: [f32; 3], to: [f32; 3]) -> [f32; 3] {
    let d = [to[0] - eye[0], to[1] - eye[1], to[2] - eye[2]];
    let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
    [
        -d[2].atan2(flat.max(1.0)).to_degrees(),
        d[1].atan2(d[0]).to_degrees(),
        0.0,
    ]
}

/// The player in the table's use box (on it, in front or behind, wherever
/// he first fits), looking at its middle. The box (`trigger_box_use`) is
/// 100 long along the table's forward but only 32 deep (its right), so he
/// stands within 31 units of it that way.
/// Buried's `sloth` stub is Leroy himself (candy and booze are given to
/// him): both must face each other, so stand 48 in front of him looking at
/// his chest. Any other stub is a table.
fn stand_at(world: &mut World, client: ClientId, name: &str, origin: [f32; 3], angles: [f32; 3]) {
    if name != "sloth" {
        return stand_at_table(world, client, origin, angles);
    }
    let leroy = with_vm(world, |vm, world| {
        let f = vm.intern("_sloth_ai");
        let v = vm.raw_field(vm.level, f);
        let n = super::entnum(vm, &v)?;
        world
            .resource::<Zm>()
            .ents
            .get(&n)
            .map(|e| (e.origin, e.angles))
    })
    .flatten();
    let Some((at, ang)) = leroy else {
        diag::warn!(Sim, "bo2zm t6 build test: no Leroy (level._sloth_ai)");
        return;
    };
    let (fwd, _, _) = gsc_t6::math::angle_vectors([0.0, ang[1], 0.0]);
    let feet = ground(
        world,
        [at[0] + fwd[0] * 48.0, at[1] + fwd[1] * 48.0, at[2] + 16.0],
    );
    super::teleport_player(world, client, feet);
    super::set_player_view(
        world,
        client,
        look_at(
            [feet[0], feet[1], feet[2] + 60.0],
            [at[0], at[1], at[2] + 50.0],
        ),
    );
    diag::info!(
        Sim,
        "bo2zm t6 build test: Leroy at ({:.0} {:.0} {:.0}) yaw {:.0}; player at ({:.0} {:.0} {:.0})",
        at[0],
        at[1],
        at[2],
        ang[1],
        feet[0],
        feet[1],
        feet[2]
    );
}

fn stand_at_table(world: &mut World, client: ClientId, origin: [f32; 3], angles: [f32; 3]) {
    let (_, rt, _) = gsc_t6::math::angle_vectors([0.0, angles[1], 0.0]);
    let offsets = [
        [0.0; 3],
        rt.map(|v| -v * 26.0),
        rt.map(|v| v * 26.0),
        rt.map(|v| -v * 20.0),
        rt.map(|v| v * 20.0),
    ];
    let mut feet = ground(world, origin);
    for o in offsets {
        let p = [origin[0] + o[0], origin[1] + o[1], origin[2] + o[2]];
        let t = frame(world).trace_static_world(
            p,
            p,
            crate::bullet_collision::PLAYER_MINS,
            crate::bullet_collision::PLAYER_MAXS,
            crate::bullet_collision::MASK_PLAYER_SOLID,
        );
        if t.startsolid == 0 && t.allsolid == 0 {
            feet = ground(world, p);
            break;
        }
    }
    super::teleport_player(world, client, feet);
    super::set_player_view(
        world,
        client,
        look_at([feet[0], feet[1], feet[2] + 60.0], origin),
    );
    diag::info!(
        Sim,
        "bo2zm t6 build test: table at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
        origin[0],
        origin[1],
        origin[2],
        feet[0],
        feet[1],
        feet[2]
    );
}

fn press(world: &mut World, client: ClientId) {
    let mut req = world.resource_mut::<crate::step::StepRequest>();
    for (id, cmd) in &mut req.input.cmds {
        if *id == client {
            cmd.buttons |= playerstate_iw4::buttons::USE;
        }
    }
}
