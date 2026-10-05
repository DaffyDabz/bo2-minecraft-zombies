//! bo2mc (Minecraft Zombies): Black Ops II's Nuketown rules played on the
//! Minecraft world (`IW4L_BO2MC=1`). The BO2 scripts run as in Nuketown;
//! this glue moves what they place into the spawn room, quiets Nuketown's
//! own set pieces and ties the rounds to the world's nights:
//! - the map's entities (`filter_map_entities`): Nuketown's props, doors,
//!   clips and triggers go; the player starts, the one magic box and the wall
//!   buys move into the room (`crate::bo2mc` holds its fixed layout);
//! - script hooks (`install`): the perk machines stand in the room from the
//!   start (no perks from the sky), the box never moves, Nuketown's clocks,
//!   mannequins, signs, bus and story lines stay quiet, the Carpenter gives
//!   wood and doors, and a finished round waits for the next nightfall;
//! - every tick (`tick`): zombies rise where the world side says (around the
//!   player, right by him underground), ones left behind or stuck rise again
//!   closer, the clock waits at midnight for the round, sunset warns a player
//!   underground, and falls hurt through BO2's own player damage.

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Array, HookAction, Key, ObjKind, ObjRef, Value, Vm};

use super::{Zm, frame, with_vm};
use crate::bo2mc::{self, Ask, Request, WeaponItem};
use crate::world::ClientId;

/// The flag a finished round waits on until night falls.
const NIGHT: &str = "bo2mc_night";
/// Day clock: night falls, midnight, dawn.
const NIGHTFALL: f64 = 13000.0;
const MIDNIGHT: f64 = 18000.0;
const DAWN: f64 = 23000.0;
const SUNSET_WARN: f64 = 11500.0;
/// The magic box Nuketown starts at, the one kept.
const KEPT_CHEST: &str = "start_chest1";
/// Spawn spots handed to BO2's spawning at most.
const POOL: usize = 32;

/// The rules side's state for one game (a restart starts a new one).
#[derive(Resource, Default)]
pub(crate) struct McRules {
    pool: Vec<ObjRef>,
    pool_live: usize,
    pool_ms: i64,
    /// A round is over and the next waits for night.
    pub holding: bool,
    holding_since: i64,
    morning_seen: bool,
    /// The clock waits at midnight for the round.
    paused: bool,
    warned_ms: i64,
    /// When the clock was last asked to run again (between rounds).
    unpause_ms: i64,
    /// Every zone was enabled (Nuketown's houses have no doors here).
    zones_on: bool,
    /// The armor he wears (Minecraft points, toughness), from the world side.
    armor: (f32, f32),
    /// He holds the knife item: his attack is a knife swing.
    knife: bool,
    /// The ammo station's trigger (entity number), and use held last tick.
    station: Option<u32>,
    /// The inventory's body model (entity number) and the idle it plays.
    puppet: Option<u32>,
    puppet_anim: Option<String>,
    use_was: bool,
    perks_on: Option<i64>,
    /// The rules side's own clock while the world side runs none: ticks
    /// and paused.
    fake: Option<(f64, bool)>,
    /// The world side publishes the day clock.
    world_clock: bool,
    rounds_logged: i32,
    /// His items as last logged (names and indices), to log changes.
    items_logged: String,
    points_given: bool,
    /// His most health last tick: Juggernog's rise comes as full golden
    /// hearts (BO2's regen, which would fill them, is off).
    max_seen: i32,
}

/// Guns he may own at once (playtest 1b: every gun is an item in its own
/// slot, no BO2 2-gun limit). BO2's player holds 15 weapons in all; the
/// knife, grenades, mines, monkeys and a perk bottle or last-stand pistol
/// need theirs, so 8 guns is the most that always fits.
const GUN_LIMIT: i32 = 8;

/// The world side runs (the block world stands in for the map): requests
/// reach someone.
fn world_side() -> bool {
    crate::voxel::active()
}

/// IW4L_BO2MC_FAKE_DAY=1: with no world side, a day clock here (tests of
/// the night rounds on plain Nuketown).
fn fake_day() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("IW4L_BO2MC_FAKE_DAY").is_ok_and(|v| v == "1"))
}

fn day_speed() -> f64 {
    static SPEED: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SPEED.get_or_init(|| {
        std::env::var("IW4L_BO2MC_DAY_SPEED")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0)
    })
}

/// The day clock runs (the world side's or the test clock).
fn clock_live() -> bool {
    world_side() || fake_day()
}

/// A request for the world side, dropped when none runs (they would pile
/// up).
fn push(r: Request) {
    if world_side() || fake_day() {
        bo2mc::push(r);
    }
}

/// A clock request, also carried out on the rules side's own clock while
/// the world side runs none.
fn push_clock(world: &mut World, r: Request) {
    if let Some(mut rules) = world.get_resource_mut::<McRules>()
        && let Some((t, paused)) = rules.fake.as_mut()
    {
        match &r {
            Request::PauseDay(p) => *paused = *p,
            Request::SetDay(x) => *t = *x,
            _ => {}
        }
        bo2mc::set_day(*t, *paused);
    }
    push(r);
}

// ------------------------------------------------------------ map entities

fn get<'a>(e: &'a [(String, String)], k: &str) -> Option<&'a str> {
    e.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(k))
        .map(|(_, v)| v.as_str())
}

fn set(e: &mut Vec<(String, String)>, k: &str, v: String) {
    match e.iter_mut().find(|(key, _)| key.eq_ignore_ascii_case(k)) {
        Some(row) => row.1 = v,
        None => e.push((k.to_owned(), v)),
    }
}

fn vec3(s: Option<&str>) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (i, t) in s.unwrap_or("").split_whitespace().take(3).enumerate() {
        out[i] = t.parse().unwrap_or(0.0);
    }
    out
}

fn text3(v: [f32; 3]) -> String {
    format!("{} {} {}", v[0], v[1], v[2])
}

/// Does this Nuketown map entity stay on the block world?
fn keep(e: &[(String, String)]) -> bool {
    let cls = get(e, "classname").unwrap_or("");
    let targetname = get(e, "targetname").unwrap_or("");
    let noteworthy = get(e, "script_noteworthy").unwrap_or("");
    match cls {
        // Props, clocks, mannequins, doors and the crates over the perk
        // spots; only the rocket of the game-over scene stays.
        "script_model" => targetname == "intermission_rocket",
        // Clips, doors, debris and their triggers, the bus, the bunker,
        // sounds and glass: Nuketown's houses are not here.
        "script_brushmodel" | "trigger_multiple" | "trigger_use_touch" | "trigger_damage"
        | "trigger_use" | "trigger_radius" | "trigger_hurt" | "trigger_once" | "glass" => false,
        c if c.starts_with("zbarrier") => noteworthy == format!("{KEPT_CHEST}_zbarrier"),
        "script_struct" if targetname == "treasure_chest_use" => noteworthy == KEPT_CHEST,
        _ => true,
    }
}

