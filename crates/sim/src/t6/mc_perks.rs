//! bo2mc (Minecraft Zombies): every Black Ops II perk in the spawn room.
//!
//! His ask (10-04): "In this room will be every single perk and the mystery
//! box." Nuketown's scripts place its own four machines and Pack-a-Punch
//! (`mc_rules::before_zm_init` moves them into the room). The other eight
//! are BO2's own perks run by BO2's own scripts: the generic perk script
//! (`_zm_perks`) already sells Stamin-Up, Deadshot, Mule Kick, Tombstone
//! and Who's Who once a map turns them on (`level.zombiemode_using_*`);
//! Electric Cherry, Vulture Aid and PhD Flopper register themselves
//! (`enable_*_perk_for_level`, as Mob of the Dead and Buried do). Their
//! machines, bottles, icons and jingles come from the other maps' zones
//! (`assets::lane::t6_perks`). Here:
//! - `install`: the perks are turned on before `_zm::init`, and right
//!   before `perk_machine_spawn_init` one machine spot per perk is added at
//!   the room's free spots (`EXTRA`);
//! - `tick`: the machines power on with Nuketown's (two seconds after the
//!   opening black screen).
//! Engine sides of the perks (sprint, fire rate, aim) are `crate::bo2_perks`.
//!
//! Test aids: `IW4L_BO2MC_PERK_LIMIT=<n>` (BO2's own limit is 4 perks),
//! `IW4L_BO2MC_POINTS=<n>` (that many points once the black screen is
//! gone).

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{HookAction, Key, ObjRef, Value, Vm};

use super::{Zm, with_vm};
use crate::bo2mc;

/// The eight perks Nuketown lacks: (specialty, room spot
/// (`bo2mc::perk_spots`), machine model while off). Spots 1, 4, 7, 10 hold
/// Nuketown's Quick Revive, Juggernog, Speed Cola and Double Tap.
pub const EXTRA: [(&str, usize, &str); 8] = [
    ("specialty_longersprint", 0, "zombie_vending_marathon"),
    ("specialty_additionalprimaryweapon", 2, "zombie_vending_three_gun"),
    ("specialty_deadshot", 3, DEADSHOT_MODEL),
    ("specialty_scavenger", 5, "zombie_vending_tombstone"),
    ("specialty_grenadepulldeath", 6, "p6_zm_vending_electric_cherry_off"),
    ("specialty_finalstand", 8, "p6_zm_vending_chugabud"),
    ("specialty_nomotionsensor", 9, "p6_zm_vending_vultureaid"),
    ("specialty_flakjacket", 11, PHD_MODEL),
];

/// Deadshot's machine: Mob of the Dead's is the only one in his files (the
/// generic script's `zombie_vending_ads` is in no zone), lit only.
const DEADSHOT_MODEL: &str = "p6_zm_al_vending_ads_on";

/// PhD Flopper's machine: its script's `zombie_vending_nuke` is in no
/// zone; Die Rise's (lit only) is.
const PHD_MODEL: &str = "zombie_vending_nuke_on_lo";

/// What the machines wait on to power up (`turn_*_on`, the custom perks'
/// machine threads).
const POWER_ON: [&str; 8] = [
    "marathon_on",
    "additionalprimaryweapon_on",
    "deadshot_on",
    "tombstone_on",
    "chugabud_on",
    "electric_cherry_on",
    "specialty_nomotionsensor_on",
    "divetonuke_on",
];

#[derive(Resource, Default)]
struct McPerks {
    /// When the machines power on (ms), -1 once done.
    power_at: Option<i64>,
    points_given: bool,
    /// Player 0 was in a dive's flight last tick.
    diving: bool,
    /// His perks last tick (the log line when they change).
    perks_seen: Vec<String>,
    /// What his client was last sent of Vulture Aid's sight.
    vulture_sent: String,
    /// The down test's time (ms), -1 once done.
    down_at: Option<i64>,
    /// Player 0 was down last tick.
    was_down: bool,
}

/// Field of an object or array value by name.
fn field(vm: &mut Vm<World>, v: &Value, name: &str) -> Value {
    match v {
        Value::Object(o) => {
            let f = vm.intern(name);
            vm.raw_field(*o, f)
        }
        Value::Array(a) => {
            let k = Key::Str(vm.intern(name));
            a.get(&k).unwrap_or_default()
        }
        _ => Value::Undefined,
    }
}

