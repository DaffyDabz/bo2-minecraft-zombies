//! The Zombies Declassified maps: the Black Ops 1 maps rebuilt in Black Ops
//! II's zone format (cancelled DLC5, leaked). Their authors' fixes ship as
//! GSC source (`storage/t6/raw/scripts/zm/zzz_*.gsc`) that Plutonium
//! compiles at load; we have no source compiler, so the fixes the maps
//! need to play are done here, each named after its source file.

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{FuncRef, Key, ObjRef, Value, Vm};

use super::{Zm, client, frame, localize, weapon, with_vm};
use crate::script_player;

/// The leaked BO1 maps (and Call of the Dead, which shares their scripts).
const MAPS: &[&str] = &[
    "zm_prototype",
    "zm_asylum",
    "zm_sumpf",
    "zm_factory",
    "zm_coast",
    "zm_theater",
    "zm_pentagon",
    "zm_cosmodrome",
    "zm_temple",
    "zm_moon",
];

pub(crate) fn is_declassified(map: &str) -> bool {
    MAPS.contains(&map)
}

/// Fixes still to do on this map.
#[derive(Resource, Default)]
pub(super) struct Fixes {
    round_driver: bool,
    cabinet: bool,
}

/// The level started: arm this map's fixes.
pub(super) fn start(world: &mut World, map: &str) {
    if !is_declassified(map) {
        return;
    }
    world.insert_resource(Fixes {
        round_driver: map != "zm_theater",
        cabinet: map == "zm_prototype",
    });
    if map == "zm_prototype" || map == "zm_factory" {
        proto_names(world);
    }
    advance(world, 0);
}

/// Each frame until every fix is done.
pub(super) fn advance(world: &mut World, now: i64) {
    zones_log(world, now);
    ents_log(world, now);
    dog_round_test(world, now);
    cabinet_test(world, now);
    use_test(world, now);
    let Some(f) = world.get_resource::<Fixes>() else {
        return;
    };
    let cabinet = f.cabinet;
    if f.round_driver && round_driver(world) {
        world.resource_mut::<Fixes>().round_driver = false;
    }
    if cabinet && cabinet_setup(world, now) {
        world.resource_mut::<Fixes>().cabinet = false;
    }
    let f = world.resource::<Fixes>();
    if !f.round_driver && !f.cabinet {
        world.remove_resource::<Fixes>();
    }
}

/// zzz_protonames.gsc: the SMR and Executioner wall buys' retail hints
/// carry pre-release names ("TOZ Saritch"...); the mod's own strings
/// (`ZM_PROTO_WB_SMR`, `ZM_PROTO_WB_EXEC`) have the retail ones. The mod
/// swaps the key `get_weapon_hint` returns; the player reads the same if
/// the retail keys read the mod's text.
fn proto_names(world: &mut World) {
    let mut zm = world.resource_mut::<Zm>();
    for (retail, fixed) in [
        ("ZOMBIE_WEAPON_SARITCH", "ZM_PROTO_WB_SMR"),
        ("ZOMBIE_WEAPON_JUDGE", "ZM_PROTO_WB_EXEC"),
    ] {
        let Some(t) = zm.strings.get(fixed).cloned() else {
            diag::warn!(
                Sim,
                "bo2zm t6 declassified: no {fixed} string (zzz_protonames)"
            );
            continue;
        };
        diag::info!(
            Sim,
            "bo2zm t6 declassified: {retail} reads {t:?} (zzz_protonames)"
        );
        zm.strings.insert(retail.to_owned(), t);
    }
}

/// zzz_cabinet.gsc: Nacht's sniper cabinet. The map has the cabinet, its
/// two doors and a use trigger (`weapon_cabinet_use`) but no script
/// anywhere handles it (BO1's zombie_cod5_prototype.gsc did). Pay 1500:
/// the doors swing open and you get the DSR 50; then it sells the gun
/// again or its ammo like a wall buy.
#[derive(Resource, Default)]
struct Cabinets(Vec<Cabinet>);