/// A rigid move: everything turns `yaw` degrees about `pivot` and goes to
/// `to`.
#[derive(Clone, Copy)]
struct Shift {
    pivot: [f32; 3],
    to: [f32; 3],
    yaw: f32,
}

impl Shift {
    fn to(from: [f32; 3], from_yaw: f32, to: [f32; 3], to_yaw: f32) -> Self {
        Self {
            pivot: from,
            to,
            yaw: to_yaw - from_yaw,
        }
    }
    fn apply(&self, e: &mut Vec<(String, String)>) {
        let o = vec3(get(e, "origin"));
        let d = [o[0] - self.pivot[0], o[1] - self.pivot[1], o[2] - self.pivot[2]];
        let (s, c) = self.yaw.to_radians().sin_cos();
        let p = [
            self.to[0] + d[0] * c - d[1] * s,
            self.to[1] + d[0] * s + d[1] * c,
            self.to[2] + d[2],
        ];
        set(e, "origin", text3(p));
        let mut a = vec3(get(e, "angles"));
        a[1] += self.yaw;
        set(e, "angles", text3(a));
    }
}

/// Where the wall buys go: (map point on the wall, the yaw the wall faces
/// into the room). The long walls first (round the box, the doors and
/// Pack-a-Punch), then the short walls between the perk machines.
fn wall_spots() -> Vec<([f32; 3], f32)> {
    const B: f32 = 36.0;
    // At his eye (60): the wall buy is used by looking at its chalk from
    // right in front of it.
    const Z: f32 = 60.0;
    let hz = bo2mc::ROOM_HALF_Z as f32 + 0.5;
    let hx = bo2mc::ROOM_HALF_X as f32 + 0.5;
    let mut out = Vec::new();
    for dx in [-7, 2, 4, 6] {
        out.push(([dx as f32 * B, hz * B - 1.0, Z], -90.0));
    }
    for dx in [-2, -4, -6, 7] {
        out.push(([dx as f32 * B, -hz * B + 1.0, Z], 90.0));
    }
    for dz in [-1, 1] {
        out.push(([-hx * B + 1.0, -(dz as f32) * B, Z], 0.0));
        out.push(([hx * B - 1.0, -(dz as f32) * B, Z], 180.0));
    }
    out
}

/// The player starts: round the middle of the room, facing the north door.
fn start_spots() -> Vec<[f32; 3]> {
    [[0, 0], [1, 0], [-1, 0], [0, 1], [1, 1], [-1, 1], [2, 0], [-2, 0]]
        .iter()
        .map(|&[dx, dz]| bo2mc::cell_floor(dx, dz))
        .collect()
}

/// Nuketown's map entities on the block world: what makes no sense there
/// goes, the starts, the box and the wall buys move into the spawn room.
pub(super) fn filter_map_entities(ents: &mut Vec<Vec<(String, String)>>) {
    let before = ents.len();
    ents.retain(|e| keep(e));
    let dropped = before - ents.len();
    let (start_pos, start_yaw) = bo2mc::player_start();
    let starts = start_spots();
    let walls = wall_spots();
    let (box_pos, box_face) = bo2mc::box_spot();
    // Rigid moves for groups: by the targetname of the structs they reach.
    let mut by_target: BTreeMap<String, Shift> = BTreeMap::new();
    let (mut n_start, mut n_wall) = (0usize, 0usize);
    for e in ents.iter_mut() {
        let cls = get(e, "classname").unwrap_or("").to_owned();
        let targetname = get(e, "targetname").unwrap_or("").to_owned();
        let noteworthy = get(e, "script_noteworthy").unwrap_or("").to_owned();
        let origin = vec3(get(e, "origin"));
        let yaw = vec3(get(e, "angles"))[1];
        if cls == "script_struct" && noteworthy == "initial_spawn" {
            let p = starts[n_start % starts.len()];
            n_start += 1;
            set(e, "origin", text3([p[0], p[1], p[2] + 1.0]));
            set(e, "angles", text3([0.0, start_yaw, 0.0]));
        } else if cls == "script_struct" && targetname == "player_respawn_point" {
            set(e, "origin", text3([start_pos[0], start_pos[1], start_pos[2] + 1.0]));
            set(e, "angles", text3([0.0, start_yaw, 0.0]));
        } else if (cls == "script_struct" && targetname == "treasure_chest_use")
            || cls.starts_with("zbarrier")
        {
            // The box's use side is to the left of its angles.
            let s = Shift::to(origin, yaw, box_pos, box_face - 90.0);
            s.apply(e);
        } else if cls == "script_struct"
            && matches!(
                targetname.as_str(),
                "weapon_upgrade" | "tazer_upgrade" | "bowie_upgrade" | "claymore_purchase"
            )
        {
            let Some(&(p, face)) = walls.get(n_wall) else {
                continue;
            };
            n_wall += 1;
            // Its right points into the wall (the bought gun slides out
            // along it).
            let s = Shift::to(origin, yaw, p, face - 90.0);
            if let Some(t) = get(e, "target") {
                by_target.insert(t.to_owned(), s);
            }
            s.apply(e);
        }
    }
    // The wall buys' chalk and model structs (two levels of `target`).
    for _ in 0..2 {
        let mut more = Vec::new();
        for e in ents.iter_mut() {
            if get(e, "classname") != Some("script_struct") {
                continue;
            }
            let Some(s) = get(e, "targetname").and_then(|t| by_target.get(t)).copied() else {
                continue;
            };
            if let Some(t) = get(e, "target") {
                more.push((t.to_owned(), s));
            }
            s.apply(e);
            set(e, "bo2mc_moved", "1".to_owned());
        }
        by_target = more.into_iter().collect();
    }
    diag::info!(
        Sim,
        "bo2mc map: {dropped} of {before} Nuketown entities dropped; {n_start} starts, {n_wall} wall buys and the box moved into the room"
    );
}

// ------------------------------------------------------------ script hooks