fn set_field(vm: &mut Vm<World>, o: ObjRef, name: &str, v: Value) {
    let f = vm.intern(name);
    vm.set_raw_field(o, f, v);
}

fn env_int(name: &str) -> Option<i32> {
    std::env::var(name).ok().and_then(|s| s.trim().parse().ok())
}

pub(super) fn install(vm: &mut Vm<World>) {
    let hooks: &[(&str, &str, gsc_t6::FnHook<World>)] = &[
        ("maps/mp/zm_nuked_perks", "init_nuked_perks", turn_perks_on_for_level),
        ("maps/mp/zombies/_zm_perks", "perk_machine_spawn_init", add_machines),
        ("maps/mp/zombies/_zm_perks", "turn_deadshot_on", machine_models),
        ("maps/mp/zombies/_zm_perk_divetonuke", "divetonuke_perk_machine_think", machine_models),
        ("maps/mp/zombies/_zm_chugabud", "chugabud_get_spawnpoint", whos_who_spawn),
        // Guns are Minecraft items without BO2's limit (his call, 10-05):
        // going down with Mule Kick takes no gun away.
        ("maps/mp/zombies/_zm", "take_additionalprimaryweapon", nothing),
    ];
    let mut missing = Vec::new();
    for (script, name, hook) in hooks {
        if !vm.hook_function(script, name, *hook) {
            missing.push(*name);
        }
    }
    diag::info!(
        Sim,
        "bo2mc perks: {} hooks{}",
        hooks.len() - missing.len(),
        if missing.is_empty() { String::new() } else { format!(", missing {}", missing.join(" ")) }
    );
}

/// The perks that turn themselves on, and their scripts.
const ENABLE: [(&str, &str); 4] = [
    ("maps/mp/zombies/_zm_perk_electric_cherry", "enable_electric_cherry_perk_for_level"),
    ("maps/mp/zombies/_zm_perk_vulture", "enable_vulture_perk_for_level"),
    ("maps/mp/zombies/_zm_perk_divetonuke", "enable_divetonuke_perk_for_level"),
    ("maps/mp/zombies/_zm_chugabud", "init"),
];

/// Right before `_zm::init` (Nuketown's `init_nuked_perks`): the other
/// perks are on for this level, as their own maps turn them on.
fn turn_perks_on_for_level(vm: &mut Vm<World>, world: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let level = vm.level;
    for flag in [
        "zombiemode_using_marathon_perk",
        "zombiemode_using_deadshot_perk",
        "zombiemode_using_additionalprimaryweapon_perk",
        "zombiemode_using_tombstone_perk",
        "zombiemode_using_chugabud_perk",
    ] {
        set_field(vm, level, flag, Value::Int(1));
    }
    let mut on = Vec::new();
    for (script, func) in ENABLE {
        match vm.spawn_named(world, script, func, Value::Object(level), Vec::new()) {
            Some(_) => on.push(func),
            None => diag::warn!(Sim, "bo2mc perks: {script}::{func} not loaded"),
        }
    }
    diag::info!(Sim, "bo2mc perks: turned on for the level: {}", on.join(" "));
    HookAction::Continue
}

/// `perk_machine_spawn_init` spawns a machine at every perk struct: the
/// other perks' structs join Nuketown's at the room's free spots.
fn add_machines(vm: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let level = Value::Object(vm.level);
    if let Some(n) = env_int("IW4L_BO2MC_PERK_LIMIT") {
        let l = vm.level;
        set_field(vm, l, "perk_purchase_limit", Value::Int(n));
    }
    let tn = field(vm, &level, "override_perk_targetname");
    let name = if tn.is_undefined() { "zm_perk_machine".to_owned() } else { vm.to_text(&tn) };
    let names = field(vm, &level, "struct_class_names");
    let by_tn = field(vm, &names, "targetname");
    let Value::Array(arr) = field(vm, &by_tn, &name) else {
        diag::warn!(Sim, "bo2mc perks: no {name} structs");
        return HookAction::Continue;
    };
    let spots = bo2mc::perk_spots();
    let mut placed = Vec::new();
    for (perk, spot, model) in EXTRA {
        let Some(&(pos, face)) = spots.get(spot) else { continue };
        let Value::Object(s) = vm.new_struct() else { continue };
        set_field(vm, s, "origin", Value::Vec3(pos));
        // A machine's front is 90 degrees right of its angles.
        set_field(vm, s, "angles", Value::Vec3([0.0, face + 90.0, 0.0]));
        let v = vm.string(perk);
        set_field(vm, s, "script_noteworthy", v);
        let v = vm.string(model);
        set_field(vm, s, "model", v);
        let v = vm.string(&name);
        set_field(vm, s, "targetname", v);
        arr.write().push(Value::Object(s));
        placed.push(perk);
    }
    diag::info!(Sim, "bo2mc perks: machines added to the room: {}", placed.join(" "));
    HookAction::Continue
}