#[derive(Clone)]
struct Cabinet {
    trig: ObjRef,
    n: u32,
    /// (entnum, swings the other way: script_noteworthy "right").
    doors: Vec<(u32, bool)>,
    weapon: String,
    cost: i32,
    open: bool,
}

/// Wire the cabinet triggers once the weapons are registered. True when
/// done (wired, or none after 30 s).
fn cabinet_setup(world: &mut World, now: i64) -> bool {
    with_vm(world, |vm, world| {
        let level = Value::Object(vm.level);
        if !matches!(
            path(vm, world, &level, &["zombie_weapons"]),
            Value::Array(_)
        ) {
            return false;
        }
        let (tn, tg, nw) = (
            vm.intern("targetname"),
            vm.intern("target"),
            vm.intern("script_noteworthy"),
        );
        let (wu, zc) = (
            vm.intern("zombie_weapon_upgrade"),
            vm.intern("zombie_cost"),
        );
        let ents: Vec<(u32, ObjRef)> = world
            .resource::<Zm>()
            .ents
            .iter()
            .filter_map(|(n, e)| e.obj.map(|o| (*n, o)))
            .collect();
        let named = |vm: &Vm<World>, name: &str| -> Vec<(u32, ObjRef)> {
            ents.iter()
                .filter(|(_, o)| vm.alive(*o) && vm.to_text(&vm.raw_field(*o, tn)) == name)
                .copied()
                .collect()
        };
        // zzz_zm_doorcreak.gsc: the first-room door's body has no
        // script_sound, so _zm_blockers falls back to
        // level.zombie_sounds["door_slide_open"] = zmb_door_slide_open,
        // which only Nuketown's bank holds: the door opens silently. Give
        // it the bank's wooden door (the same recording; `_close` too).
        let ss = vm.intern("script_sound");
        for (_, d) in named(vm, "zombie_door") {
            let target = vm.to_text(&vm.raw_field(d, tg));
            if target.is_empty() {
                continue;
            }
            for (_, body) in named(vm, &target) {
                let v = vm.string("zmb_small_wood_door");
                vm.set_field(world, body, ss, v);
            }
        }
        let trigs = named(vm, "weapon_cabinet_use");
        if trigs.is_empty() {
            if now > 30_000 {
                diag::warn!(
                    Sim,
                    "bo2zm t6 declassified: no weapon_cabinet_use trigger"
                );
                return true;
            }
            return false;
        }
        let key = Value::IStr(vm.intern("ZOMBIE_CABINET_OPEN_1500"));
        let hint = localize(vm, world, &key, &[]);
        let mut list = Vec::new();
        for (n, o) in trigs {
            let target = vm.to_text(&vm.raw_field(o, tg));
            let doors: Vec<(u32, bool)> = if target.is_empty() {
                Vec::new()
            } else {
                named(vm, &target)
                    .into_iter()
                    .map(|(d, dobj)| (d, vm.to_text(&vm.raw_field(dobj, nw)) == "right"))
                    .collect()
            };
            let weapon = match vm.raw_field(o, wu) {
                Value::Undefined => "dsr50_zm".to_owned(),
                v => vm.to_text(&v),
            };
            let cost = vm.raw_field(o, zc).as_int().unwrap_or(1500);
            let mut zm = world.resource_mut::<Zm>();
            // The doors never block shots or the use trace (BO1
            // notsolid()s them).
            for (d, _) in &doors {
                if let Some(e) = zm.ents.get_mut(d) {
                    e.solid = false;
                }
            }
            if let Some(e) = zm.ents.get_mut(&n) {
                e.hint = Some(hint.clone());
                e.cursor_hint = Some("HINT_NOICON".to_owned());
                diag::info!(
                    Sim,
                    "bo2zm t6 declassified: cabinet ent{n} {} at ({:.0} {:.0} {:.0}) brush {:?} box {:?}, doors {doors:?}, {weapon} {cost}, hint {hint:?} (zzz_cabinet)",
                    e.classname,
                    e.origin[0],
                    e.origin[1],
                    e.origin[2],
                    e.brush,
                    e.box_dims
                );
            }
            list.push(Cabinet {
                trig: o,
                n,
                doors,
                weapon,
                cost,
                open: false,
            });
        }
        world.insert_resource(Cabinets(list));
        true
    })
    .unwrap_or(true)
}