pub(super) fn install(vm: &mut Vm<World>) {
    vm.dvars.insert("magic_chest_movable".into(), "0".into());
    let quiet: &[(&str, &str)] = &[
        // Collision patches, exploit fixes and drops at Nuketown spots.
        ("maps/mp/zm_nuked", "nuked_collision_patch"),
        ("maps/mp/zm_nuked_ffotd", "prone_under_garage_door_exploit"),
        ("maps/mp/zm_nuked", "perks_behind_door"),
        // Its set pieces: mannequins, clocks, the town sign, the bus.
        ("maps/mp/zm_nuked", "nuked_mannequin_init"),
        ("maps/mp/zm_nuked", "nuked_doomsday_clock_think"),
        ("maps/mp/zm_nuked", "nuked_population_sign_think"),
        ("maps/mp/zm_nuked", "bus_taser_blocker"),
        ("maps/mp/zm_nuked", "bus_random_horn"),
        ("maps/mp/zm_nuked", "nuked_update_traversals"),
        ("maps/mp/zm_nuked", "fake_lighting_cleanup"),
        // Its story: the bunker, the moon transmissions, the eye change.
        ("maps/mp/zm_nuked", "marlton_vo_inside_bunker"),
        ("maps/mp/zm_nuked", "moon_transmission_vo"),
        ("maps/mp/zm_nuked", "zombie_eye_glow_change"),
        ("maps/mp/zm_nuked", "switch_announcer_to_richtofen"),
        ("maps/mp/zm_nuked", "sndmusiceastereggs"),
    ];
    let mut missing = Vec::new();
    for (script, name) in quiet {
        if !vm.hook_function(script, name, nothing) {
            missing.push(*name);
        }
    }
    let hooks: &[(&str, &str, gsc_t6::FnHook<World>)] = &[
        ("maps/mp/zm_nuked_perks", "perks_from_the_sky", perks_in_the_room),
        ("maps/mp/zombies/_zm", "init", before_zm_init),
        ("maps/mp/zombies/_zm", "round_over", round_over),
        ("maps/mp/zombies/_zm_zonemgr", "create_spawner_list", spawner_list),
        ("maps/mp/zm_nuked", "include_powerups", include_powerups),
        ("maps/mp/zombies/_zm_powerups", "func_should_drop_carpenter", yes),
        ("maps/mp/zombies/_zm_powerups", "start_carpenter", carpenter),
        ("maps/mp/zombies/_zm_powerups", "start_carpenter_new", carpenter),
        // Playtest 1: no health coming back by itself (food heals, on the
        // world side).
        ("maps/mp/zombies/_zm_playerhealth", "playerhealthregen", nothing),
        // Playtest 1b: every gun is an item in its own slot; a wall buy or
        // a box gun never replaces the one in his hands.
        ("maps/mp/zombies/_zm_utility", "get_player_weapon_limit", gun_limit),
    ];
    for (script, name, hook) in hooks {
        if !vm.hook_function(script, name, *hook) {
            missing.push(*name);
        }
    }
    diag::info!(
        Sim,
        "bo2mc rules: {} script hooks{}",
        quiet.len() + hooks.len() - missing.len(),
        if missing.is_empty() {
            String::new()
        } else {
            format!(", missing {}", missing.join(" "))
        }
    );
}

fn nothing(_: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    HookAction::Return(Value::Undefined)
}

fn yes(_: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    HookAction::Return(Value::Int(1))
}

fn gun_limit(_: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    HookAction::Return(Value::Int(GUN_LIMIT))
}

/// Nuketown lifts its machines into the sky and drops them over the
/// rounds: here they stand in the room from the start, so only its power
/// switch (`turn_perks_on`) runs. The machines are turned on in `tick`.
fn perks_in_the_room(vm: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    match vm.find_function("maps/mp/zm_nuked_perks", "turn_perks_on") {
        Some(f) => HookAction::Redirect(f, Vec::new()),
        None => HookAction::Return(Value::Undefined),
    }
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

/// Right before `_zm::init` spawns the machines (`perk_machine_spawn_init`
/// at each struct Nuketown picked at random): those structs move to the
/// room's spots, Quick Revive, Juggernog, Speed Cola and Double Tap on the
/// short walls, Pack-a-Punch on the south wall.
fn before_zm_init(vm: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let level = Value::Object(vm.level);
    let names = field(vm, &level, "struct_class_names");
    let by_tn = field(vm, &names, "targetname");
    let structs = field(vm, &by_tn, "zm_perk_machine_override");
    let Value::Array(arr) = structs else {
        diag::warn!(Sim, "bo2mc perks: no zm_perk_machine_override structs");
        return HookAction::Continue;
    };
    let spots = bo2mc::perk_spots();
    let mut placed = Vec::new();
    let mut spare = [0usize, 5, 6, 11, 2, 8].into_iter();
    for v in arr.snapshot().values_in_order() {
        let Value::Object(o) = v else { continue };
        let perk = {
            let f = vm.intern("script_noteworthy");
            let t = vm.raw_field(*o, f);
            vm.to_text(&t)
        };
        let (pos, face) = match perk.as_str() {
            "specialty_weapupgrade" => bo2mc::pap_spot(),
            "specialty_quickrevive" => spots[1],
            "specialty_armorvest" => spots[4],
            "specialty_fastreload" => spots[7],
            "specialty_rof" => spots[10],
            _ => match spare.next() {
                Some(i) => spots[i],
                None => continue,
            },
        };
        // A machine's front is 90 degrees right of its angles.
        set_field(vm, *o, "origin", Value::Vec3(pos));
        set_field(vm, *o, "angles", Value::Vec3([0.0, face + 90.0, 0.0]));
        set_field(vm, *o, "blocker_model", Value::Undefined);
        placed.push(perk);
    }
    diag::info!(Sim, "bo2mc perks: in the room: {}", placed.join(" "));
    HookAction::Continue
}

/// A round is over (BO2's `round_over`, the ten seconds before the next
/// round): the next waits for nightfall, and dawn comes now.
fn round_over(vm: &mut Vm<World>, world: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let Some(wait) = vm.find_function("common_scripts/utility", "flag_wait") else {
        return HookAction::Continue;
    };
    let level = Value::Object(vm.level);
    let name = vm.string(NIGHT);
    vm.spawn_named(world, "common_scripts/utility", "flag_clear", level, vec![name.clone()]);
    let now = world.resource::<Zm>().now_ms;
    let round = {
        let f = vm.intern("round_number");
        vm.raw_field(vm.level, f).as_int().unwrap_or(0)
    };
    if let Some(mut r) = world.get_resource_mut::<McRules>() {
        r.holding = true;
        r.holding_since = now;
        r.morning_seen = false;
        r.paused = false;
    }
    if clock_live() {
        push_clock(world, Request::PauseDay(false));
        push_clock(world, Request::SetDay(DAWN));
    }
    diag::info!(
        Sim,
        "bo2mc round {} over at {}s (clock {:.0}): dawn; round {round} waits for nightfall",
        round - 1,
        now / 1000,
        bo2mc::day_ticks()
    );
    HookAction::Redirect(wait, vec![name])
}

/// BO2 lists where zombies may spawn each second (`create_spawner_list`):
/// once the world side names spots, those are the list.
fn spawner_list(vm: &mut Vm<World>, world: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let Some(r) = world.get_resource::<McRules>() else {
        return HookAction::Continue;
    };
    if r.pool_live == 0 {
        return HookAction::Continue;
    }
    let pool: Vec<ObjRef> = r.pool[..r.pool_live].to_vec();
    set_spawn_list(vm, &pool);
    HookAction::Return(Value::Undefined)
}

fn set_spawn_list(vm: &mut Vm<World>, pool: &[ObjRef]) {
    let mut a = Array::new();
    for o in pool {
        a.push(Value::Object(*o));
    }
    let f = vm.intern("zombie_spawn_locations");
    vm.set_raw_field(vm.level, f, Value::array(a));
}

/// Nuketown's power-ups plus the Carpenter (it repairs nothing here: it
/// gives wood).
fn include_powerups(vm: &mut Vm<World>, world: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let level = Value::Object(vm.level);
    let name = vm.string("carpenter");
    vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "include_powerup", level, vec![name]);
    HookAction::Continue
}

