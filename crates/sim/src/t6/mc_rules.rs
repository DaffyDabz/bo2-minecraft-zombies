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

use std::collections::{BTreeMap, HashMap, HashSet};

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
    /// The bread buy's trigger (entity number), and use held last tick.
    bread: Option<u32>,
    bread_use_was: bool,
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
    /// TranZit's bus at the start, and it has come and gone.
    bus: Option<Bus>,
    bus_done: bool,
    /// Round 1 waits for the bus to leave (BO2's `round_start` held), and
    /// since when.
    round_held: Option<i64>,
    /// The chat's /round ended this round: the next comes at once, no day.
    jump_night: bool,
    /// The dimension the zombies were last put in (a portal trip raises
    /// them all again near the players).
    dimension_seen: u8,
    /// Zombies already seen dead (each drops at most once).
    seen_dead: HashSet<u32>,
    /// The souls round under way, and where the last enemy fell (its Max
    /// Ammo drops there).
    souls: Option<Souls>,
    last_kill: Option<[f32; 3]>,
    /// The house's door buys (trigger entities), and use held last tick.
    house_doors: Option<[u32; 2]>,
    door_use_was: bool,
    /// The vault door's buy trigger, and use held last tick.
    vault_door: Option<u32>,
    vault_use_was: bool,
    /// The windows' rebuild triggers, and when the next board goes up while
    /// he holds use.
    window_trigs: Option<Vec<u32>>,
    board_at: Option<i64>,
    /// Animations the glue plays on its own models: entity -> (anim,
    /// start ms, looping); a finished one holds its last frame.
    anims: HashMap<u32, (String, i64, bool)>,
    last_ms: i64,
    /// Monkey bombs he had when the box gave him more (they add up).
    monkeys_before: Option<i32>,
}

