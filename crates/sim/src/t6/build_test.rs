//! IW4L_T6_BUILD test aid for BO2's buildables (taken from zm-buried eb066e3).

use std::sync::{Mutex, OnceLock, PoisonError};

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, Value, Vm};

use super::{Zm, frame, with_vm};
use crate::world::ClientId;
/// IW4L_T6_BUILD=<buildable>[@<seconds>] (turbine, springpad_zm,
/// subwoofer_zm, headchopper_zm ...): a test aid for BO2's buildables.
/// From that time (15 s unless given) every 12 s: the player stands on the
/// next part still lying in the world (`level.buildable_stubs`, the stub's
/// `buildablezone.pieces`), presses use (pick up), then stands at the
/// stub's table looking at it and holds use 4 s (build). When every part is
/// built he presses use there once (take it). Every step is logged.
pub(crate) fn build_test(world: &mut World, now: i64) {
    static WANT: OnceLock<Option<(String, i64)>> = OnceLock::new();
    static PART: Mutex<Option<ObjRef>> = Mutex::new(None);
    let Some((name, start)) = WANT
        .get_or_init(|| {
            let v = std::env::var("IW4L_T6_BUILD").ok()?;
            Some(match v.split_once('@') {
                Some((n, s)) => (
                    n.to_owned(),
                    s.parse::<f32>().map_or(15000, |s| (s * 1000.0) as i64),
                ),
                None => (v, 15000),
            })
        })
        .as_ref()
    else {
        return;
    };
    let (name, start) = (name.as_str(), *start);
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
        let next = parts
            .iter()
            .find(|p| !p.built && p.model.is_some())
            .cloned();
        *PART.lock().unwrap_or_else(PoisonError::into_inner) = next.as_ref().map(|p| p.obj);
        let summary: Vec<String> = parts
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
                stand_at_table(world, client, stub_origin, stub_angles);
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
    if (1000..1500).contains(&t) {
        press(world, client);
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
        stand_at_table(world, client, stub_origin, stub_angles);
    }
    if have_part && (4000..4000 + tick).contains(&t) {
        let hint = world
            .resource::<Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        diag::info!(Sim, "bo2zm t6 build test: at the table, hint {hint:?}");
    }
    if have_part && (4200..8500).contains(&t) {
        press(world, client);
    }
    if (9000..9000 + tick).contains(&t) {
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
            "bo2zm t6 build test: {name} {built}/{} built (as read at the round start); weapons {names:?}, hint {hint:?}",
            parts.len()
        );
    }
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

/// The stub of that buildable: its table's place and angles, its parts.
fn find_stub(
    vm: &mut Vm<World>,
    world: &mut World,
    name: &str,
) -> Option<([f32; 3], [f32; 3], Vec<Part>)> {
    let (eq, zone, pieces, mn, built, model, angles) = (
        vm.intern("equipname"),
        vm.intern("buildablezone"),
        vm.intern("pieces"),
        vm.intern("modelname"),
        vm.intern("built"),
        vm.intern("model"),
        vm.intern("angles"),
    );
    let stub = buildable_stubs(vm)
        .into_iter()
        .find(|&s| vm.to_text(&vm.raw_field(s, eq)) == name)?;
    let origin = origin_or_field(vm, world, stub);
    let ang = vm.raw_field(stub, angles).as_vec3().unwrap_or_default();
    let z = vm.raw_field(stub, zone).as_obj()?;
    let Value::Array(a) = vm.raw_field(z, pieces) else {
        return Some((origin, ang, Vec::new()));
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
    Some((origin, ang, parts))
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

/// The player in the table's use box (on it, in front, behind or beside,
/// wherever he first fits), looking at its middle.
fn stand_at_table(world: &mut World, client: ClientId, origin: [f32; 3], angles: [f32; 3]) {
    let (fw, rt, _) = gsc_t6::math::angle_vectors([0.0, angles[1], 0.0]);
    let offsets = [
        [0.0; 3],
        fw.map(|v| -v * 28.0),
        fw.map(|v| v * 28.0),
        rt.map(|v| -v * 40.0),
        rt.map(|v| v * 40.0),
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
