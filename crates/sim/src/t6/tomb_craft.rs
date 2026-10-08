//! Origins (zm_tomb) crafting test aid.
//!
//! IW4L_T6_CRAFT=<craftable>[@<seconds>] (tomb_shield_zm, equip_dieseldrone_zm,
//! gramophone, elemental_staff_fire ...): from that time (30 s unless given)
//! one step every 10 s, for 15 steps. Each step looks at the craftable's
//! parts (`level.a_uts_craftables`, the stub's `craftablespawn.a_piecespawns`):
//! a part still lying in the world is picked up (the player stands beside it
//! and presses use, four times); when none is left he stands at the crafting
//! table and holds use for 6.5 s (one part goes on per hold); once the stub
//! is `crafted` he presses use there to take it. Every step is logged: the parts, the
//! table's state, his hint, weapons and carried part.
//!
//! Origins crafts at any of its three open tables (`open_craftable_trigger`,
//! stub `open_table`); the first hold there turns that table into this
//! craftable's (`craftable_transfer_data`). Staffs and the gramophone have
//! their own tables.

use std::sync::{Mutex, OnceLock};

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, Value, Vm};

use super::tomb_test::{look_and_use, place_near};
use super::{frame, with_vm};
use crate::world::ClientId;

const STEP: i64 = 10_000;
const STEPS: i64 = 15;

fn want() -> Option<&'static (String, i64)> {
    static WANT: OnceLock<Option<(String, i64)>> = OnceLock::new();
    WANT.get_or_init(|| {
        let v = std::env::var("IW4L_T6_CRAFT").ok()?;
        Some(match v.split_once('@') {
            Some((n, s)) => (
                n.to_owned(),
                s.parse::<f32>().map_or(30_000, |s| (s * 1000.0) as i64),
            ),
            None => (v, 30_000),
        })
    })
    .as_ref()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Doing {
    Pick([f32; 3]),
    Craft([f32; 3]),
    Take([f32; 3]),
}

struct Part {
    name: String,
    /// Where its model lies (none: carried, crafted or not spawned).
    at: Option<[f32; 3]>,
    shared: bool,
    crafted: bool,
}

struct Table {
    name: String,
    origin: [f32; 3],
    angles: [f32; 3],
    crafted: bool,
}

/// `level.a_uts_craftables`.
fn stubs(vm: &mut Vm<World>) -> Vec<ObjRef> {
    let f = vm.intern("a_uts_craftables");
    match vm.raw_field(vm.level, f) {
        Value::Array(a) => a
            .read()
            .values_in_order()
            .filter_map(Value::as_obj)
            .collect(),
        _ => Vec::new(),
    }
}

fn text(vm: &mut Vm<World>, o: ObjRef, f: &str) -> String {
    let f = vm.intern(f);
    let v = vm.raw_field(o, f);
    vm.to_text(&v)
}

fn flag(vm: &mut Vm<World>, o: ObjRef, f: &str) -> bool {
    let f = vm.intern(f);
    gsc_t6::truthy(&vm.raw_field(o, f))
}

/// The stub's craftable (`craftablespawn.craftable_name`).
fn stub_name(vm: &mut Vm<World>, s: ObjRef) -> String {
    let f = vm.intern("craftablespawn");
    match vm.raw_field(s, f).as_obj() {
        Some(cs) => text(vm, cs, "craftable_name"),
        None => String::new(),
    }
}