/// A use trigger fired: true when it was a cabinet (handled here, as the
/// missing script would have).
pub(super) fn cabinet_used(vm: &mut Vm<World>, world: &mut World, t: ObjRef, p: ObjRef) -> bool {
    let Some(i) = world
        .get_resource::<Cabinets>()
        .and_then(|c| c.0.iter().position(|c| c.trig == t))
    else {
        return false;
    };
    let c = world.resource::<Cabinets>().0[i].clone();
    let pv = Value::Object(p);
    let Ok(id) = client(vm, world, &pv) else {
        return true;
    };
    let score_f = vm.intern("score");
    let score = vm.get_field(world, p, score_f).as_int().unwrap_or(0);
    // Half the price, as a wall buy's ammo. (The mod asked
    // `get_ammo_cost`, which for the DSR 50, a box gun priced 50 in the
    // scripts, says 30.)
    let ammo_cost = (c.cost / 2 + 9) / 10 * 10;
    let origin = {
        let e = world.resource::<Zm>().ents.get(&c.n).cloned();
        e.map_or([0.0; 3], |e| super::triggers::center(world, &e))
    };
    let has = [c.weapon.clone(), c.weapon.replace("_zm", "_upgraded_zm")]
        .iter()
        .any(|w| weapon(world, w).is_ok_and(|w| script_player::has_weapon(&frame(world), id, w)));
    let charge = |vm: &mut Vm<World>, world: &mut World, points: i32| {
        vm.spawn_named(
            world,
            "maps/mp/zombies/_zm_score",
            "minus_to_player_score",
            pv.clone(),
            vec![Value::Int(points)],
        );
        super::natives_fx::sound(world, crate::EventAudience::All, "zmb_cha_ching", origin);
    };
    let give = |vm: &mut Vm<World>, world: &mut World, f: &str| {
        let w = vm.string(&c.weapon);
        vm.spawn_named(world, "maps/mp/zombies/_zm_weapons", f, pv.clone(), vec![w]);
    };
    if !c.open {
        if score < c.cost {
            return true;
        }
        charge(vm, world, c.cost);
        // The small wooden doors creak as they swing (the mod's choice:
        // BO1 played only the purchase sound).
        super::natives_fx::sound(
            world,
            crate::EventAudience::All,
            "zmb_small_wood_door",
            origin,
        );
        let key = Value::IStr(vm.intern("ZOMBIE_WEAPONCOSTAMMO"));
        let hint = localize(
            vm,
            world,
            &key,
            &[Value::Int(c.cost), Value::Int(ammo_cost)],
        );
        {
            let mut zm = world.resource_mut::<Zm>();
            let now = zm.now_ms;
            for (d, right) in &c.doors {
                let Some(from) = zm.ents.get(d).map(|e| e.angles) else {
                    continue;
                };
                let mut to = from;
                to[1] += if *right { -120.0 } else { 120.0 };
                zm.movers.rotate_to(*d, from, to, now, 0.3, 0.2, 0.1);
            }
            if let Some(e) = zm.ents.get_mut(&c.n) {
                e.hint = Some(hint);
            }
        }
        world.resource_mut::<Cabinets>().0[i].open = true;
        give(vm, world, "weapon_give");
        diag::info!(
            Sim,
            "bo2zm t6 declassified: cabinet opened, {} for {}",
            c.weapon,
            c.cost
        );
    } else if !has {
        if score >= c.cost {
            charge(vm, world, c.cost);
            give(vm, world, "weapon_give");
            diag::info!(
                Sim,
                "bo2zm t6 declassified: cabinet sold {} for {}",
                c.weapon,
                c.cost
            );
        }
    } else if score >= ammo_cost {
        let full = weapon(world, &c.weapon)
            .is_ok_and(|w| script_player::ammo_fraction(&frame(world), id, w, false) >= 1.0);
        if !full {
            charge(vm, world, ammo_cost);
            give(vm, world, "ammo_give");
            diag::info!(Sim, "bo2zm t6 declassified: cabinet ammo for {ammo_cost}");
        }
    }
    true
}