/// The Carpenter: 32 oak planks and 2 oak doors into his inventory (the
/// pick-up sound and the announcer are BO2's, before this).
fn carpenter(_: &mut Vm<World>, _: &mut World, _: &Value, _: &[Value]) -> HookAction {
    push(Request::Give {
        item: "minecraft:oak_planks".into(),
        count: 32,
    });
    push(Request::Give {
        item: "minecraft:oak_door".into(),
        count: 2,
    });
    diag::info!(Sim, "bo2mc carpenter: 32 oak planks and 2 oak doors");
    HookAction::Return(Value::Undefined)
}

// ------------------------------------------------------------ playtest 1

/// Nuketown opens its houses' zones with doors that are not here: every
/// zone on, once BO2's zone manager is up (the box's and the wall buys'
/// use triggers live in zones, and a zone that is off never lists them).
fn enable_zones(world: &mut World) {
    if world.resource::<McRules>().zones_on {
        return;
    }
    let done = with_vm(world, |vm, world| {
        if !level_flag(vm, "zones_initialized") {
            return false;
        }
        let level = Value::Object(vm.level);
        let Value::Array(zones) = field(vm, &level, "zones") else {
            return false;
        };
        let keys: Vec<Value> = zones.snapshot().keys().map(Key::value).collect();
        for k in &keys {
            vm.spawn_named(world, "maps/mp/zombies/_zm_zonemgr", "enable_zone", level.clone(), vec![k.clone()]);
        }
        diag::info!(Sim, "bo2mc zones: all {} on", keys.len());
        true
    })
    .unwrap_or(false);
    if done {
        world.resource_mut::<McRules>().zones_on = true;
    }
}

/// A melee weapon's name (the knife and its upgrades).
fn is_melee(name: &str) -> bool {
    ["knife", "bowie", "tazer", "sickle"].iter().any(|k| name.contains(k))
}

/// What the world side asks: heal from food, the armor he wears, the item
/// he picked.
fn asks(world: &mut World) {
    for a in bo2mc::take_asks() {
        match a {
            Ask::Heal(n) => {
                let down = world.resource::<Zm>().players.get(&0).is_some_and(|p| p.laststand);
                let mut f = frame(world);
                if let Some(ps) = f.player_mut(ClientId(0))
                    && ps.health > 0
                    && !down
                {
                    ps.health = (ps.health + n.max(0)).min(ps.max_health.max(1));
                }
            }
            Ask::Armor { points, toughness } => {
                world.resource_mut::<McRules>().armor = (points.max(0.0), toughness.max(0.0));
            }
            Ask::SelectBlock => world.resource_mut::<McRules>().knife = false,
            Ask::Select(name) => {
                let knife = is_melee(&name);
                world.resource_mut::<McRules>().knife = knife;
                // A grenade or monkey item stays in the pocket: the gun
                // stays up and his click throws it (BO2's offhand). A mine
                // comes up in his hands, as BO2 places claymores.
                let offhand = bo2mc::player_weapons()
                    .iter()
                    .any(|i| i.name == name && matches!(i.kind, "lethal" | "tactical"));
                if !knife
                    && !offhand
                    && let Ok(w) = super::weapon(world, &name)
                    && w != 0
                {
                    crate::script_player::switch_to_weapon(&mut frame(world), ClientId(0), w);
                }
            }
        }
    }
}

/// No knife on V (playtest 1): the knife is an item, and with it picked his
/// attack is the knife swing.
fn buttons(world: &mut World) {
    if super::autoplay::enabled() {
        return;
    }
    let knife = world.resource::<McRules>().knife;
    let melee = weapon_iw4::BUTTON_MELEE;
    let attack = playerstate_iw4::buttons::ATTACK;
    let mut req = world.resource_mut::<crate::step::StepRequest>();
    for (_, cmd) in &mut req.input.cmds {
        if knife {
            if cmd.buttons & attack != 0 {
                cmd.buttons = (cmd.buttons & !attack) | melee;
            }
        } else {
            cmd.buttons &= !melee;
        }
    }
}

/// His Minecraft armor on a hit BO2 has decided (`finishplayerdamage`);
/// a fall goes through it whole, as in Minecraft.
pub(super) fn armor_cut(world: &World, amount: i32, means: &str) -> i32 {
    if !bo2mc::enabled() || means == "MOD_FALLING" {
        return amount;
    }
    let (points, toughness) = world.get_resource::<McRules>().map_or((0.0, 0.0), |r| r.armor);
    let cut = bo2mc::armor_cut(amount, points, toughness);
    if cut != amount {
        diag::info!(Sim, "bo2mc armor: {means} {amount} -> {cut} ({points} armor points)");
    }
    cut
}

/// BO2's own short-lived weapons that are no items (drinking a perk, the
/// knuckle crack, the last stand pistol's helpers).
fn not_an_item(name: &str) -> bool {
    ["perk_bottle", "knuckle_crack", "syrette", "death_throe", "fists", "chalk_draw", "flourish", "none"]
        .iter()
        .any(|k| name.contains(k))
}