/// Deadshot's and PhD Flopper's machines: the models their scripts name
/// are in none of his zones; the lit ones that are stand in, off and on.
fn machine_models(vm: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let level = Value::Object(vm.level);
    let assets = field(vm, &level, "machine_assets");
    for (perk, model) in [("deadshot", DEADSHOT_MODEL), ("divetonuke", PHD_MODEL)] {
        if let Value::Object(o) = field(vm, &assets, perk) {
            for f in ["off_model", "on_model"] {
                let v = vm.string(model);
                set_field(vm, o, f, v);
            }
        }
    }
    HookAction::Continue
}

/// Who's Who: Black Ops II puts the player back on his feet 500-700 units
/// away on the map's path nodes (Nuketown's, meaningless on the Minecraft
/// world, often inside the ground). Here he gets up in the spawn room, at
/// the room spot farthest from where he went down; his body stays where he
/// fell, for him to revive.
fn whos_who_spawn(vm: &mut Vm<World>, world: &mut World, s: &Value, _: &[Value]) -> HookAction {
    let Some(down) = super::entnum(vm, s)
        .and_then(|n| super::frame(world).player(crate::world::ClientId(n)).map(|ps| ps.origin))
    else {
        return HookAction::Continue;
    };
    let spot = [(-4, 3), (4, 3), (-4, -3), (4, -3)]
        .into_iter()
        .map(|(dx, dz)| bo2mc::cell_floor(dx, dz))
        .max_by(|a, b| {
            let d = |p: &[f32; 3]| (p[0] - down[0]).hypot(p[1] - down[1]);
            d(a).total_cmp(&d(b))
        })
        .unwrap_or([0.0; 3]);
    let Value::Object(st) = vm.new_struct() else {
        return HookAction::Continue;
    };
    set_field(vm, st, "origin", Value::Vec3([spot[0], spot[1], spot[2] + 2.0]));
    set_field(vm, st, "angles", Value::Vec3([0.0, 90.0, 0.0]));
    diag::info!(Sim, "bo2mc perks: Who's Who gets him up in the room at {spot:?} (down at {down:?})");
    HookAction::Return(Value::Object(st))
}

/// Damage to player 0 through BO2's own damage callback (as the engine's
/// `natives_ai::player_damage`, without its IW4L_T6_GOD test switch: the
/// PhD landing and the down test are the scripts' own damage).
fn script_damage(world: &mut World, amount: i32, means: &str, at: [f32; 3]) {
    let Some(obj) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) else {
        return;
    };
    with_vm(world, |vm, world| {
        let means = vm.string(means);
        let weapon = vm.string("none");
        let hitloc = vm.string("none");
        let args = vec![
            Value::Undefined,
            Value::Undefined,
            Value::Int(amount),
            Value::Int(0),
            means,
            weapon,
            Value::Vec3(at),
            Value::Vec3([0.0, 0.0, -1.0]),
            hitloc,
            Value::Int(0),
            Value::Int(0),
        ];
        vm.spawn_named(
            world,
            "maps/mp/gametypes_zm/_callbacksetup",
            "codecallback_playerdamage",
            Value::Object(obj),
            args,
        );
    });
}