/// IW4L_T6_DZCABINET=1: at 5 s 10000 points; at 6 s the player stands at
/// the cabinet trigger looking at it and uses it (open) at 7 s, again at
/// 9 s (ammo: full, so nothing); weapons, points and hint logged at 8.5
/// and 10.5 s.
fn cabinet_test(world: &mut World, now: i64) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("IW4L_T6_DZCABINET").is_some()) {
        return;
    }
    let tick = i64::from(crate::MATCH_TICK_MS);
    let at = |t: i64| (t..t + tick).contains(&now);
    let id = crate::world::ClientId(0);
    let player = world.resource::<Zm>().players.get(&0).map(|p| p.obj);
    let Some(p) = player else { return };
    if at(5000) {
        with_vm(world, |vm, world| {
            let f = vm.intern("score");
            vm.set_field(world, p, f, Value::Int(10000));
        });
    }
    let cab = world
        .get_resource::<Cabinets>()
        .and_then(|c| c.0.first().cloned());
    let Some(cab) = cab else {
        if at(6000) {
            diag::warn!(Sim, "bo2zm t6 dz cabinet test: no cabinet wired");
        }
        return;
    };
    if at(6000) {
        let e = world.resource::<Zm>().ents.get(&cab.n).cloned();
        let Some(e) = e else { return };
        let c = super::triggers::center(world, &e);
        // Facing the doors (their midpoint), a step back from them.
        let doors: Vec<[f32; 3]> = {
            let zm = world.resource::<Zm>();
            cab.doors
                .iter()
                .filter_map(|(d, _)| zm.ents.get(d).map(|e| e.origin))
                .collect()
        };
        let look_at = if doors.is_empty() {
            c
        } else {
            let k = doors.len() as f32;
            std::array::from_fn(|i| doors.iter().map(|o| o[i]).sum::<f32>() / k)
        };
        let d = [look_at[0] - c[0], look_at[1] - c[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1.0);
        let feet = [
            c[0] - d[0] / len * 30.0,
            c[1] - d[1] / len * 30.0,
            c[2] - 30.0,
        ];
        super::teleport_player(world, id, feet);
        let yaw = d[1].atan2(d[0]).to_degrees();
        super::set_player_view(world, id, [5.0, yaw, 0.0]);
        diag::info!(
            Sim,
            "bo2zm t6 dz cabinet test: player at ({:.0} {:.0} {:.0}), trigger centre ({:.0} {:.0} {:.0})",
            feet[0],
            feet[1],
            feet[2],
            c[0],
            c[1],
            c[2]
        );
    }
    if (7000..7300).contains(&now) || (9000..9300).contains(&now) {
        let mut req = world.resource_mut::<crate::step::StepRequest>();
        for (cid, cmd) in &mut req.input.cmds {
            if *cid == id {
                cmd.buttons |= playerstate_iw4::buttons::USE;
            }
        }
    }
    if at(8500) || at(10500) {
        let names: Vec<String> = {
            let f = frame(world);
            script_player::weapons(&f, id, script_player::WeaponList::All)
                .into_iter()
                .map(|w| script_player::weapon_name(&f, w))
                .collect()
        };
        let score = with_vm(world, |vm, world| {
            let f = vm.intern("score");
            vm.get_field(world, p, f).as_int().unwrap_or(-1)
        })
        .unwrap_or(-1);
        let zm = world.resource::<Zm>();
        let hint = zm.hints.get(&0).cloned().unwrap_or_default();
        let open = world.resource::<Cabinets>().0[0].open;
        diag::info!(
            Sim,
            "bo2zm t6 dz cabinet test: open {open}, points {score}, weapons {names:?}, hint {hint:?}"
        );
    }
}