/// His health and BO2 weapons for the world side (hearts, items).
fn publish_player(world: &mut World) {
    let client = ClientId(0);
    let Some(ps) = frame(world).player(client).copied() else {
        return;
    };
    let list = crate::script_player::weapons(&frame(world), client, crate::script_player::WeaponList::All);
    let knife = world.resource::<McRules>().knife;
    let mut items = Vec::new();
    for w in list {
        let name = super::weapon_text(world, w);
        if not_an_item(&name) {
            continue;
        }
        let f = frame(world);
        let class = f.equipment_facts_for(w).map_or(0, |eq| eq.offhand_class);
        let kind = if is_melee(&name) {
            "melee"
        } else if name.contains("claymore") {
            "mine"
        } else if class == 1 {
            "lethal"
        } else if class >= 2 {
            "tactical"
        } else {
            "gun"
        };
        items.push(WeaponItem {
            index: w,
            clip: crate::script_player::ammo_clip(&f, client, w),
            stock: crate::script_player::ammo_stock(&f, client, w),
            upgraded: name.contains("_upgraded"),
            held: if kind == "melee" { knife } else { ps.weapon == w && !knife },
            name,
            kind,
        });
        drop(f);
        // A grenade's count is its clip, as BO2's scripts keep it (2 to
        // start, more each round up to 4): no spare stock on top.
        if matches!(kind, "lethal" | "tactical" | "mine")
            && crate::script_player::ammo_stock(&frame(world), client, w) > 0
        {
            crate::script_player::set_ammo_stock(&mut frame(world), client, w, 0);
            if let Some(last) = items.last_mut() {
                last.stock = 0;
            }
        }
    }
    // The knife the scripts gave him (BO2 keeps it off his weapon list).
    if let Some(k) = world.resource::<Zm>().players.get(&0).and_then(|p| p.melee_weapon.clone())
        && !items.iter().any(|i| i.name == k)
    {
        let index = super::weapon(world, &k).unwrap_or(0);
        items.push(WeaponItem {
            name: k,
            index,
            kind: "melee",
            held: knife,
            ..WeaponItem::default()
        });
    }
    let shown: Vec<String> = items.iter().map(|i| format!("{}#{}:{}", i.name, i.index, i.kind)).collect();
    let shown = shown.join(" ");
    if world.resource::<McRules>().items_logged != shown {
        diag::info!(Sim, "bo2mc items: {shown}");
        world.resource_mut::<McRules>().items_logged = shown;
    }
    let mut ps = ps;
    let before = world.resource::<McRules>().max_seen;
    if before > 0 && ps.max_health > before && ps.health > 0 {
        let gain = ps.max_health - before;
        if let Some(p) = frame(world).player_mut(client) {
            p.health = (p.health + gain).min(p.max_health);
            ps.health = p.health;
        }
        diag::info!(Sim, "bo2mc: most health {before} -> {}: {gain} golden", ps.max_health);
    }
    world.resource_mut::<McRules>().max_seen = ps.max_health;
    bo2mc::set_player(ps.health, ps.max_health, items);
}

/// The ammo station (playtest 1): against the east wall between the perk
/// machines, "Hold [use] to buy ammo": the gun in his hands filled, 1000
/// points (4500 Pack-a-Punched).
const AMMO_COST: i32 = 1000;
const AMMO_COST_UPGRADED: i32 = 4500;

fn ammo_hint(cost: i32) -> String {
    format!("Hold ^3[{{+activate}}]^7 to buy ammo [Cost: {cost}]")
}

/// The inventory's character window (playtest 1): his BO2 body (the model
/// the scripts gave him) stands where the world side says while the
/// inventory is open, playing a standing idle, and hides when it shuts.
fn inventory_puppet(world: &mut World) {
    // IW4L_BO2MC_PUPPET_TEST=1 (tests): the body stands in the room, three
    // cells north of the middle, facing south.
    let spot = if std::env::var("IW4L_BO2MC_PUPPET_TEST").is_ok_and(|v| v == "1") {
        Some(bo2mc::Puppet { origin: bo2mc::cell_floor(0, -3), yaw: -90.0 })
    } else {
        bo2mc::puppet()
    };
    let body = world.resource::<Zm>().players.get(&0).and_then(|p| p.body_model.clone());
    let n = match (world.resource::<McRules>().puppet, spot, body) {
        (Some(n), ..) => n,
        (None, Some(_), Some(model)) => {
            let n = with_vm(world, |vm, world| {
                let n = world.resource_mut::<Zm>().alloc_entnum();
                let obj = vm.alloc_object(ObjKind::Entity(n));
                world.resource_mut::<Zm>().ents.insert(
                    n,
                    super::Ent {
                        obj: Some(obj),
                        classname: "script_model".into(),
                        model: model.clone(),
                        hidden: true,
                        ..Default::default()
                    },
                );
                n
            });
            let Some(n) = n else { return };
            let anim = puppet_idle(world);
            diag::info!(Sim, "bo2mc inventory body: {model} as ent {n}, idle {anim:?}");
            let mut r = world.resource_mut::<McRules>();
            r.puppet = Some(n);
            r.puppet_anim = anim;
            n
        }
        _ => return,
    };
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
        match spot {
            Some(p) => {
                e.origin = p.origin;
                e.angles = [0.0, p.yaw, 0.0];
                e.hidden = false;
            }
            None => e.hidden = true,
        }
    }
    let number = world.resource::<Zm>().presences.by_ent.get(&n).and_then(|s| u32::try_from(s.number).ok());
    if spot.is_some() && number != bo2mc::puppet_number() {
        diag::info!(Sim, "bo2mc inventory body: ent {n} goes out as {number:?} at {:?}", spot.map(|s| (s.origin, s.yaw)));
    }
    bo2mc::set_puppet_number(number.filter(|_| spot.is_some()));
}

/// A standing idle for his body: BO2's own player stand anims if the zones
/// hold one, else any standing idle.
fn puppet_idle(world: &World) -> Option<String> {
    let zm = world.resource::<Zm>();
    const WANT: [&str; 6] = ["pb_stand_alert", "pb_stand_alert_pistol", "pb_stand_alert_rifle", "pt_stand_core_pistol", "pb_afterlife_idle", "pb_stand_idle"];
    if let Some(a) = WANT.iter().find(|a| zm.anims.contains_key(**a)) {
        return Some((*a).to_owned());
    }
    let mut names: Vec<&String> = zm.anims.keys().filter(|k| k.starts_with("pb_") || k.starts_with("pt_")).collect();
    names.sort();
    diag::info!(Sim, "bo2mc inventory body: player anims in the zones: {}", names.iter().take(40).map(|s| s.as_str()).collect::<Vec<_>>().join(" "));
    names.iter().find(|k| k.contains("stand") && !k.contains("2")).or(names.first()).map(|s| (*s).clone())
}

/// The inventory body's idle (looping), for its model row.
pub(super) fn puppet_anim(world: &World, n: u32, now: i64) -> Option<(String, f32, f32)> {
    let r = world.get_resource::<McRules>()?;
    if r.puppet != Some(n) {
        return None;
    }
    let name = r.puppet_anim.clone()?;
    let a = world.resource::<Zm>().anims.get(&name)?.clone();
    let length_ms = ((f32::from(a.numframes.max(1)) / a.framerate.max(1.0)) * 1000.0).max(1.0) as i64;
    Some((name, (now % length_ms) as f32 / length_ms as f32, 1000.0 / length_ms as f32))
}