/// The craftable's parts (from its first stub) and every table it may be
/// crafted at: its own stubs and the open tables, inside the map (the
/// shield's and drone's own triggers lie at x -7040, out of the world).
fn survey(vm: &mut Vm<World>, world: &mut World, name: &str) -> (Vec<Part>, Vec<Table>) {
    let (mut parts, mut tables) = (Vec::new(), Vec::new());
    let (cs_f, ps_f, model_f, origin_f, angles_f) = (
        vm.intern("craftablespawn"),
        vm.intern("a_piecespawns"),
        vm.intern("model"),
        vm.intern("origin"),
        vm.intern("angles"),
    );
    for s in stubs(vm) {
        let sn = stub_name(vm, s);
        if sn != name && sn != "open_table" {
            continue;
        }
        let origin = vm.raw_field(s, origin_f).as_vec3().unwrap_or_default();
        if sn == name && parts.is_empty() {
            let cs = vm.raw_field(s, cs_f).as_obj();
            let list: Vec<ObjRef> = match cs.map(|cs| vm.raw_field(cs, ps_f)) {
                Some(Value::Array(a)) => a
                    .read()
                    .values_in_order()
                    .filter_map(Value::as_obj)
                    .collect(),
                _ => Vec::new(),
            };
            for p in list {
                let m = vm.raw_field(p, model_f);
                parts.push(Part {
                    name: text(vm, p, "modelname"),
                    at: super::origin_of(vm, world, &m),
                    shared: flag(vm, p, "in_shared_inventory"),
                    crafted: flag(vm, p, "crafted"),
                });
            }
        }
        if origin[0] > -7000.0 {
            tables.push(Table {
                name: sn,
                origin,
                angles: vm.raw_field(s, angles_f).as_vec3().unwrap_or_default(),
                crafted: flag(vm, s, "crafted"),
            });
        }
    }
    (parts, tables)
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

/// Where the player stands at a table: inside its use box (32 deep along
/// the table's right, 100 wide) but off its middle, so the trigger's origin
/// is in front of his eyes (`is_player_looking_at`), first spot he fits in.
fn table_spot(world: &mut World, origin: [f32; 3], angles: [f32; 3]) -> [f32; 3] {
    let (fw, rt, _) = gsc_t6::math::angle_vectors([0.0, angles[1], 0.0]);
    for (f, r) in [
        (30.0f32, -14.0f32),
        (30.0, 14.0),
        (-30.0, -14.0),
        (-30.0, 14.0),
        (45.0, 0.0),
        (-45.0, 0.0),
    ] {
        let p = std::array::from_fn(|i| origin[i] + fw[i] * f + rt[i] * r);
        let feet = ground(world, p);
        let up = [feet[0], feet[1], feet[2] + 2.0];
        let t = frame(world).trace_static_world(
            up,
            up,
            crate::bullet_collision::PLAYER_MINS,
            crate::bullet_collision::PLAYER_MAXS,
            crate::bullet_collision::MASK_PLAYER_SOLID,
        );
        if t.startsolid == 0 && t.allsolid == 0 && feet[2] > origin[2] - 120.0 {
            return feet;
        }
    }
    place_near(world, origin, angles[1], 30.0).unwrap_or_else(|| ground(world, origin))
}

static DOING: Mutex<Option<Doing>> = Mutex::new(None);
/// The table chosen at the first craft step.
static TABLE: Mutex<Option<([f32; 3], [f32; 3])>> = Mutex::new(None);

fn d2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

pub(crate) fn craft_test(world: &mut World, now: i64) {
    let Some((name, at)) = want() else {
        return;
    };
    let client = ClientId(0);
    let tick = i64::from(crate::MATCH_TICK_MS);
    let t = now - at;
    if !(0..STEPS * STEP + tick).contains(&t) {
        return;
    }
    let (step, t) = (t / STEP, t % STEP);
    if t < tick {
        let me = frame(world)
            .player_mut(client)
            .map(|p| p.origin)
            .unwrap_or_default();
        let (parts, tables) =
            with_vm(world, |vm, world| survey(vm, world, name)).unwrap_or_default();
        let list: Vec<String> = parts
            .iter()
            .map(|p| {
                let state = match p.at {
                    _ if p.crafted => "crafted".to_owned(),
                    Some(o) => format!("lying ({:.0} {:.0} {:.0})", o[0], o[1], o[2]),
                    None if p.shared => "carried".to_owned(),
                    None => "away".to_owned(),
                };
                format!("{} {state}", p.name)
            })
            .collect();
        let mine: Vec<String> = tables
            .iter()
            .map(|t| {
                format!(
                    "{} ({:.0} {:.0} {:.0}) crafted {}",
                    t.name,
                    t.origin[0],
                    t.origin[1],
                    t.origin[2],
                    u8::from(t.crafted)
                )
            })
            .collect();
        diag::info!(
            Sim,
            "bo2zm t6 craft test {step}: {name} parts {list:?}; tables {mine:?}"
        );
        if step == 0 {
            with_vm(world, |vm, world| {
                if let Some(p) = world.resource::<super::Zm>().players.get(&0).map(|p| p.obj) {
                    let f = vm.intern("score");
                    vm.set_field(world, p, f, Value::Int(10_000));
                }
            });
        }
        let lying = parts
            .iter()
            .filter(|p| !p.crafted)
            .filter_map(|p| p.at)
            .min_by(|a, b| d2(*a, me).total_cmp(&d2(*b, me)));
        let done = tables.iter().any(|t| t.name == *name && t.crafted);
        let doing = if let Some(c) = lying {
            let feet = place_near(world, c, 0.0, 24.0).unwrap_or_else(|| ground(world, c));
            super::teleport_player(world, client, feet);
            diag::info!(
                Sim,
                "bo2zm t6 craft test {step}: part at ({:.0} {:.0} {:.0}); player at ({:.0} {:.0} {:.0})",
                c[0],
                c[1],
                c[2],
                feet[0],
                feet[1],
                feet[2]
            );
            Doing::Pick(c)
        } else {
            let mut table = *TABLE.lock().unwrap();
            if table.is_none() {
                // Its own table if it has one in the map, else the open
                // table nearest the player.
                table = tables
                    .iter()
                    .filter(|t| t.name == *name)
                    .chain(
                        tables
                            .iter()
                            .filter(|t| t.name == "open_table")
                            .min_by(|a, b| d2(a.origin, me).total_cmp(&d2(b.origin, me))),
                    )
                    .map(|t| (t.origin, t.angles))
                    .next();
                *TABLE.lock().unwrap() = table;
            }
            let Some((o, a)) = table else {
                diag::warn!(Sim, "bo2zm t6 craft test {step}: no table for {name}");
                return;
            };
            let feet = table_spot(world, o, a);
            super::teleport_player(world, client, feet);
            diag::info!(
                Sim,
                "bo2zm t6 craft test {step}: table ({:.0} {:.0} {:.0}) yaw {:.0}; player at ({:.0} {:.0} {:.0})",
                o[0],
                o[1],
                o[2],
                a[1],
                feet[0],
                feet[1],
                feet[2]
            );
            if done {
                Doing::Take(o)
            } else {
                Doing::Craft(o)
            }
        };
        *DOING.lock().unwrap() = Some(doing);
    }
    let doing = *DOING.lock().unwrap();
    // Use triggers fire on a fresh press, and the unitrigger manager hands
    // out the part's or table's trigger a moment after he arrives: press
    // (0.3 s) at 1, 2.5, 4 and 5.5 s; a craft holds from 2 s to 8.5 s.
    let pulse = (0..4).any(|k| (1_000 + 1_500 * k..1_300 + 1_500 * k).contains(&t));
    match doing {
        Some(Doing::Pick(c)) if pulse => look_and_use(world, client, [c[0], c[1], c[2] + 8.0]),
        Some(Doing::Craft(o)) if (2_000..8_500).contains(&t) => look_and_use(world, client, o),
        Some(Doing::Take(o)) if pulse => look_and_use(world, client, o),
        _ => {}
    }
    if (2_200..2_200 + tick).contains(&t) || (9_000..9_000 + tick).contains(&t) {
        let names: Vec<String> = {
            let f = frame(world);
            crate::script_player::weapons(&f, client, crate::script_player::WeaponList::All)
                .into_iter()
                .map(|w| crate::script_player::weapon_name(&f, w))
                .collect()
        };
        let hint = world
            .resource::<super::Zm>()
            .hints
            .get(&0)
            .cloned()
            .unwrap_or_default();
        let held = with_vm(world, |vm, world| {
            let p = world
                .resource::<super::Zm>()
                .players
                .get(&0)
                .map(|p| p.obj)?;
            let f = vm.intern("current_craftable_piece");
            let piece = vm.raw_field(p, f).as_obj()?;
            Some(text(vm, piece, "modelname"))
        })
        .flatten();
        diag::info!(
            Sim,
            "bo2zm t6 craft test {step} {}s: {doing:?}; hint {hint:?}, weapons {names:?}, carrying {held:?}",
            t / 1000
        );
    }
}