/// IW4L_T6_DZUSE=<targetname>[#k][,...] (e.g. use_master_switch,gas_access#1):
/// for each name in turn (5 s apart, from IW4L_T6_DZUSE_AT) he gets 10000
/// points and stands in the use trigger of that name (the k-th, from 0),
/// looking at it;
/// 1 s later he holds use for half a second, and 2.5 s after that his
/// place, points, perks, hint and the power flag are logged.
fn use_test(world: &mut World, now: i64) {
    static WANT: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let want = WANT.get_or_init(|| {
        std::env::var("IW4L_T6_DZUSE")
            .map(|s| s.split(',').map(str::to_owned).collect())
            .unwrap_or_default()
    });
    if want.is_empty() {
        return;
    }
    let tick = i64::from(crate::MATCH_TICK_MS);
    let id = crate::world::ClientId(0);
    let Some(p) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) else {
        return;
    };
    // IW4L_T6_DZUSE_AT=<ms> (15000): when the first use starts (a big map's
    // player spawns late; a teleport before that is undone).
    static AT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    static PLACED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let start = *AT.get_or_init(|| {
        std::env::var("IW4L_T6_DZUSE_AT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15000)
    });
    let Some(rel) = now.checked_sub(start) else {
        return;
    };
    let (i, t) = ((rel / 5000) as usize, rel % 5000);
    let Some(name) = want.get(i) else { return };
    if PLACED.load(std::sync::atomic::Ordering::Relaxed) == i {
        PLACED.store(i + 1, std::sync::atomic::Ordering::Relaxed);
        with_vm(world, |vm, world| {
            let f = vm.intern("score");
            vm.set_field(world, p, f, Value::Int(10000));
        });
        let (tname, nth) = match name.split_once('#') {
            Some((n, k)) => (n, k.parse().unwrap_or(0)),
            None => (name.as_str(), 0),
        };
        let trig = with_vm(world, |vm, world| {
            let tn = vm.intern("targetname");
            world
                .resource::<Zm>()
                .ents
                .values()
                .filter(|e| {
                    e.classname.starts_with("trigger_use")
                        && e.obj
                            .is_some_and(|o| vm.to_text(&vm.raw_field(o, tn)) == tname)
                })
                .nth(nth)
                .cloned()
        })
        .flatten();
        let Some(trig) = trig else {
            diag::warn!(Sim, "bo2zm t6 dz use test: no use trigger {name}");
            return;
        };
        let c = super::triggers::center(world, &trig);
        let stand = [c[0], c[1], c[2] + 20.0];
        let down = [stand[0], stand[1], stand[2] - 300.0];
        let tr = frame(world).trace_static_world(
            stand,
            down,
            crate::bullet_collision::PLAYER_MINS,
            crate::bullet_collision::PLAYER_MAXS,
            crate::bullet_collision::MASK_PLAYER_SOLID,
        );
        let feet: [f32; 3] = std::array::from_fn(|k| stand[k] + (down[k] - stand[k]) * tr.fraction);
        super::teleport_player(world, id, feet);
        let d = [c[0] - feet[0], c[1] - feet[1], c[2] - feet[2] - 60.0];
        let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let view = [
            -d[2].atan2(flat).to_degrees(),
            d[1].atan2(d[0]).to_degrees(),
            0.0,
        ];
        super::set_player_view(world, id, view);
        diag::info!(
            Sim,
            "bo2zm t6 dz use test: {name} trigger centre ({:.0} {:.0} {:.0}), player at ({:.0} {:.0} {:.0})",
            c[0],
            c[1],
            c[2],
            feet[0],
            feet[1],
            feet[2]
        );
    }
    if (1000..1500).contains(&t) {
        let mut req = world.resource_mut::<crate::step::StepRequest>();
        for (cid, cmd) in &mut req.input.cmds {
            if *cid == id {
                cmd.buttons |= playerstate_iw4::buttons::USE;
            }
        }
    }
    if (4000..4000 + tick).contains(&t) {
        let (score, power) = with_vm(world, |vm, world| {
            let f = vm.intern("score");
            let score = vm.get_field(world, p, f).as_int().unwrap_or(-1);
            let flag = vm.intern("flag");
            let power = match vm.raw_field(vm.level, flag) {
                Value::Array(a) => {
                    let k = Key::Str(vm.intern("power_on"));
                    a.get(&k).is_some_and(|v| gsc_t6::truthy(&v))
                }
                _ => false,
            };
            (score, power)
        })
        .unwrap_or((-1, false));
        let zm = world.resource::<Zm>();
        let perks: Vec<String> = zm
            .players
            .get(&0)
            .map(|p| p.perks.iter().cloned().collect())
            .unwrap_or_default();
        let hint = zm.hints.get(&0).cloned().unwrap_or_default();
        let at = frame(world).player(id).map(|ps| ps.origin.map(f32::round));
        diag::info!(
            Sim,
            "bo2zm t6 dz use test: after {name}: player at {at:?}, points {score}, power_on {power}, perks {perks:?}, hint {hint:?}"
        );
    }
}