fn ammo_station(world: &mut World, now: i64) {
    let client = ClientId(0);
    // Put it up once.
    let station = match world.resource::<McRules>().station {
        Some(n) => n,
        None => {
            let spot = bo2mc::cell_floor(bo2mc::ROOM_HALF_X, 1);
            let n = with_vm(world, |vm, world| {
                let (t, m) = {
                    let mut zm = world.resource_mut::<Zm>();
                    (zm.alloc_entnum(), zm.alloc_entnum())
                };
                let tobj = vm.alloc_object(ObjKind::Entity(t));
                let mobj = vm.alloc_object(ObjKind::Entity(m));
                let mut zm = world.resource_mut::<Zm>();
                zm.ents.insert(
                    t,
                    super::Ent {
                        obj: Some(tobj),
                        classname: "trigger_radius_use".into(),
                        origin: spot,
                        radius: 44.0,
                        height: 72.0,
                        hint: Some(ammo_hint(AMMO_COST)),
                        ..Default::default()
                    },
                );
                // The max ammo can, on the floor against the wall.
                zm.ents.insert(
                    m,
                    super::Ent {
                        obj: Some(mobj),
                        classname: "script_model".into(),
                        model: "zombie_ammocan".into(),
                        origin: [spot[0] + 6.0, spot[1], spot[2] + 2.0],
                        angles: [0.0, 180.0, 0.0],
                        ..Default::default()
                    },
                );
                t
            });
            let Some(n) = n else { return };
            world.resource_mut::<McRules>().station = Some(n);
            diag::info!(Sim, "bo2mc ammo station: ent {n} on the east wall");
            n
        }
    };
    let Some(ps) = frame(world).player(client).copied() else {
        return;
    };
    let w = ps.weapon;
    let name = super::weapon_text(world, w);
    let cost = if name.contains("_upgraded") { AMMO_COST_UPGRADED } else { AMMO_COST };
    let hint = ammo_hint(cost);
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&station)
        && e.hint.as_deref() != Some(hint.as_str())
    {
        e.hint = Some(hint.clone());
    }
    // A fresh press of use while it is the trigger he faces.
    let held = crate::script_player::buttons(&mut frame(world), client)
        & (playerstate_iw4::buttons::USE | playerstate_iw4::buttons::USE_RELOAD)
        != 0;
    let was = std::mem::replace(&mut world.resource_mut::<McRules>().use_was, held);
    let facing = world.resource::<Zm>().hints.get(&0).is_some_and(|h| *h == hint);
    if !held || was || !facing {
        return;
    }
    let gun = w != 0 && !is_melee(&name) && !not_an_item(&name);
    let full = {
        let f = frame(world);
        f.combat_facts_for(w).is_none_or(|facts| {
            crate::script_player::ammo_clip(&f, client, w) >= facts.clip_size
                && crate::script_player::ammo_stock(&f, client, w) >= facts.max_ammo
        })
    };
    let score = frame(world).client_meta(client).map_or(0, |m| m.score);
    let ok = gun && !full && score >= cost;
    let pobj = world.resource::<Zm>().players.get(&0).map(|p| p.obj);
    with_vm(world, |vm, world| {
        let Some(p) = pobj else { return };
        let me = Value::Object(p);
        if ok {
            vm.spawn_named(world, "maps/mp/zombies/_zm_score", "minus_to_player_score", me.clone(), vec![Value::Int(cost)]);
        }
        let sound = vm.string(if ok { "purchase" } else { "no_purchase" });
        vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "play_sound_on_ent", me, vec![sound]);
    });
    if ok {
        let mut f = frame(world);
        if let Some(facts) = f.combat_facts_for(w) {
            crate::script_player::set_ammo_clip(&mut f, client, w, facts.clip_size);
            crate::script_player::set_ammo_stock(&mut f, client, w, facts.max_ammo);
        }
    }
    let what = if ok {
        "filled"
    } else if !gun {
        "refused: no gun in hand"
    } else if full {
        "refused: full"
    } else {
        "refused: points"
    };
    diag::info!(Sim, "bo2mc ammo station at {}s: {name} {what} ({score} points, cost {cost})", now / 1000);
}

/// The level started (map entities, the game type's and the map's main):
/// the night flag stands set (the game starts at nightfall).
pub(super) fn level_started(vm: &mut Vm<World>, world: &mut World) {
    let level = Value::Object(vm.level);
    let name = vm.string(NIGHT);
    vm.spawn_named(
        world,
        "common_scripts/utility",
        "flag_init",
        level,
        vec![name, Value::Int(1)],
    );
}

// ------------------------------------------------------------ every tick

pub(super) fn tick(world: &mut World, now: i64) {
    if !bo2mc::enabled() || !world.resource::<Zm>().started {
        return;
    }
    if !world.contains_resource::<McRules>() {
        world.insert_resource(McRules::default());
    }
    test_clock(world);
    enable_zones(world);
    test_points(world, now);
    asks(world);
    buttons(world);
    ammo_station(world, now);
    inventory_puppet(world);
    publish_player(world);
    spawn_spots(world, now);
    turn_perks_on(world, now);
    night(world, now);
    world_damage(world, now);
    rise_again_closer(world, now);
}

/// Test aid: IW4L_BO2MC_TEST_POINTS=N adds N points once, 5 s in (to buy
/// several guns in a hidden run).
fn test_points(world: &mut World, now: i64) {
    static N: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    let n = *N.get_or_init(|| std::env::var("IW4L_BO2MC_TEST_POINTS").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
    if n <= 0 || now < 5000 || world.resource::<McRules>().points_given {
        return;
    }
    let Some(obj) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) else {
        return;
    };
    world.resource_mut::<McRules>().points_given = true;
    with_vm(world, |vm, world| {
        vm.spawn_named(world, "maps/mp/zombies/_zm_score", "add_to_player_score", Value::Object(obj), vec![Value::Int(n)]);
    });
    diag::info!(Sim, "bo2mc test: {n} points");
}

/// The rules side's own day clock while the world side publishes none
/// (IW4L_BO2MC_FAKE_DAY=1 on plain Nuketown, or a world side without its
/// clock yet): 20 ticks a second times IW4L_BO2MC_DAY_SPEED, starting at
/// nightfall. Once someone else moves the clock it is theirs.
fn test_clock(world: &mut World) {
    if !(world_side() || fake_day()) {
        return;
    }
    // Requests nobody carries out are dropped, not hoarded.
    if bo2mc::pending() > 20_000 {
        let dropped = bo2mc::take_requests().len();
        diag::warn!(Sim, "bo2mc: {dropped} requests nobody carried out dropped");
    }
    let mut r = world.resource_mut::<McRules>();
    if r.world_clock {
        return;
    }
    let now = bo2mc::day_ticks();
    let external = match r.fake {
        Some((mine, _)) => (now - mine).abs() > 0.5,
        None => now != 0.0,
    };
    if external && !fake_day() {
        r.world_clock = true;
        r.fake = None;
        diag::info!(Sim, "bo2mc clock: the world side runs the day (at {now:.0})");
        return;
    }
    let (mut t, paused) = r.fake.unwrap_or((NIGHTFALL, false));
    if !paused {
        t = (t + day_speed()) % 24000.0;
    }
    r.fake = Some((t, paused));
    bo2mc::set_day(t, paused);
}