/// IW4L_BO2MC_DOWN_AT=<n> (test aid): five seconds after he has n perks,
/// one blow takes all his health (he goes down: Who's Who, Tombstone, solo
/// Quick Revive).
fn down_test(world: &mut World, now: i64) {
    let Some(n) = env_int("IW4L_BO2MC_DOWN_AT") else { return };
    let count = world.resource::<Zm>().players.get(&0).map_or(0, |p| p.perks.len());
    let at = world.resource::<McPerks>().down_at;
    match at {
        None if count >= n as usize => world.resource_mut::<McPerks>().down_at = Some(now + 5000),
        Some(t) if t > 0 && now >= t => {
            let origin = super::frame(world).player(crate::world::ClientId(0)).map_or([0.0; 3], |ps| ps.origin);
            script_damage(world, 1000, "MOD_MELEE", origin);
            world.resource_mut::<McPerks>().down_at = Some(-1);
            diag::info!(Sim, "bo2mc perks test: down with {count} perks at {}s", now / 1000);
        }
        _ => {}
    }
}

fn nothing(_: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    HookAction::Return(Value::Undefined)
}

fn level_flag(vm: &mut Vm<World>, name: &str) -> bool {
    let level = Value::Object(vm.level);
    let flags = field(vm, &level, "flag");
    gsc_t6::truthy(&field(vm, &flags, name))
}

pub(super) fn tick(world: &mut World, now: i64) {
    if !bo2mc::enabled() || !world.resource::<Zm>().started {
        return;
    }
    if !world.contains_resource::<McPerks>() {
        world.insert_resource(McPerks::default());
    }
    match world.resource::<McPerks>().power_at {
        Some(at) if at < 0 => {}
        Some(at) if now >= at => {
            with_vm(world, |vm, world| {
                let level = vm.level;
                for name in POWER_ON {
                    vm.notify_str(world, level, name, &[]);
                }
            });
            diag::info!(Sim, "bo2mc perks: the other machines turned on at {}s", now / 1000);
            world.resource_mut::<McPerks>().power_at = Some(-1);
        }
        Some(_) => {}
        None => {
            if with_vm(world, |vm, _| level_flag(vm, "initial_blackscreen_passed")).unwrap_or(false) {
                world.resource_mut::<McPerks>().power_at = Some(now + 2000);
            }
        }
    }
    give_test_points(world);
    dive_lands(world);
    log_perks(world);
    vulture_sight(world);
    down_test(world, now);
    tombstone_solo(world);
}

/// Tombstone, solo. In Black Ops II its stone only appears when a player
/// who bled out spawns again next round (co-op); solo, going down without
/// Quick Revive ends the game, and with it the stone is deleted as he gets
/// up, so solo Tombstone did nothing. BO2's own `_zm` keeps the perks a
/// downed Tombstone player had (`player_laststand` ->
/// `tombstone_save_perks` into `self.laststand_perks`) and has the function
/// that gives them back but one at random (`laststand_giveback_player_perks`),
/// which nothing in the shipped scripts calls. Here it runs when he gets up
/// from a solo Quick Revive with Tombstone: he keeps his guns (Quick
/// Revive's own) and his perks but one, once (Tombstone itself is used up).
fn tombstone_solo(world: &mut World) {
    let Some((down, obj)) = world.resource::<Zm>().players.get(&0).map(|p| (p.laststand, p.obj)) else {
        return;
    };
    let was = std::mem::replace(&mut world.resource_mut::<McPerks>().was_down, down);
    if !was || down {
        return;
    }
    let given = with_vm(world, |vm, world| {
        let saved = field(vm, &Value::Object(obj), "laststand_perks");
        if saved.is_undefined() {
            return false;
        }
        vm.spawn_named(world, "maps/mp/zombies/_zm", "laststand_giveback_player_perks", Value::Object(obj), Vec::new());
        set_field(vm, obj, "laststand_perks", Value::Undefined);
        true
    })
    .unwrap_or(false);
    if given {
        diag::info!(Sim, "bo2mc perks: Tombstone gives his perks back (but one)");
    }
}

/// Each machine's HUD icon (Vulture Aid's marks), by its room spot.
const SPOT_ICONS: [&str; 12] = [
    "specialty_marathon_zombies",
    "specialty_quickrevive_zombies",
    "specialty_additionalprimaryweapon_zombies",
    "specialty_ads_zombies",
    "specialty_juggernaut_zombies",
    "specialty_tombstone_zombies",
    "specialty_electric_cherry_zombie",
    "specialty_fastreload_zombies",
    "specialty_chugabud_zombies",
    "specialty_vulture_zombies",
    "specialty_doubletap_zombies",
    "specialty_divetonuke_zombies",
];