/// The bus's run: its model and the driver's, the stage and when it began,
/// its x (map) and speed once it drives off.
struct Bus {
    model: u32,
    driver: u32,
    stage: u8,
    at: i64,
    x: f32,
    speed: f32,
    /// When a player was last seen aboard (the doors shut 2 s after).
    aboard_at: i64,
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
pub(super) fn push(r: Request) {
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

/// The player starts: round the middle of the room, facing the north door.
fn start_spots() -> Vec<[f32; 3]> {
    [[0, 0], [1, 0], [-1, 0], [0, 1], [1, 1], [-1, 1], [2, 0], [-2, 0]]
        .iter()
        .map(|&[dx, dz]| bo2mc::cell_floor(dx, dz))
        .collect()
}

/// A copy of the wall buy selling `from` (its structs, renamed) selling
/// `to` instead: the frag grenades (10-07, his ask: grenades are bought,
/// and Nuketown has no frag chalk; BO2 has none, it uses the Semtex
/// grenade's) and the vault's rare weapons (no chalk: their sign says it).
fn clone_wallbuy(ents: &mut Vec<Vec<(String, String)>>, from: &str, to: &str) {
    let is_from = |e: &Vec<(String, String)>| {
        get(e, "classname") == Some("script_struct")
            && matches!(get(e, "targetname"), Some("weapon_upgrade" | "claymore_purchase"))
            && get(e, "zombie_weapon_upgrade") == Some(from)
    };
    let Some(source) = ents.iter().find(|e| is_from(e)).cloned() else {
        diag::warn!(Sim, "bo2mc map: no {from} wall buy to copy for {to}");
        return;
    };
    let suffix = format!("_bo2mc_{to}");
    let rename = |e: &mut Vec<(String, String)>, top: bool| {
        for k in ["targetname", "target"] {
            if let Some(v) = get(e, k).filter(|_| k == "target" || !top).map(|v| format!("{v}{suffix}")) {
                set(e, k, v);
            }
        }
    };
    let mut top = source;
    rename(&mut top, true);
    set(&mut top, "zombie_weapon_upgrade", to.to_owned());
    let mut out = vec![top];
    // Its chalk and model structs (two levels of `target`).
    let mut want: Vec<String> = get(&out[0], "target").map(|t| t.trim_end_matches(suffix.as_str()).to_owned()).into_iter().collect();
    for _ in 0..2 {
        let mut next = Vec::new();
        for e in ents.iter() {
            if get(e, "classname") != Some("script_struct") || !want.iter().any(|w| get(e, "targetname") == Some(w)) {
                continue;
            }
            let mut c = e.clone();
            if let Some(t) = get(&c, "target") {
                next.push(t.to_owned());
            }
            // Its model stays the copied one's: BO2 sizes the use box
            // from it (a model the map has not loaded gives no box, and
            // no buy). The gun that slides out is the weapon's own
            // (`wallbuys::world_field`).
            rename(&mut c, false);
            out.push(c);
        }
        want = next;
    }
    diag::info!(Sim, "bo2mc map: {to} wall buy added ({} structs, a copy of {from})", out.len());
    ents.extend(out);
}

/// Nuketown's map entities on the block world: what makes no sense there
/// goes, the starts, the box and the wall buys move into the spawn room.
pub(super) fn filter_map_entities(ents: &mut Vec<Vec<(String, String)>>) {
    let before = ents.len();
    ents.retain(|e| keep(e));
    let dropped = before - ents.len();
    clone_wallbuy(ents, "sticky_grenade_zm", "frag_grenade_zm");
    for c in bo2mc::CHALKS.iter().filter(|c| matches!(c.wall, bo2mc::ChalkWall::VaultWest | bo2mc::ChalkWall::VaultEast) && !c.weapon.is_empty()) {
        clone_wallbuy(ents, "ak74u_zm", c.weapon);
    }
    let (start_pos, start_yaw) = bo2mc::player_start();
    let starts = start_spots();
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
            // In the bus (TranZit's arrival), else in the room.
            let (p, yaw) = if bo2mc::bus_on() {
                let s = bo2mc::bus_starts();
                s[n_start % s.len()]
            } else {
                (starts[n_start % starts.len()], start_yaw)
            };
            n_start += 1;
            set(e, "origin", text3([p[0], p[1], p[2] + 1.0]));
            set(e, "angles", text3([0.0, yaw, 0.0]));
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
            let weapon = get(e, "zombie_weapon_upgrade").unwrap_or("").to_owned();
            let Some((p, face)) = bo2mc::chalk_of(&weapon).map(|c| c.spot()) else {
                diag::warn!(Sim, "bo2mc map: wall buy {weapon:?} has no chalk spot, left where it was");
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
        ("maps/mp/zombies/_zm", "round_start", round_start),
        ("maps/mp/zombies/_zm_zonemgr", "create_spawner_list", spawner_list),
        ("maps/mp/zm_nuked", "include_powerups", include_powerups),
        ("maps/mp/zm_nuked", "custom_add_weapons", prison_weapons),
        ("maps/mp/zombies/_zm_powerups", "func_should_drop_carpenter", yes),
        ("maps/mp/zombies/_zm_powerups", "start_carpenter", carpenter),
        ("maps/mp/zombies/_zm_powerups", "start_carpenter_new", carpenter),
        // Playtest 1: no health coming back by itself (food heals, on the
        // world side).
        ("maps/mp/zombies/_zm_playerhealth", "playerhealthregen", nothing),
        // Playtest 1b: every gun is an item in its own slot; a wall buy or
        // a box gun never replaces the one in his hands.
        ("maps/mp/zombies/_zm_utility", "get_player_weapon_limit", gun_limit),
        // 10-07: grenades, claymores and monkey bombs are items: none come
        // free each round, they come from the chalk and the box.
        ("maps/mp/zombies/_zm", "award_grenades_for_survivors", nothing),
        ("maps/mp/zombies/_zm_weap_claymore", "give_claymores_after_rounds", nothing),
        ("maps/mp/zombies/_zm_magicbox", "treasure_chest_canplayerreceiveweapon", monkeys_in_box),
        ("maps/mp/zombies/_zm_weap_cymbal_monkey", "player_give_cymbal_monkey", monkeys_given),
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

/// Monkey bombs a box hit gives (BO2's own three), on top of his.
const MONKEYS_PER_BOX: i32 = 3;

/// His monkey bombs and how many he can carry (0, 0 without any).
fn monkey_count(world: &mut World) -> (Option<u32>, i32, i32) {
    let Ok(w) = super::weapon(world, "cymbal_monkey_zm") else {
        return (None, 0, 0);
    };
    let f = frame(world);
    let cap = f.combat_facts_for(w).map_or(0, |c| c.clip_size);
    let held = f.player(ClientId(0)).is_some_and(|ps| ps.weapons.contains(&(w as i32)));
    let n = if held { crate::script_player::ammo_clip(&f, ClientId(0), w) } else { 0 };
    (Some(w), n, cap)
}

/// The box offers monkey bombs to him while he has room for more (BO2
/// never offers a weapon he holds).
fn monkeys_in_box(vm: &mut Vm<World>, world: &mut World, _: &Value, args: &[Value]) -> HookAction {
    if !bo2mc::enabled() || args.get(1).is_none_or(|w| vm.to_text(w) != "cymbal_monkey_zm") {
        return HookAction::Continue;
    }
    let (_, n, cap) = monkey_count(world);
    if n > 0 && n < cap {
        return HookAction::Return(Value::Int(1));
    }
    HookAction::Continue
}

/// The box gives him monkey bombs: they add to the ones he has (set in
/// `publish_player` once BO2 has given them).
fn monkeys_given(_: &mut Vm<World>, world: &mut World, _: &Value, _: &[Value]) -> HookAction {
    if bo2mc::enabled() {
        let (_, n, _) = monkey_count(world);
        world.resource_mut::<McRules>().monkeys_before = Some(n);
    }
    HookAction::Continue
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
    let mut souls = false;
    let mut jump = false;
    if let Some(mut r) = world.get_resource_mut::<McRules>() {
        jump = std::mem::take(&mut r.jump_night);
        r.holding = true;
        // The chat's /round: night again at once (no clock: no ten seconds).
        r.holding_since = if jump { now - 10_000 } else { now };
        r.morning_seen = jump;
        r.paused = false;
        if let Some(s) = r.souls.as_mut() {
            s.zombies_done = true;
            souls = true;
        }
    }
    if souls {
        diag::info!(Sim, "bo2mc round {} zombies done at {}s: the souls round waits for its hounds and wolves", round - 1, now / 1000);
        return HookAction::Redirect(wait, vec![name]);
    }
    if bo2mc::endless() {
        diag::info!(Sim, "bo2mc round {} over at {}s in {}: round {round} comes in ten seconds", round - 1, now / 1000, bo2mc::dimension_name(bo2mc::dimension()));
        return HookAction::Redirect(wait, vec![name]);
    }
    if clock_live() {
        push_clock(world, Request::PauseDay(false));
        push_clock(world, Request::SetDay(if jump { NIGHTFALL } else { DAWN }));
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

/// Round 1 (BO2's `round_start`) waits while the bus is still at the stop:
/// it starts when the bus has gone.
fn round_start(_: &mut Vm<World>, world: &mut World, _: &Value, _: &[Value]) -> HookAction {
    if !bo2mc::enabled() || !bo2mc::bus_on() {
        return HookAction::Continue;
    }
    let now = world.resource::<Zm>().now_ms;
    let Some(mut r) = world.get_resource_mut::<McRules>() else {
        return HookAction::Continue;
    };
    if r.bus_done || r.round_held.is_some() {
        return HookAction::Continue;
    }
    r.round_held = Some(now);
    diag::info!(Sim, "bo2mc bus: round 1 waits for the bus at {}s", now / 1000);
    HookAction::Return(Value::Undefined)
}

/// The bus has gone (or never came): round 1 starts.
fn release_round(world: &mut World, now: i64, why: &str) {
    if world.resource_mut::<McRules>().round_held.take().is_none() {
        return;
    }
    with_vm(world, |vm, world| {
        let level = Value::Object(vm.level);
        vm.spawn_named(world, "maps/mp/zombies/_zm", "round_start", level, vec![]);
    });
    diag::info!(Sim, "bo2mc bus: round 1 starts at {}s ({why})", now / 1000);
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

/// Mob of the Dead's weapons in the weapon table (the asset lane loads
/// them when zm_prison's zones are on disk; without them, nothing): name,
/// its Pack-a-Punch weapon, hint, wall-buy cost, announcer line. Out of the
/// box (BO2's `include_weapon(name, 0)`); a wall buy at a spot sells one at
/// its cost and `weapon_give` hands one out. Costs are placeholders.
const PRISON_WEAPONS: &[(&str, &str, &str, i32, &str)] = &[
    ("spoon_zm_alcatraz", "", "Spork", 5000, ""),
    ("spork_zm_alcatraz", "", "Golden Spork", 15000, ""),
    ("blundergat_zm", "blundergat_upgraded_zm", "Blundergat", 10000, "wpck_shotgun"),
];

/// Nuketown's weapon table, then Mob of the Dead's ones on top (above).
fn prison_weapons(vm: &mut Vm<World>, world: &mut World, _: &Value, _: &[Value]) -> HookAction {
    let level = Value::Object(vm.level);
    let mut added = Vec::new();
    for &(name, upgrade, hint, cost, vo) in PRISON_WEAPONS {
        if super::weapon(world, name).is_err() {
            continue;
        }
        let w = vm.string(name);
        let args = vec![w.clone(), Value::Int(0)];
        if vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "include_weapon", level.clone(), args).is_none() {
            diag::warn!(Sim, "bo2mc weapons: no include_weapon");
            break;
        }
        let up = if upgrade.is_empty() || super::weapon(world, upgrade).is_err() {
            Value::Undefined
        } else {
            let u = vm.string(upgrade);
            vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "include_weapon", level.clone(), vec![u.clone(), Value::Int(0)]);
            u
        };
        if name.starts_with("spork") || name.starts_with("spoon") {
            vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "register_melee_weapon_for_level", level.clone(), vec![w.clone()]);
        }
        // Its prompt is the M14's with its name ("Hold [E] for M14
        // [Cost: 500]"): Nuketown has no strings for them.
        let key = format!("BO2MC_WEAPON_{}", name.to_ascii_uppercase());
        let m14 = world.resource::<Zm>().strings.get("ZOMBIE_WEAPON_M14").filter(|t| t.contains("M14")).cloned();
        let key = match m14 {
            Some(t) => {
                world.resource_mut::<Zm>().strings.insert(key.clone(), t.replace("M14", hint));
                key
            }
            None => hint.to_owned(),
        };
        let hint = Value::IStr(vm.intern(&key));
        let vo = vm.string(vo);
        let empty = vm.string("");
        let args = vec![w, up, hint, Value::Int(cost), vo, empty, Value::Undefined];
        if vm.spawn_named(world, "maps/mp/zombies/_zm_weapons", "add_zombie_weapon", level.clone(), args).is_none() {
            diag::warn!(Sim, "bo2mc weapons: no add_zombie_weapon");
            break;
        }
        added.push(name);
    }
    if !added.is_empty() {
        diag::info!(Sim, "bo2mc weapons: {} in the weapon table", added.join(" "));
    }
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

/// A melee weapon's name (the knife and its upgrades, the vault's Spork).
pub(super) fn is_melee(name: &str) -> bool {
    ["knife", "bowie", "tazer", "sickle", "spork", "spoon"].iter().any(|k| name.contains(k))
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
            Ask::Swing { damage, reach } => swing(world, damage, reach),
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

/// BO2 damage per point of a Minecraft weapon's attack damage: an iron
/// sword's 6 is the knife's 150 (a round-one zombie in one swing).
const SWING_DAMAGE_PER_POINT: f32 = 25.0;

/// His Minecraft weapon or tool's swing. His 10-08: "make the Minecraft
/// swords do damage to the Nazi zombies, according to what they are ...
/// that same thing to the other tools". The nearest zombie on his line of
/// sight within reach takes it, as the knife's melee.
fn swing(world: &mut World, damage: f32, reach: f32) {
    let Some(obj) = world.resource::<Zm>().players.get(&0).filter(|p| !p.laststand).map(|p| p.obj) else {
        return;
    };
    let Some((eye, fwd)) = frame(world).player(ClientId(0)).filter(|ps| ps.health > 0).map(|ps| {
        (
            [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current],
            crate::bullet::angles_to_forward(ps.viewangles),
        )
    }) else {
        return;
    };
    let target = {
        let zm = world.resource::<Zm>();
        let mut best: Option<(f32, u32, [f32; 3])> = None;
        for (n, a) in &zm.actors {
            let Some(e) = zm.ents.get(n).filter(|e| a.alive && !e.hidden) else {
                continue;
            };
            // Its body: a standing cylinder at its feet, the ray's nearest
            // point to its middle on it.
            let mid = [e.origin[0], e.origin[1], e.origin[2] + 36.0];
            let t = (0..3).map(|k| (mid[k] - eye[k]) * fwd[k]).sum::<f32>().clamp(0.0, reach);
            let p: [f32; 3] = std::array::from_fn(|k| eye[k] + fwd[k] * t);
            let (dx, dy) = (p[0] - e.origin[0], p[1] - e.origin[1]);
            if dx * dx + dy * dy <= 20.0 * 20.0
                && (e.origin[2] - 4.0..=e.origin[2] + 76.0).contains(&p[2])
                && best.is_none_or(|(b, _, _)| t < b)
            {
                best = Some((t, *n, p));
            }
        }
        best
    };
    let Some((_, n, point)) = target else { return };
    let amount = ((damage * SWING_DAMAGE_PER_POINT).round() as i32).max(1);
    diag::info!(Sim, "bo2mc swing: zombie {n} takes {amount} ({damage:.1} Minecraft damage)");
    with_vm(world, |vm, world| {
        super::actors::damage(
            vm,
            world,
            n,
            Value::Object(obj),
            Value::Object(obj),
            amount,
            0,
            "MOD_MELEE",
            "knife_zm",
            point,
            fwd,
            "torso_upper",
        );
    });
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
        // 10-07: no free frag grenades (BO2 hands them out at the start);
        // grenades come from the chalk.
        if name.trim_end_matches("_mp") == "frag_grenade_zm" {
            crate::script_player::take_weapon(&mut frame(world), client, w);
            diag::info!(Sim, "bo2mc items: free frag grenades taken");
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
    // The box's monkey bombs add to the ones he had.
    if let Some(before) = world.resource_mut::<McRules>().monkeys_before.take() {
        let (w, n, cap) = monkey_count(world);
        if let Some(w) = w {
            let want = (before + MONKEYS_PER_BOX).min(cap.max(1));
            crate::script_player::set_ammo_clip(&mut frame(world), client, w, want);
            if let Some(i) = items.iter_mut().find(|i| i.index == w) {
                i.clip = want;
            }
            diag::info!(Sim, "bo2mc items: box monkey bombs {before} + {MONKEYS_PER_BOX} -> {want} (had {n} after the give, carry {cap})");
        }
    }
    // His claymores all down: the chalk sells him more (BO2 sells them
    // once and gives two each round, which is off).
    if items.iter().any(|i| i.kind == "mine" && i.clip == 0)
        && let Some(p) = world.resource::<Zm>().players.get(&0).map(|p| p.obj)
    {
        let cleared = with_vm(world, |vm, _| {
            let me = Value::Object(p);
            let had = !matches!(field(vm, &me, "current_placeable_mine"), Value::Undefined);
            if had {
                set_field(vm, p, "current_placeable_mine", Value::Undefined);
            }
            had
        })
        .unwrap_or(false);
        if cleared {
            diag::info!(Sim, "bo2mc items: claymores used up, the chalk sells more");
        }
    }
    let shown: Vec<String> = items
        .iter()
        .map(|i| match i.kind {
            "gun" | "melee" => format!("{}#{}:{}", i.name, i.index, i.kind),
            _ => format!("{}#{}:{}x{}", i.name, i.index, i.kind, i.clip),
        })
        .collect();
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
    let perks = world.resource::<Zm>().players.get(&client.0).map_or(Vec::new(), |p| p.perks.iter().cloned().collect());
    bo2mc::set_player(ps.health, ps.max_health, items, perks);
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

/// An animation's length in ms (None: not loaded).
fn anim_ms(world: &World, name: &str) -> Option<i64> {
    let a = world.resource::<Zm>().anims.get(name)?;
    Some(((f32::from(a.numframes.max(1)) / a.framerate.max(1.0)) * 1000.0).max(1.0) as i64)
}

/// The glue's models' animations for their model rows: the bus's and its
/// driver's (`McRules::anims`), the inventory body's idle (looping).
pub(super) fn puppet_anim(world: &World, n: u32, now: i64) -> Option<(String, f32, f32)> {
    let r = world.get_resource::<McRules>()?;
    if let Some((name, start, looping)) = r.anims.get(&n) {
        let length_ms = anim_ms(world, name)?;
        let t = (now - start).max(0);
        if *looping {
            return Some((name.clone(), (t % length_ms) as f32 / length_ms as f32, 1000.0 / length_ms as f32));
        }
        let done = t >= length_ms;
        return Some((name.clone(), (t as f32 / length_ms as f32).min(1.0), if done { 0.0 } else { 1000.0 / length_ms as f32 }));
    }
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

fn bread_hint() -> String {
    format!("Hold ^3[{{+activate}}]^7 to buy Bread x{} [Cost: {}]", bo2mc::BREAD_COUNT, bo2mc::BREAD_COST)
}

/// The bread on the house's north wall (his 10-08): twelve loaves into his
/// Minecraft inventory for 100 points. The loaf itself hangs on the wall
/// from the world side.
fn bread_buy(world: &mut World, now: i64) {
    let client = ClientId(0);
    let hint = bread_hint();
    if world.resource::<McRules>().bread.is_none() {
        let spot = bo2mc::cell_floor(bo2mc::BREAD_CELL.0, bo2mc::BREAD_CELL.1);
        let n = with_vm(world, |vm, world| {
            let t = world.resource_mut::<Zm>().alloc_entnum();
            let tobj = vm.alloc_object(ObjKind::Entity(t));
            world.resource_mut::<Zm>().ents.insert(
                t,
                super::Ent {
                    obj: Some(tobj),
                    classname: "trigger_radius_use".into(),
                    origin: spot,
                    radius: 40.0,
                    height: 72.0,
                    hint: Some(hint.clone()),
                    ..Default::default()
                },
            );
            t
        });
        let Some(n) = n else { return };
        world.resource_mut::<McRules>().bread = Some(n);
        diag::info!(Sim, "bo2mc bread: ent {n} on the north wall");
    }
    // A fresh press of use while it is the trigger he faces.
    let held = crate::script_player::buttons(&mut frame(world), client)
        & (playerstate_iw4::buttons::USE | playerstate_iw4::buttons::USE_RELOAD)
        != 0;
    let was = std::mem::replace(&mut world.resource_mut::<McRules>().bread_use_was, held);
    let facing = world.resource::<Zm>().hints.get(&0).is_some_and(|h| *h == hint);
    if !held || was || !facing {
        return;
    }
    let score = frame(world).client_meta(client).map_or(0, |m| m.score);
    let ok = score >= bo2mc::BREAD_COST;
    let pobj = world.resource::<Zm>().players.get(&0).map(|p| p.obj);
    with_vm(world, |vm, world| {
        let Some(p) = pobj else { return };
        let me = Value::Object(p);
        if ok {
            vm.spawn_named(world, "maps/mp/zombies/_zm_score", "minus_to_player_score", me.clone(), vec![Value::Int(bo2mc::BREAD_COST)]);
        }
        let sound = vm.string(if ok { "purchase" } else { "no_purchase" });
        vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "play_sound_on_ent", me, vec![sound]);
    });
    if ok {
        bo2mc::push(Request::Give { item: "minecraft:bread".into(), count: bo2mc::BREAD_COUNT });
    }
    diag::info!(Sim, "bo2mc bread at {}s: {} ({score} points)", now / 1000, if ok { "bought" } else { "refused: points" });
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
    bread_buy(world, now);
    bus_arrival(world, now);
    house_doors(world, now);
    vault_door(world, now);
    windows(world, now);
    world.resource_mut::<McRules>().last_ms = now;
    inventory_puppet(world);
    publish_player(world);
    spawn_spots(world, now);
    turn_perks_on(world, now);
    night(world, now);
    world_damage(world, now);
    spectator_unseen(world);
    rise_again_closer(world, now);
    nether_drops(world, now);
    souls_round(world, now);
}

/// The Nether has no blazes: its zombies sometimes drop a blaze rod (one in
/// `BLAZE_ROD_CHANCE`), for the Eyes of Ender that open the End.
const BLAZE_ROD_CHANCE: u64 = 6;

fn nether_drops(world: &mut World, now: i64) {
    let mut seen = std::mem::take(&mut world.resource_mut::<McRules>().seen_dead);
    let mut dead = Vec::new();
    {
        let zm = world.resource::<Zm>();
        seen.retain(|n| zm.actors.get(n).is_some_and(|a| !a.alive));
        for (n, a) in &zm.actors {
            if a.alive || a.scripted.is_some() {
                continue;
            }
            // Hidden: one that rose again elsewhere, not a kill.
            let Some(e) = zm.ents.get(n) else { continue };
            if e.hidden || !seen.insert(*n) {
                continue;
            }
            dead.push((*n, e.origin, a.aitype.contains("dog")));
        }
    }
    world.resource_mut::<McRules>().seen_dead = seen;
    if let Some(&(_, at, _)) = dead.last() {
        world.resource_mut::<McRules>().last_kill = Some(at);
    }
    for (n, at, dog) in dead {
        let Some(block) = bo2mc::map_to_block([at[0], at[1], at[2] + 8.0]) else {
            continue;
        };
        // A cheap mix of the zombie and the time, a few bits per roll.
        let mut roll = (u64::from(n).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (now as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)) >> 13;
        let mut next = |m: u64| {
            let r = roll % m;
            roll /= m;
            r
        };
        // His 10-08: "have drops like those mobs? like rotten flesh and
        // bones": a Minecraft zombie's and skeleton's (0-2 of each), and
        // rarely a zombie's iron ingot, carrot or potato. Hellhounds drop
        // nothing.
        if !dog {
            let flesh = next(3) as u32;
            let bones = next(3) as u32;
            if flesh > 0 {
                push(Request::Drop { item: "minecraft:rotten_flesh".into(), count: flesh, at: block });
            }
            if bones > 0 {
                push(Request::Drop { item: "minecraft:bone".into(), count: bones, at: block });
            }
            if next(RARE_DROP_CHANCE) == 0 {
                let item = ["minecraft:iron_ingot", "minecraft:carrot", "minecraft:potato"][next(3) as usize];
                push(Request::Drop { item: item.into(), count: 1, at: block });
            }
        }
        if bo2mc::dimension() == bo2mc::NETHER && next(BLAZE_ROD_CHANCE) == 0 {
            push(Request::Drop { item: "minecraft:blaze_rod".into(), count: 1, at: block });
            diag::info!(Sim, "bo2mc nether: zombie {n} dropped a blaze rod at {block:?}");
        }
    }
}

/// One zombie in this many drops an iron ingot, a carrot or a potato
/// (Minecraft's zombie: 2.5%).
const RARE_DROP_CHANCE: u64 = 40;

// ------------------------------------------------------------ souls rounds

/// Every fifth round is a souls round, Nacht der Untoten's hellhound round:
/// thick fog, "Fetch me their souls!", hellhounds and a pack of angry
/// Minecraft wolves, and no zombies. It is over when every hound and wolf
/// is dead: the last one drops a Max Ammo and the fog lifts.
const SOULS_EVERY: i32 = 5;

/// IW4L_BO2MC_SOULS_EVERY=1 makes every round a souls round (hidden tests).
fn souls_every() -> i32 {
    std::env::var("IW4L_BO2MC_SOULS_EVERY").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0).unwrap_or(SOULS_EVERY)
}
/// The longest the next round waits on a hound or wolf nobody can reach.
const SOULS_GIVE_UP_MS: i64 = 120_000;

#[derive(Default)]
struct Souls {
    round: i32,
    started: i64,
    zombies_done: bool,
    /// The hellhounds (actor numbers).
    dogs: Vec<u32>,
    /// Hellhounds still owed: none came when the round began (no spawn
    /// spots yet), so they are tried again each second.
    owed: usize,
    retry_at: i64,
}

fn start_souls(world: &mut World, now: i64, round: i32) {
    let wolves = (3 + round / 10).min(6) as u32;
    // Nacht der Untoten's solo dog round: six hounds, more each time.
    let want = ((5 + round / 5) as usize).min(12);
    let dogs = super::mc_dogs::spawn(world, round, want);
    let hounds = dogs.len();
    let owed = if dogs.is_empty() { want } else { 0 };
    world.resource_mut::<McRules>().souls =
        Some(Souls { round, started: now, zombies_done: false, dogs, owed, retry_at: now + 1000 });
    world.resource_mut::<McRules>().last_kill = None;
    bo2mc::set_souls_fog(true);
    souls_music(world, "dog_start");
    push(Request::Notice { text: "Fetch me their souls!".into(), seconds: 5.0 });
    push(Request::SoulsWolves { count: wolves });
    diag::info!(Sim, "bo2mc souls round {round} at {}s: {hounds} hellhounds, {wolves} wolves, fog in", now / 1000);
}

/// BO2's own dog-round audio: the music state (`dog_start` / `dog_end`) and,
/// at the start, the round sting and the announcer (`play_dog_round`).
fn souls_music(world: &mut World, state: &str) {
    let player = world.resource::<Zm>().players.get(&0).map(|p| p.obj);
    with_vm(world, |vm, world| {
        let level = Value::Object(vm.level);
        let s = vm.string(state);
        vm.spawn_named(world, "maps/mp/zombies/_zm_audio", "change_zombie_music", level, vec![s]);
        if state == "dog_start"
            && let Some(p) = player
        {
            vm.spawn_named(world, "maps/mp/zombies/_zm_ai_dogs", "play_dog_round", Value::Object(p), vec![]);
        }
    });
}

fn souls_round(world: &mut World, now: i64) {
    let Some((round, started, done, dogs)) =
        world.resource::<McRules>().souls.as_ref().map(|s| (s.round, s.started, s.zombies_done, s.dogs.clone()))
    else {
        return;
    };
    if let Some(at) = bo2mc::take_souls_wolf_death() {
        world.resource_mut::<McRules>().last_kill = Some(at);
    }
    // Nacht's dog round has no zombies: BO2's round queue stays empty, so
    // the round's zombies are "done" at once and only hounds and wolves come.
    if !done {
        with_vm(world, |vm, _| {
            for name in ["zombie_total", "zombie_total_subtract"] {
                let f = vm.intern(name);
                vm.set_raw_field(vm.level, f, Value::Int(0));
            }
        });
    }
    let (owed, retry_at) = world.resource::<McRules>().souls.as_ref().map_or((0, 0), |s| (s.owed, s.retry_at));
    if owed > 0 {
        if now >= retry_at {
            let more = super::mc_dogs::spawn(world, round, owed);
            if let Some(s) = world.resource_mut::<McRules>().souls.as_mut() {
                s.retry_at = now + 1000;
                if !more.is_empty() {
                    diag::info!(Sim, "bo2mc souls round {round}: {} owed hellhounds came", more.len());
                    s.owed = 0;
                    s.dogs.extend(more);
                }
            }
        }
        return;
    }
    let dogs_alive = {
        let zm = world.resource::<Zm>();
        dogs.iter().filter(|n| zm.actors.get(n).is_some_and(|a| a.alive)).count()
    };
    let wolves = bo2mc::souls_wolves();
    // The wolves join a server tick or two after they are asked for.
    let settled = now - started >= 5000;
    let since = world.resource::<McRules>().holding_since;
    let give_up = done && now - since >= SOULS_GIVE_UP_MS;
    if !(done && settled && dogs_alive == 0 && wolves == 0) && !give_up {
        return;
    }
    let at = world.resource::<McRules>().last_kill.or_else(|| player_spots(world).first().copied());
    if let Some(at) = at {
        with_vm(world, |vm, world| {
            let name = vm.string("full_ammo");
            vm.spawn_named(
                world,
                "maps/mp/zombies/_zm_powerups",
                "specific_powerup_drop",
                Value::Object(vm.level),
                vec![name, Value::Vec3(at)],
            );
        });
    }
    bo2mc::set_souls_fog(false);
    souls_music(world, "dog_end");
    if clock_live() && !bo2mc::endless() {
        push_clock(world, Request::PauseDay(false));
        push_clock(world, Request::SetDay(DAWN));
    }
    {
        let mut r = world.resource_mut::<McRules>();
        r.souls = None;
        r.holding_since = now;
        r.morning_seen = false;
    }
    diag::info!(
        Sim,
        "bo2mc souls round {round} cleared at {}s{}: Max Ammo at {at:?}, fog lifts",
        now / 1000,
        if give_up { format!(" (gave up on {dogs_alive} hounds, {wolves} wolves)") } else { String::new() }
    );
}

// ------------------------------------------------------------ TranZit's bus

/// The driver's spot on the bus (from the bus's origin, its frame): his
/// seat behind the windscreen, on the left, and the way he faces (his
/// model looks down its -y: 90 faces the road ahead). His 10-08: "the
/// driver is in front of it" (he sat past the windscreen, at x 385).
const DRIVER_AT: [f32; 4] = [330.0, 45.0, 40.0, 90.0];

/// `IW4L_BO2MC_DRIVER="x y z yaw"` places him elsewhere (hidden tests).
fn driver_at() -> [f32; 4] {
    std::env::var("IW4L_BO2MC_DRIVER")
        .ok()
        .and_then(|v| {
            let n: Vec<f32> = v.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
        })
        .unwrap_or(DRIVER_AT)
}
/// The bus's top speed and how fast it gets there (map units, seconds).
const BUS_TOP_SPEED: f32 = 500.0;
const BUS_ACCEL: f32 = 120.0;
/// It is gone (hidden) once its origin passes this x: near the road's end.
const BUS_GONE_X: f32 = 50.0 * 36.0;
/// The longest the doors stay open with someone still aboard.
/// The longest round 1 waits for the bus (from when BO2 would start it).
const ROUND_HOLD_MAX_MS: i64 = 180_000;
const BUS_WAIT_MS: i64 = 45_000;

/// A script model of the glue's own, standing at `origin`.
fn new_model(world: &mut World, model: &str, origin: [f32; 3], yaw: f32) -> Option<u32> {
    with_vm(world, |vm, world| {
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        world.resource_mut::<Zm>().ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                classname: "script_model".into(),
                model: model.into(),
                origin,
                angles: [0.0, yaw, 0.0],
                ..Default::default()
            },
        );
        n
    })
}

fn play_anim(world: &mut World, n: u32, anim: &str, now: i64, looping: bool) {
    if anim_ms(world, anim).is_none() {
        diag::info!(Sim, "bo2mc bus: no animation {anim}");
    }
    world.resource_mut::<McRules>().anims.insert(n, (anim.to_owned(), now, looping));
}

fn bus_sound(world: &mut World, alias: &str, at: [f32; 3]) {
    super::natives_fx::sound(world, crate::EventAudience::All, alias, at);
}

/// bo2mc: the bus's model while it stands at the stop: solid as its own
/// boxes (`transit`), as TranZit's bus is.
pub(super) fn parked_bus(world: &World) -> Option<u32> {
    let bus = world.get_resource::<McRules>()?.bus.as_ref()?;
    (bus.stage < 5).then_some(bus.model)
}

/// The players' positions (map).
fn player_spots(world: &mut World) -> Vec<[f32; 3]> {
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let f = frame(world);
    clients.iter().filter_map(|&c| f.player(ClientId(c)).filter(|p| p.pm_type == 0).map(|p| p.origin)).collect()
}

/// The arrival (`bo2mc::bus_on`): the bus stands at the road's stop when
/// the world is in; the driver calls the stop, the doors open, everyone
/// gets off, the doors shut and the bus drives off east and is gone.
fn bus_arrival(world: &mut World, now: i64) {
    if !bo2mc::bus_on() {
        return;
    }
    // A world that never stands (or a bus that never comes) holds no round.
    if world.resource::<McRules>().round_held.is_some_and(|at| now - at >= ROUND_HOLD_MAX_MS) {
        world.resource_mut::<McRules>().bus_done = true;
        release_round(world, now, "the bus never came");
    }
    if world.resource::<McRules>().bus_done {
        release_round(world, now, "the bus has gone");
        return;
    }
    if !bo2mc::room_built() {
        return;
    }
    let Some(bus) = world.resource_mut::<McRules>().bus.take() else {
        // Put it up once.
        let o = bo2mc::BUS_ORIGIN;
        let d = driver_at();
        let driver_at = [o[0] + d[0], o[1] + d[1], o[2] + d[2]];
        let (Some(model), Some(driver)) = (new_model(world, "veh_t6_civ_bus_zombie", o, 0.0), new_model(world, "p6_anim_zm_bus_driver", driver_at, d[3])) else {
            diag::warn!(Sim, "bo2mc bus: its models would not load; no bus");
            world.resource_mut::<McRules>().bus_done = true;
            bo2mc::push(Request::BusGone);
            return;
        };
        play_anim(world, model, "v_zombie_bus_all_doors_idle_closed", now, true);
        play_anim(world, driver, "ai_zombie_bus_driver_idle", now, true);
        bus_sound(world, "zmb_bus_airbrake", o);
        diag::info!(Sim, "bo2mc bus: parked at the stop (ents {model}, {driver})");
        world.resource_mut::<McRules>().bus = Some(Bus { model, driver, stage: 0, at: now, x: o[0], speed: 0.0, aboard_at: now });
        return;
    };
    let mut bus = bus;
    let o = [bus.x, bo2mc::BUS_ORIGIN[1], bo2mc::BUS_ORIGIN[2]];
    let front = [o[0] + 337.0, o[1] - 35.0, o[2] + 40.0];
    let since = now - bus.at;
    let pick = (now / 7 % 3) as usize;
    let next = |bus: &mut Bus, stage: u8| {
        bus.stage = stage;
        bus.at = now;
    };
    match bus.stage {
        // The driver calls the stop.
        0 if since >= 2000 => {
            bus_sound(world, &format!("vox_bus_stop_generic_{pick}"), front);
            play_anim(world, bus.driver, "ai_zombie_bus_driver_idle_dialog", now, true);
            next(&mut bus, 1);
        }
        // The doors open: everybody off.
        1 if since >= 2500 => {
            play_anim(world, bus.model, "v_zombie_bus_all_doors_open", now, false);
            bus_sound(world, "zmb_bus_door_open", front);
            bus_sound(world, &format!("vox_bus_doors_open_{}", (now / 11 % 5) as usize), front);
            diag::info!(Sim, "bo2mc bus: doors open at {}s", now / 1000);
            bus.aboard_at = now;
            next(&mut bus, 2);
        }
        // Everyone is off (two seconds clear of it), or he stayed too long.
        2 => {
            let talking = world.resource::<McRules>().anims.get(&bus.driver).is_some_and(|a| a.0 != "ai_zombie_bus_driver_idle");
            if since >= 3000 && talking {
                play_anim(world, bus.driver, "ai_zombie_bus_driver_idle", now, true);
            }
            if player_spots(world).iter().any(|p| bo2mc::in_bus(*p)) {
                bus.aboard_at = now;
            }
            if now - bus.aboard_at >= 2000 || since >= BUS_WAIT_MS {
                next(&mut bus, 3);
            }
        }
        3 => {
            play_anim(world, bus.model, "v_zombie_bus_all_doors_close", now, false);
            bus_sound(world, "zmb_bus_door_close", front);
            bus_sound(world, &format!("vox_bus_doors_close_{}", (now / 13 % 5) as usize), front);
            play_anim(world, bus.driver, "ai_zombie_bus_driver_idle", now, true);
            bo2mc::push(Request::BusGone);
            diag::info!(Sim, "bo2mc bus: everyone off, doors shut at {}s", now / 1000);
            next(&mut bus, 4);
        }
        // The horn, and away.
        4 if since >= 2500 => {
            bus_sound(world, "zmb_bus_horn_leave", front);
            bus_sound(world, "zmb_bus_start_move", o);
            if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&bus.model) {
                e.loop_sound = Some("zmb_bus_exterior_loop".into());
            }
            next(&mut bus, 5);
        }
        5 => {
            let dt = ((now - world.resource::<McRules>().last_ms).clamp(0, 200)) as f32 / 1000.0;
            bus.speed = (bus.speed + BUS_ACCEL * dt).min(BUS_TOP_SPEED);
            bus.x += bus.speed * dt;
            let gone = bus.x > BUS_GONE_X;
            let mut zm = world.resource_mut::<Zm>();
            if let Some(e) = zm.ents.get_mut(&bus.model) {
                e.origin[0] = bus.x;
                e.hidden = gone;
                if gone {
                    e.loop_sound = None;
                }
            }
            if let Some(e) = zm.ents.get_mut(&bus.driver) {
                e.origin[0] = bus.x + driver_at()[0];
                e.hidden = gone;
            }
            if gone {
                drop(zm);
                diag::info!(Sim, "bo2mc bus: gone down the road at {}s", now / 1000);
                let mut r = world.resource_mut::<McRules>();
                r.bus_done = true;
                r.anims.remove(&bus.model);
                r.anims.remove(&bus.driver);
                return;
            }
        }
        _ => {}
    }
    world.resource_mut::<McRules>().bus = Some(bus);
}

fn door_hint() -> String {
    format!("Hold ^3[{{+activate}}]^7 to open Door [Cost: {}]", bo2mc::HOUSE_DOOR_COST)
}

/// The house's doors are bought (`bo2mc::HOUSE_DOOR_COST`, on with the
/// bus): a buy trigger outside each door while they are locked; a buy
/// opens the house.
fn house_doors(world: &mut World, now: i64) {
    if !bo2mc::bus_on() || !bo2mc::room_built() {
        return;
    }
    let client = ClientId(0);
    let trigs = match world.resource::<McRules>().house_doors {
        Some(t) => t,
        None => {
            if !bo2mc::doors_locked() {
                return;
            }
            let mut made = Vec::new();
            for d in bo2mc::DOORS {
                let spot = d.front(1);
                let n = with_vm(world, |vm, world| {
                    let n = world.resource_mut::<Zm>().alloc_entnum();
                    let obj = vm.alloc_object(ObjKind::Entity(n));
                    world.resource_mut::<Zm>().ents.insert(
                        n,
                        super::Ent {
                            obj: Some(obj),
                            classname: "trigger_radius_use".into(),
                            origin: spot,
                            radius: 44.0,
                            height: 72.0,
                            hint: Some(door_hint()),
                            ..Default::default()
                        },
                    );
                    n
                });
                made.extend(n);
            }
            let [a, b] = made[..] else { return };
            diag::info!(Sim, "bo2mc house: doors locked, buys {a} and {b} ({} points)", bo2mc::HOUSE_DOOR_COST);
            world.resource_mut::<McRules>().house_doors = Some([a, b]);
            [a, b]
        }
    };
    if !bo2mc::doors_locked() {
        return;
    }
    let held = crate::script_player::buttons(&mut frame(world), client)
        & (playerstate_iw4::buttons::USE | playerstate_iw4::buttons::USE_RELOAD)
        != 0;
    let was = std::mem::replace(&mut world.resource_mut::<McRules>().door_use_was, held);
    let hint = door_hint();
    let facing = world.resource::<Zm>().hints.get(&0).is_some_and(|h| *h == hint);
    if !held || was || !facing {
        return;
    }
    let Some(me) = frame(world).player(client).map(|p| p.origin) else {
        return;
    };
    // The door he stands at.
    let door = (0..bo2mc::DOORS.len())
        .min_by(|&a, &b| {
            let d = |i: usize| {
                let o = world.resource::<Zm>().ents.get(&trigs[i]).map_or([0.0; 3], |e| e.origin);
                (o[0] - me[0]).powi(2) + (o[1] - me[1]).powi(2)
            };
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(0);
    let cost = bo2mc::HOUSE_DOOR_COST;
    let score = frame(world).client_meta(client).map_or(0, |m| m.score);
    let ok = score >= cost;
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
    diag::info!(Sim, "bo2mc house: door {door} {} at {}s ({score} points)", if ok { "bought" } else { "refused: points" }, now / 1000);
    if !ok {
        return;
    }
    bo2mc::unlock_doors();
    bo2mc::push(Request::OpenDoor { door });
    let mut zm = world.resource_mut::<Zm>();
    for t in trigs {
        if let Some(e) = zm.ents.get_mut(&t) {
            e.trigger_off = true;
            e.hint = None;
        }
    }
}

fn vault_hint() -> String {
    format!("Hold ^3[{{+activate}}]^7 to open Door [Cost: {}]", bo2mc::VAULT_DOOR_COST)
}

/// The vault's door (his 10-07 ask): a buy trigger outside it while it is
/// locked, `bo2mc::VAULT_DOOR_COST`.
fn vault_door(world: &mut World, now: i64) {
    if !bo2mc::room_built() || !bo2mc::vault_locked() {
        return;
    }
    let client = ClientId(0);
    let trig = match world.resource::<McRules>().vault_door {
        Some(t) => t,
        None => {
            let d = bo2mc::VAULT_DOOR;
            let spot = bo2mc::cell_floor(d.cell[0] + d.outward[0], d.cell[2] + d.outward[1]);
            let n = with_vm(world, |vm, world| {
                let n = world.resource_mut::<Zm>().alloc_entnum();
                let obj = vm.alloc_object(ObjKind::Entity(n));
                world.resource_mut::<Zm>().ents.insert(
                    n,
                    super::Ent {
                        obj: Some(obj),
                        classname: "trigger_radius_use".into(),
                        origin: spot,
                        radius: 44.0,
                        height: 72.0,
                        hint: Some(vault_hint()),
                        ..Default::default()
                    },
                );
                n
            });
            let Some(n) = n else { return };
            diag::info!(Sim, "bo2mc vault: door locked, buy {n} at {spot:?} ({} points)", bo2mc::VAULT_DOOR_COST);
            world.resource_mut::<McRules>().vault_door = Some(n);
            n
        }
    };
    let held = crate::script_player::buttons(&mut frame(world), client)
        & (playerstate_iw4::buttons::USE | playerstate_iw4::buttons::USE_RELOAD)
        != 0;
    let was = std::mem::replace(&mut world.resource_mut::<McRules>().vault_use_was, held);
    let hint = vault_hint();
    let facing = world.resource::<Zm>().hints.get(&0).is_some_and(|h| *h == hint);
    if !held || was || !facing {
        return;
    }
    let cost = bo2mc::VAULT_DOOR_COST;
    let score = frame(world).client_meta(client).map_or(0, |m| m.score);
    let ok = score >= cost;
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
    diag::info!(Sim, "bo2mc vault: door {} at {}s ({score} points)", if ok { "bought" } else { "refused: points" }, now / 1000);
    if !ok {
        return;
    }
    bo2mc::unlock_vault();
    bo2mc::push(Request::OpenVault);
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&trig) {
        e.trigger_off = true;
        e.hint = None;
    }
}

const REBUILD_HINT: &str = "Hold ^3[{+activate}]^7 to Rebuild Barrier";
/// Holding use: the first board this long after he presses, then one a
/// board this often (BO2's pace).
const BOARD_FIRST_MS: i64 = 500;
const BOARD_EVERY_MS: i64 = 1000;

/// The windows (`bo2mc::WINDOWS`): a rebuild trigger inside each, shown
/// while boards are down; holding use nails one back a second, 10 points
/// each (BO2's barricades, Minecraft's planks).
fn windows(world: &mut World, now: i64) {
    if !bo2mc::room_built() {
        return;
    }
    let trigs = match world.resource::<McRules>().window_trigs.clone() {
        Some(t) => t,
        None => {
            let mut made = Vec::new();
            for w in bo2mc::WINDOWS {
                let spot = w.inside();
                let n = with_vm(world, |vm, world| {
                    let n = world.resource_mut::<Zm>().alloc_entnum();
                    let obj = vm.alloc_object(ObjKind::Entity(n));
                    world.resource_mut::<Zm>().ents.insert(
                        n,
                        super::Ent {
                            obj: Some(obj),
                            classname: "trigger_radius_use".into(),
                            origin: spot,
                            radius: 54.0,
                            height: 72.0,
                            hint: None,
                            trigger_off: true,
                            ..Default::default()
                        },
                    );
                    n
                });
                made.extend(n);
            }
            diag::info!(Sim, "bo2mc windows: {} rebuild triggers {made:?}", made.len());
            world.resource_mut::<McRules>().window_trigs = Some(made.clone());
            made
        }
    };
    // Each trigger on while its window is missing boards.
    let mut open = Vec::new();
    {
        let mut zm = world.resource_mut::<Zm>();
        for (w, t) in trigs.iter().enumerate() {
            let missing = bo2mc::boards_up(w).is_some_and(|n| (n as usize) < bo2mc::WINDOW_BOARDS);
            if let Some(e) = zm.ents.get_mut(t) {
                e.trigger_off = !missing;
                e.hint = missing.then(|| REBUILD_HINT.to_string());
            }
            if missing {
                open.push(w);
            }
        }
    }
    let client = ClientId(0);
    let held = crate::script_player::buttons(&mut frame(world), client)
        & (playerstate_iw4::buttons::USE | playerstate_iw4::buttons::USE_RELOAD)
        != 0;
    let facing = world.resource::<Zm>().hints.get(&0).is_some_and(|h| h == REBUILD_HINT);
    if !held || !facing || open.is_empty() {
        world.resource_mut::<McRules>().board_at = None;
        return;
    }
    let Some(at) = world.resource::<McRules>().board_at else {
        world.resource_mut::<McRules>().board_at = Some(now + BOARD_FIRST_MS);
        return;
    };
    if now < at {
        return;
    }
    world.resource_mut::<McRules>().board_at = Some(now + BOARD_EVERY_MS);
    let Some(me) = frame(world).player(client).map(|p| p.origin) else {
        return;
    };
    // The window he stands at.
    let Some(window) = open.into_iter().min_by(|&a, &b| {
        let d = |w: usize| {
            let o = bo2mc::WINDOWS[w].inside();
            (o[0] - me[0]).powi(2) + (o[1] - me[1]).powi(2)
        };
        d(a).total_cmp(&d(b))
    }) else {
        return;
    };
    bo2mc::push(Request::Rebuild { window });
    let pobj = world.resource::<Zm>().players.get(&0).map(|p| p.obj);
    with_vm(world, |vm, world| {
        let Some(p) = pobj else { return };
        vm.spawn_named(world, "maps/mp/zombies/_zm_score", "add_to_player_score", Value::Object(p), vec![Value::Int(bo2mc::BOARD_POINTS)]);
    });
    diag::info!(Sim, "bo2mc windows: board back in window {window} at {}s (+{})", now / 1000, bo2mc::BOARD_POINTS);
}

/// Test aid: IW4L_BO2MC_TEST_POINTS=N adds N points once, 5 s in (to buy
/// several guns in a hidden run).
fn test_points(world: &mut World, now: i64) {
    static N: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    let n = *N.get_or_init(|| std::env::var("IW4L_BO2MC_TEST_POINTS").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
    if n <= 0 || now < 5000 || world.resource::<McRules>().points_given {
        return;
    }
    // Once he is in (his spawn sets the score to the start's 500).
    if world.resource::<Zm>().wallbuy_since.is_none_or(|t| now - t < 3000) {
        return;
    }
    let Some(obj) = world.resource::<Zm>().players.get(&0).filter(|p| p.begun).map(|p| p.obj) else {
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
    // The Nether and the End have no day: BO2's own ten seconds between
    // rounds, the waves never stop.
    let live = clock_live() && !bo2mc::endless();
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
        if fall && world.resource::<McRules>().souls.is_none() {
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
            if round > 0 && round % souls_every() == 0 {
                start_souls(world, now, round);
            }
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
    test_souls(world, now, round);
}

/// The chat's /round <n>: round n, now. In a round, its zombies drop dead
/// (no power-ups) and BO2's own round end counts on to n, and night comes
/// again at once; between rounds the coming round is n and night falls
/// now. Round 1 still waiting for the bus is n when the bus has gone.
fn jump_round(world: &mut World, now: i64, n: i32) {
    let n = n.clamp(1, 255);
    let (holding, held) = {
        let r = world.resource::<McRules>();
        (r.holding, r.round_held.is_some())
    };
    let between = holding || held;
    // In a round BO2 adds the one itself as the round ends.
    let set = if between { n } else { n - 1 };
    with_vm(world, |vm, _| {
        let lv = vm.level;
        let level = Value::Object(lv);
        set_field(vm, lv, "round_number", Value::Int(set));
        if between {
            // Their pace for round n, as round_think sets it at the end of
            // the round before (round 1's is init's own).
            let vars = field(vm, &level, "zombie_vars");
            let easy = field(vm, &level, "gamedifficulty").as_int() == Some(0);
            let name = if easy { "zombie_move_speed_multiplier_easy" } else { "zombie_move_speed_multiplier" };
            if let Some(m) = field(vm, &vars, name).as_float()
                && n > 1
            {
                set_field(vm, lv, "zombie_move_speed", Value::Float((n - 1) as f32 * m));
            }
        } else {
            for name in ["zombie_total", "zombie_total_subtract"] {
                set_field(vm, lv, name, Value::Int(0));
            }
        }
    });
    if held {
        diag::info!(Sim, "bo2mc chat: /round {n}: round {n} starts when the bus has gone");
        return;
    }
    if holding {
        world.resource_mut::<McRules>().morning_seen = true;
        world.resource_mut::<McRules>().holding_since = now - 10_000;
        if clock_live() {
            push_clock(world, Request::PauseDay(false));
            push_clock(world, Request::SetDay(NIGHTFALL));
        }
        diag::info!(Sim, "bo2mc chat: /round {n} between rounds: night falls now");
        return;
    }
    // Every zombie of this round drops; round_wait sees none left.
    let living: Vec<(u32, ObjRef, [f32; 3], i32)> = {
        let zm = world.resource::<Zm>();
        zm.actors
            .iter()
            .filter(|(_, a)| a.alive)
            .filter_map(|(n, a)| Some((*n, a.obj?, zm.ents.get(n)?.origin, a.health)))
            .collect()
    };
    let killed = living.len();
    with_vm(world, |vm, world| {
        for (k, obj, at, health) in living {
            set_field(vm, obj, "no_powerups", Value::Int(1));
            super::actors::damage(vm, world, k, Value::Object(obj), Value::Object(obj), health + 100, 0, "MOD_UNKNOWN", "none", at, [0.0; 3], "none");
        }
    });
    world.resource_mut::<McRules>().jump_night = true;
    diag::info!(Sim, "bo2mc chat: /round {n} at {}s: this round's {killed} zombies dropped, round {n} next", now / 1000);
}

/// The chat's /points: BO2's own score calls, as a buy or a board uses.
fn chat_points(world: &mut World, amount: i32, set: bool) {
    let Some(obj) = world.resource::<Zm>().players.get(&0).filter(|p| p.begun).map(|p| p.obj) else {
        diag::info!(Sim, "bo2mc chat: /points before he is in: dropped");
        return;
    };
    let score = frame(world).client_meta(ClientId(0)).map_or(0, |m| m.score);
    let add = if set { amount.max(0) - score } else { amount.max(-score) };
    with_vm(world, |vm, world| {
        let me = Value::Object(obj);
        if add > 0 {
            vm.spawn_named(world, "maps/mp/zombies/_zm_score", "add_to_player_score", me, vec![Value::Int(add)]);
        } else if add < 0 {
            vm.spawn_named(world, "maps/mp/zombies/_zm_score", "minus_to_player_score", me, vec![Value::Int(-add)]);
        }
    });
    diag::info!(Sim, "bo2mc chat: /points {add:+} ({score} -> {})", score + add);
}

/// IW4L_BO2MC_TEST_SOULS=<seconds> (hidden tests): a souls round starts that
/// many seconds in, whatever the round, so a test can watch it from outside.
fn test_souls(world: &mut World, now: i64, round: i32) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(at) = std::env::var("IW4L_BO2MC_TEST_SOULS").ok().and_then(|v| v.parse::<i64>().ok()) else {
        return;
    };
    if now < at * 1000 || world.resource::<McRules>().souls.is_some() || DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    start_souls(world, now, round.max(1));
}

/// A spectator (the chat's /gamemode spectator) is not hunted: BO2's
/// `ignoreme` on the player while the mode lasts.
fn spectator_unseen(world: &mut World) {
    static WAS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let now = bo2mc::game_mode() == bo2mc::SPECTATOR;
    if WAS.swap(now, std::sync::atomic::Ordering::Relaxed) == now {
        return;
    }
    let Some(obj) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) else {
        return;
    };
    diag::info!(Sim, "bo2mc chat: spectator {now}: zombies ignore him");
    with_vm(world, |vm, _| set_field(vm, obj, "ignoreme", Value::Int(i32::from(now))));
}

/// Minecraft's own damage the world side measures (falls as Minecraft
/// counts them, drowning, lava, fire, starving) goes through BO2's player
/// damage, so last stand and game over follow as for any hit. Fire and lava
/// are cut by armor as in Minecraft; falls, drowning and starving are not.
fn world_damage(world: &mut World, now: i64) {
    let mut events = bo2mc::take_player_events();
    // The chat's /round and /points, whatever state he is in.
    events.retain(|e| match *e {
        bo2mc::PlayerEvent::Round(n) => {
            jump_round(world, now, n);
            false
        }
        bo2mc::PlayerEvent::Points { amount, set } => {
            chat_points(world, amount, set);
            false
        }
        bo2mc::PlayerEvent::TimeMoved => {
            world.resource_mut::<McRules>().morning_seen = true;
            false
        }
        _ => true,
    });
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
    let phd = world.resource::<Zm>().players.get(&c).is_some_and(|p| p.perks.contains("specialty_flakjacket"));
    for event in events {
        // The chat's /kill and /tp.
        let (amount, cause) = match event {
            bo2mc::PlayerEvent::Damage { amount, cause } => (amount, cause),
            bo2mc::PlayerEvent::Kill => (100_000, "kill"),
            bo2mc::PlayerEvent::Teleport { to } => {
                diag::info!(Sim, "bo2mc chat: teleported to {to:?}");
                super::teleport_player(world, ClientId(c), to);
                continue;
            }
            bo2mc::PlayerEvent::Round(_) | bo2mc::PlayerEvent::Points { .. } | bo2mc::PlayerEvent::TimeMoved => continue,
        };
        if amount <= 0 {
            continue;
        }
        // PhD Flopper: no fall damage, Minecraft's falls too.
        if phd && cause == "fall" {
            diag::info!(Sim, "bo2mc perks: PhD Flopper takes the fall ({amount})");
            continue;
        }
        let means = match cause {
            "lava" | "in_fire" | "on_fire" => "MOD_BURNED",
            "kill" => "MOD_SUICIDE",
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
    // A portal trip: the zombies were in the world left behind; once the
    // room stands in the new one, all of them rise again around the players.
    let changed = {
        let d = bo2mc::dimension();
        let mut r = world.resource_mut::<McRules>();
        if r.dimension_seen != d && bo2mc::room_built() {
            r.dimension_seen = d;
            true
        } else {
            false
        }
    };
    let mut gone = Vec::new();
    with_vm(world, |vm, world| {
        let emerged = vm.intern("completed_emerging_into_playable_area");
        for (n, obj, at, health) in living {
            if !changed && !gsc_t6::truthy(&vm.raw_field(obj, emerged)) {
                continue;
            }
            let d = players
                .iter()
                .map(|p| ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            let reason = {
                let mut zm = world.resource_mut::<Zm>();
                if changed {
                    zm.mc.actors.remove(&n);
                }
                let st = zm.mc.actors.entry(n).or_default();
                let best = match st.best {
                    _ if st.clawing => (d, now),
                    Some((b, t)) if d >= b - 72.0 => (b, t),
                    _ => (d, now),
                };
                st.best = Some(best);
                if changed {
                    Some("the world changed")
                } else if d > 40.0 * 36.0 {
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
            if gone.len() >= 2 && !changed {
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
        Some(r) if bo2mc::enabled() && r.holding && clock_live() && !bo2mc::endless() => (round - 1).max(1),
        _ => round,
    }
}