/// Spawn spots: the world side's candidates as BO2 riser structs, the list
/// BO2 picks from.
fn spawn_spots(world: &mut World, now: i64) {
    if now - world.resource::<McRules>().pool_ms < 250 {
        return;
    }
    world.resource_mut::<McRules>().pool_ms = now;
    let player = frame(world).player(ClientId(0)).map(|ps| ps.origin);
    let mut points = bo2mc::spawn_candidates();
    if points.is_empty()
        && super::mc_nav::active()
        && let Some(p) = player
    {
        points = own_candidates(p);
    }
    if points.is_empty() {
        return;
    }
    let mut pool = world.resource::<McRules>().pool.clone();
    let k = points.len().min(POOL);
    with_vm(world, |vm, _| {
        while pool.len() < k {
            let o = vm.alloc_object(ObjKind::Struct);
            for (name, v) in [
                ("script_noteworthy", "riser_location"),
                ("script_string", "find_flesh"),
                ("targetname", "bo2mc_spawners"),
                ("zone_name", "culdesac_yellow_zone"),
            ] {
                let s = vm.string(v);
                set_field(vm, o, name, s);
            }
            set_field(vm, o, "is_enabled", Value::Int(1));
            pool.push(o);
        }
        for (o, p) in pool.iter().zip(points.iter()) {
            let yaw = player.map_or(0.0, |pl| (pl[1] - p[1]).atan2(pl[0] - p[0]).to_degrees());
            set_field(vm, *o, "origin", Value::Vec3(*p));
            set_field(vm, *o, "angles", Value::Vec3([0.0, yaw, 0.0]));
        }
        set_spawn_list(vm, &pool[..k]);
    });
    let mut r = world.resource_mut::<McRules>();
    if r.pool_live == 0 {
        diag::info!(Sim, "bo2mc spawns: {k} spots");
    }
    r.pool = pool;
    r.pool_live = k;
}

/// Spawn spots while the world side names none: the ground's top 12-28
/// blocks around the player (two clear blocks over a solid one, not by the
/// room); the cells round him when he is underground.
fn own_candidates(player: [f32; 3]) -> Vec<[f32; 3]> {
    use super::mc_path::{Blocks, Kind, stands};
    let Some(pb) = bo2mc::map_to_block([player[0], player[1], player[2] + 1.0]) else {
        return Vec::new();
    };
    let Some(room) = bo2mc::room_cell_block([0, 0, 0]) else {
        return Vec::new();
    };
    let by_room = |b: [i32; 3]| {
        (b[0] - room[0]).abs() <= bo2mc::ROOM_HALF_X + 4 && (b[2] - room[2]).abs() <= bo2mc::ROOM_HALF_Z + 4
    };
    let under = bo2mc::player_underground();
    super::mc_nav::with_live(|live| {
        let mut out = Vec::new();
        if under {
            for (dx, dz) in [(2, 0), (-2, 0), (0, 2), (0, -2), (3, 1), (-3, -1), (1, -3), (-1, 3), (4, 0), (0, -4)] {
                for dy in [0, 1, -1] {
                    let c = [pb[0] + dx, pb[1] + dy, pb[2] + dz];
                    if stands(live, c) {
                        out.push(c);
                        break;
                    }
                }
            }
        } else {
            for k in 0..40 {
                let a = k as f32 * 2.399_963;
                let r = 12.0 + ((k * 7) % 17) as f32;
                let x = pb[0] + (r * a.cos()).round() as i32;
                let z = pb[2] + (r * a.sin()).round() as i32;
                for y in (pb[1] - 16..=pb[1] + 16).rev() {
                    let c = [x, y, z];
                    if live.kind(c) != Kind::Air {
                        // The ground's top: standing on it or not, stop.
                        if stands(live, [x, y + 1, z]) && !by_room(c) {
                            out.push([x, y + 1, z]);
                        }
                        break;
                    }
                }
                if out.len() >= 24 {
                    break;
                }
            }
        }
        out.into_iter().filter_map(bo2mc::block_to_map).collect::<Vec<_>>()
    })
    .unwrap_or_default()
}

fn level_flag(vm: &mut Vm<World>, name: &str) -> bool {
    let level = Value::Object(vm.level);
    let flags = field(vm, &level, "flag");
    gsc_t6::truthy(&field(vm, &flags, name))
}

/// The machines stand in the room: two seconds after BO2's opening black
/// screen they turn on, as Nuketown's turned on when they landed.
fn turn_perks_on(world: &mut World, now: i64) {
    match world.resource::<McRules>().perks_on {
        Some(at) if at < 0 => return,
        Some(at) if now >= at => {
            with_vm(world, |vm, world| {
                let level = vm.level;
                let f = vm.intern("revive_machine_spawned");
                vm.set_raw_field(level, f, Value::Int(1));
                for name in ["revive_on", "sleight_on", "doubletap_on", "juggernog_on", "Pack_A_Punch_on"] {
                    vm.notify_str(world, level, name, &[]);
                }
                // The machines themselves hear it too (as `bring_perk`).
                let tn = vm.intern("turn_on_notify");
                let machines: Vec<(ObjRef, String)> = world
                    .resource::<Zm>()
                    .ents
                    .values()
                    .filter_map(|e| e.obj)
                    .filter_map(|o| {
                        let v = vm.raw_field(o, tn);
                        (!v.is_undefined()).then(|| (o, vm.to_text(&v)))
                    })
                    .collect();
                for (o, name) in machines {
                    vm.notify_str(world, o, &name, &[]);
                }
            });
            diag::info!(Sim, "bo2mc perks: machines turned on at {}s", now / 1000);
            world.resource_mut::<McRules>().perks_on = Some(-1);
        }
        Some(_) => {}
        None => {
            let ready = with_vm(world, |vm, _| level_flag(vm, "initial_blackscreen_passed")).unwrap_or(false);
            if ready {
                world.resource_mut::<McRules>().perks_on = Some(now + 2000);
            }
        }
    }
}

/// Night = round: a finished round waits for nightfall; a round still on
/// at midnight holds the clock; sunset warns a player underground.
fn night(world: &mut World, now: i64) {
    let live = clock_live();
    let t = bo2mc::day_ticks();
    let (holding, since, morning, paused) = {
        let r = world.resource::<McRules>();
        (r.holding, r.holding_since, r.morning_seen, r.paused)
    };
    let round = with_vm(world, |vm, _| {
        let f = vm.intern("round_number");
        vm.raw_field(vm.level, f).as_int().unwrap_or(0)
    })
    .unwrap_or(0);
    if holding && live && bo2mc::day_paused() && now - world.resource::<McRules>().unpause_ms >= 1000 {
        world.resource_mut::<McRules>().unpause_ms = now;
        push_clock(world, Request::PauseDay(false));
    }
    if holding {
        let fall = if live {
            // The morning first (the clock runs on from dawn), then dusk.
            let morning = morning || t < SUNSET_WARN;
            if morning {
                world.resource_mut::<McRules>().morning_seen = true;
            }
            morning && (NIGHTFALL..DAWN).contains(&t)
        } else {
            // No clock: BO2's own ten seconds between rounds.
            now - since >= 10_000
        };
        if fall {
            with_vm(world, |vm, world| {
                let level = Value::Object(vm.level);
                let name = vm.string(NIGHT);
                vm.spawn_named(world, "common_scripts/utility", "flag_set", level, vec![name]);
            });
            world.resource_mut::<McRules>().holding = false;
            diag::info!(
                Sim,
                "bo2mc night falls at {}s (clock {t:.0}): round {round} starts",
                now / 1000
            );
        }
    } else if live && (MIDNIGHT..DAWN).contains(&t) && !bo2mc::day_paused() && now - since >= 1000 {
        // Asked again (once a second) until the clock says it waits: a
        // world reloading under a restarted game drops a request.
        push_clock(world, Request::PauseDay(true));
        let mut r = world.resource_mut::<McRules>();
        r.holding_since = now;
        if !paused {
            r.paused = true;
            diag::info!(Sim, "bo2mc midnight at {}s: the sun waits for round {round}", now / 1000);
        }
    }
    if live
        && (SUNSET_WARN..NIGHTFALL).contains(&t)
        && bo2mc::player_underground()
        && now - world.resource::<McRules>().warned_ms >= 5000
    {
        push(Request::Notice {
            text: "Get above ground! Night is coming.".into(),
            seconds: 4.0,
        });
        world.resource_mut::<McRules>().warned_ms = now;
    }
    let round = shown_round(world, round);
    if round != world.resource::<McRules>().rounds_logged {
        world.resource_mut::<McRules>().rounds_logged = round;
        diag::info!(Sim, "bo2mc round {round} at {}s (clock {t:.0})", now / 1000);
    }
}