/// zzz_zm_roundfix.gsc: the leaked maps set `level._round_start_func =
/// _zm::round_start`, which no retail map does, so both the game type
/// (`zclassic` main -> `round_start`) and the map's
/// `post_all_players_connected` thread `round_think`: two round counters,
/// rounds go 1, 3, 5... Once the map has set the field, make it a no-op so
/// the game type's driver alone remains, as on retail maps. True when done.
fn round_driver(world: &mut World) -> bool {
    with_vm(world, |vm, world| {
        let level = vm.level;
        let f = vm.intern("_round_start_func");
        if matches!(vm.get_field(world, level, f), Value::Undefined) {
            return false;
        }
        let noop = vm.intern("declassified_round_start_noop");
        vm.set_field(world, level, f, Value::Func(FuncRef::Missing(noop)));
        diag::info!(
            Sim,
            "bo2zm t6 declassified: map-side round driver no-opped (zzz_zm_roundfix)"
        );
        true
    })
    .unwrap_or(true)
}

/// IW4L_T6_DZZONES=<seconds>: then, once, the scripts' zones: which are
/// active, and how many use triggers (unitrigger stubs) each holds
/// (a box or wall weapon is only offered in an active zone).
fn zones_log(world: &mut World, now: i64) {
    static AT: std::sync::OnceLock<Option<i64>> = std::sync::OnceLock::new();
    let Some(at) = *AT.get_or_init(|| {
        std::env::var("IW4L_T6_DZZONES")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .map(|s| s * 1000)
    }) else {
        return;
    };
    if !(at..at + i64::from(crate::MATCH_TICK_MS)).contains(&now) {
        return;
    }
    with_vm(world, |vm, world| {
        let level = Value::Object(vm.level);
        let active = path(vm, world, &level, &["active_zone_names"]);
        diag::info!(
            Sim,
            "bo2zm t6 dz zones: active {}",
            items(vm, &active).join(", ")
        );
        let zones = path(vm, world, &level, &["zones"]);
        let Value::Array(a) = &zones else {
            diag::info!(
                Sim,
                "bo2zm t6 dz zones: level.zones is {}",
                zones.type_name()
            );
            return;
        };
        let keys: Vec<Key> = a.read().keys().collect();
        for k in keys {
            let z = a.get(&k).unwrap_or_default();
            let f = |vm: &mut Vm<World>, world: &mut World, n: &str| {
                let v = path(vm, world, &z, &[n]);
                match &v {
                    Value::Array(x) => format!("[{}]", x.len()),
                    _ => vm.to_text(&v),
                }
            };
            let (en, ac, st, vol) = (
                f(vm, world, "is_enabled"),
                f(vm, world, "is_active"),
                f(vm, world, "unitrigger_stubs"),
                f(vm, world, "volumes"),
            );
            diag::info!(
                Sim,
                "bo2zm t6 dz zone {}: enabled {en} active {ac} stubs {st} volumes {vol}",
                vm.to_text(&k.value())
            );
        }
        let u = path(vm, world, &level, &["_unitriggers", "dynamic_stubs"]);
        diag::info!(
            Sim,
            "bo2zm t6 dz zones: dynamic stubs {}",
            items(vm, &u).len()
        );
    });
}