/// Vulture Aid: while he has it, his client shows every perk machine and
/// the Mystery Box through walls (`ui::vulture_hud`), as BO2's client
/// script glows them.
fn vulture_sight(world: &mut World) {
    let on = world
        .resource::<Zm>()
        .players
        .get(&0)
        .is_some_and(|p| p.perks.contains("specialty_nomotionsensor"));
    let list = if on {
        let mut out: Vec<String> = bo2mc::perk_spots()
            .iter()
            .zip(SPOT_ICONS)
            .map(|((p, _), icon)| format!("{:.0} {:.0} {:.0} {icon}", p[0], p[1], p[2] + 60.0))
            .collect();
        let (b, _) = bo2mc::box_spot();
        out.push(format!("{:.0} {:.0} {:.0} specialty_firesale_zombies", b[0], b[1], b[2] + 40.0));
        out.join(";")
    } else {
        String::new()
    };
    if list == world.resource::<McPerks>().vulture_sent {
        return;
    }
    super::set_client_dvar(world, 0, "bo2mc_vulture", &list);
    diag::info!(Sim, "bo2mc perks: Vulture Aid's sight {}", if on { "on" } else { "off" });
    world.resource_mut::<McPerks>().vulture_sent = list;
}

/// A line in the log whenever his perks change: which, and his points.
fn log_perks(world: &mut World) {
    let Some(mut now) = world.resource::<Zm>().players.get(&0).map(|p| p.perks.iter().cloned().collect::<Vec<_>>())
    else {
        return;
    };
    now.sort();
    if now == world.resource::<McPerks>().perks_seen {
        return;
    }
    let points = super::frame(world).client_meta(crate::world::ClientId(0)).map_or(0, |m| m.score);
    diag::info!(Sim, "bo2mc perks: he has {} perks, {points} points: {}", now.len(), now.join(" "));
    world.resource_mut::<McPerks>().perks_seen = now;
}

/// PhD Flopper: Black Ops II's damage script sets off the explosion when a
/// diving player with the perk takes falling damage (`player_damage_override`
/// -> `zombiemode_divetonuke_perk_func`, the damage itself then 0). The
/// engine's dive landing is that fall: a dive's flight ending on the ground
/// hands the scripts a small MOD_FALLING hit, with PhD only.
fn dive_lands(world: &mut World) {
    let Some(ps) = super::frame(world).player(crate::world::ClientId(0)).copied() else {
        return;
    };
    let flying = ps.pm_flags & movement_iw4::PMF_DIVE != 0;
    let landed = ps.pm_flags & movement_iw4::PMF_DIVE_SLIDE != 0;
    let was = std::mem::replace(&mut world.resource_mut::<McPerks>().diving, flying);
    if flying && !was {
        diag::info!(Sim, "bo2mc perks: dive at {:?}", ps.origin);
    }
    if !(was && landed) {
        return;
    }
    let phd = world
        .resource::<Zm>()
        .players
        .get(&0)
        .is_some_and(|p| p.perks.contains("specialty_flakjacket"));
    if !phd {
        return;
    }
    script_damage(world, 10, "MOD_FALLING", ps.origin);
    diag::info!(Sim, "bo2mc perks: PhD dive landed at {:?}", ps.origin);
}

/// IW4L_BO2MC_POINTS=<n>: n points once the black screen is gone (tests).
fn give_test_points(world: &mut World) {
    let Some(n) = env_int("IW4L_BO2MC_POINTS") else { return };
    if world.resource::<McPerks>().points_given || world.resource::<McPerks>().power_at.is_none() {
        return;
    }
    let Some(obj) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) else { return };
    with_vm(world, |vm, world| {
        vm.spawn_named(
            world,
            "maps/mp/zombies/_zm_score",
            "add_to_player_score",
            Value::Object(obj),
            vec![Value::Int(n)],
        );
    });
    world.resource_mut::<McPerks>().points_given = true;
    diag::info!(Sim, "bo2mc perks test: {n} points");
}