/// Minecraft's own damage the world side measures (falls as Minecraft
/// counts them, drowning, lava, fire, starving) goes through BO2's player
/// damage, so last stand and game over follow as for any hit. Fire and lava
/// are cut by armor as in Minecraft; falls, drowning and starving are not.
fn world_damage(world: &mut World, now: i64) {
    let events = bo2mc::take_player_events();
    // Not while the world loads in under him at the start.
    if events.is_empty() || now < 10_000 {
        return;
    }
    let c = 0u32;
    let Some(ps) = frame(world).player(ClientId(c)).copied() else {
        return;
    };
    let down = world.resource::<Zm>().players.get(&c).is_some_and(|p| p.laststand || p.linked.is_some());
    if ps.pm_type != 0 || down || ps.health <= 0 {
        return;
    }
    for bo2mc::PlayerEvent::Damage { amount, cause } in events {
        if amount <= 0 {
            continue;
        }
        let means = match cause {
            "lava" | "in_fire" | "on_fire" => "MOD_BURNED",
            _ => "MOD_FALLING",
        };
        diag::info!(Sim, "bo2mc {cause}: {amount} damage (health {})", ps.health);
        with_vm(world, |vm, world| {
            super::natives_ai::player_damage(
                vm,
                world,
                c,
                Value::Undefined,
                Value::Undefined,
                amount,
                0,
                means,
                "none",
                ps.origin,
                [0.0, 0.0, -1.0],
                "none",
            );
        });
    }
}

/// R4: a zombie far behind the player, stuck clawing, with no way to him or
/// not getting closer leaves (no points, no drop) and rises again at a spot
/// near him, as BO2's own spawn failsafe puts timed-out zombies back in the
/// round's queue.
fn rise_again_closer(world: &mut World, now: i64) {
    if !super::mc_nav::active() {
        return;
    }
    // Only players in the game (not the game-over camera, not down).
    let players: Vec<[f32; 3]> = {
        let ids: Vec<u32> = world
            .resource::<Zm>()
            .players
            .iter()
            .filter(|(_, p)| p.sessionstate == "playing" && !p.laststand && p.camera.is_none())
            .map(|(c, _)| *c)
            .collect();
        let f = frame(world);
        ids.iter()
            .filter_map(|&c| f.player(ClientId(c)).map(|p| p.origin))
            .collect()
    };
    if players.is_empty() {
        return;
    }
    let living: Vec<(u32, ObjRef, [f32; 3], i32)> = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .filter(|(_, a)| a.alive && a.scripted.is_none() && a.traverse.is_none())
            .filter_map(|(n, a)| Some((*n, a.obj?, zm.ents.get(n)?.origin, a.health)))
            .collect()
    };
    let mut gone = Vec::new();
    with_vm(world, |vm, world| {
        let emerged = vm.intern("completed_emerging_into_playable_area");
        for (n, obj, at, health) in living {
            if !gsc_t6::truthy(&vm.raw_field(obj, emerged)) {
                continue;
            }
            let d = players
                .iter()
                .map(|p| ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            let reason = {
                let mut zm = world.resource_mut::<Zm>();
                let st = zm.mc.actors.entry(n).or_default();
                let best = match st.best {
                    _ if st.clawing => (d, now),
                    Some((b, t)) if d >= b - 72.0 => (b, t),
                    _ => (d, now),
                };
                st.best = Some(best);
                if d > 40.0 * 36.0 {
                    Some("far behind")
                } else if st.clawing && st.claw.is_some_and(|(_, since)| now - since > 10_000) {
                    Some("clawed one spot too long")
                } else if st.no_path_since.is_some_and(|since| now - since > 5_000) {
                    Some("no way to him")
                } else if !st.clawing && d > 3.0 * 36.0 && now - best.1 > 12_000 {
                    Some("not getting closer")
                } else {
                    None
                }
            };
            let Some(reason) = reason else { continue };
            if gone.len() >= 2 {
                break;
            }
            // Back in the round's queue, as the failsafe does it.
            for (name, add) in [("zombie_total", 1), ("zombie_total_subtract", 1)] {
                let f = vm.intern(name);
                let v = vm.raw_field(vm.level, f).as_int().unwrap_or(0);
                vm.set_raw_field(vm.level, f, Value::Int(v + add));
            }
            set_field(vm, obj, "no_powerups", Value::Int(1));
            super::actors::damage(
                vm,
                world,
                n,
                Value::Object(obj),
                Value::Object(obj),
                health + 100,
                0,
                "MOD_UNKNOWN",
                "none",
                at,
                [0.0; 3],
                "none",
            );
            gone.push((n, reason, d));
        }
    });
    for (n, reason, d) in gone {
        {
            let mut zm = world.resource_mut::<Zm>();
            // Gone at once, not a corpse.
            if let Some(e) = zm.ents.get_mut(&n) {
                e.hidden = true;
            }
            if let Some(a) = zm.actors.get_mut(&n)
                && !a.alive
            {
                a.dead_since = Some(now - 14_500);
            }
        }
        super::mc_nav::forget(world, n);
        diag::info!(
            Sim,
            "bo2mc zombie {n} rises again closer ({reason}, {:.0} blocks from him)",
            d / 36.0
        );
    }
}

/// The round the HUD shows: between rounds BO2 already counts the next one;
/// here its number goes up when night falls.
pub(super) fn shown_round(world: &World, round: i32) -> i32 {
    match world.get_resource::<McRules>() {
        Some(r) if bo2mc::enabled() && r.holding && clock_live() => (round - 1).max(1),
        _ => round,
    }
}