/// IW4L_T6_DZENTS=<seconds>[,<seconds>...]:<text>[,<text>...]: at each of
/// those times, every entity whose targetname, classname, model or
/// script_noteworthy holds one of the texts: place, model, hidden.
fn ents_log(world: &mut World, now: i64) {
    static SPEC: std::sync::OnceLock<(Vec<i64>, Vec<String>)> = std::sync::OnceLock::new();
    let (times, texts) = SPEC.get_or_init(|| {
        let v = std::env::var("IW4L_T6_DZENTS").unwrap_or_default();
        let (t, w) = v.split_once(':').unwrap_or(("", ""));
        (
            t.split(',')
                .filter_map(|s| s.parse::<i64>().ok())
                .map(|s| s * 1000)
                .collect(),
            w.split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    });
    let tick = i64::from(crate::MATCH_TICK_MS);
    if !times.iter().any(|at| (*at..at + tick).contains(&now)) {
        return;
    }
    with_vm(world, |vm, world| {
        let (tn, sn) = (vm.intern("targetname"), vm.intern("script_noteworthy"));
        let zm = world.resource::<Zm>();
        let mut lines = Vec::new();
        for (n, e) in &zm.ents {
            let (name, note) = e.obj.map_or_else(Default::default, |o| {
                (
                    vm.to_text(&vm.raw_field(o, tn)),
                    vm.to_text(&vm.raw_field(o, sn)),
                )
            });
            let hay = [&name, &note, &e.classname, &e.model];
            if texts
                .iter()
                .any(|t| hay.iter().any(|h| h.contains(t.as_str())))
            {
                let o = e.origin;
                lines.push(format!(
                    "ent{n} {} {name:?} {note:?} model {:?} at ({:.0} {:.0} {:.0}) yaw {:.0} hidden {}",
                    e.classname, e.model, o[0], o[1], o[2], e.angles[1], e.hidden
                ));
            }
        }
        lines.sort();
        for l in lines {
            diag::info!(Sim, "bo2zm t6 dz ents {}s: {l}", now / 1000);
        }
    });
}

/// IW4L_T6_DZDOGROUND=<round>: at 10 s, `level.next_dog_round` is set to
/// that round (the dog tracker picks it at the start), so the hellhounds
/// come then (test aid).
fn dog_round_test(world: &mut World, now: i64) {
    static AT: std::sync::OnceLock<Option<i32>> = std::sync::OnceLock::new();
    let Some(round) = *AT.get_or_init(|| {
        std::env::var("IW4L_T6_DZDOGROUND")
            .ok()
            .and_then(|s| s.parse().ok())
    }) else {
        return;
    };
    if !(10000..10000 + i64::from(crate::MATCH_TICK_MS)).contains(&now) {
        return;
    }
    with_vm(world, |vm, world| {
        let f = vm.intern("next_dog_round");
        let level = vm.level;
        let was = vm.get_field(world, level, f);
        vm.set_field(world, level, f, Value::Int(round));
        diag::info!(
            Sim,
            "bo2zm t6 dz dogs: next_dog_round {} -> {round}",
            vm.to_text(&was)
        );
    });
}

/// `v.a.b...` (undefined where a step is missing).
fn path(vm: &mut Vm<World>, world: &mut World, v: &Value, names: &[&str]) -> Value {
    let mut v = v.clone();
    for n in names {
        let Value::Object(o) = v else {
            return Value::Undefined;
        };
        let f = vm.intern(n);
        v = vm.get_field(world, o, f);
    }
    v
}

fn items(vm: &Vm<World>, v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.read().values_in_order().map(|x| vm.to_text(x)).collect(),
        _ => Vec::new(),
    }
}
