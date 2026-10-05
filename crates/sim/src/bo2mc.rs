//! bo2mc - Minecraft Zombies: Black Ops II zombies rounds at night in a
//! Minecraft world (`IW4L_BO2MC=1` on `map t6:zm_nuked`).
//!
//! The bridge between the two halves, process-global like `crate::voxel`:
//! - the Minecraft world's owner (render_anim::minecraft_world, "the world
//!   side": blocks, day clock, inventory, the spawn room's blocks) and
//! - the Black Ops II rules (`crate::t6`, "the rules side": zombies, rounds,
//!   perks, the box, points, last stand).
//!
//! The world side publishes what the rules need to read (room layout,
//! protected blocks, dig times, the clock, where zombies may rise, whether
//! the player is underground); the rules side queues requests the world side
//! carries out (claw a block, stop/start/set the clock, give items).
//! Positions: `[i32; 3]` = block coords (x, y up, z) of the Minecraft world;
//! `[f32; 3]` = map coords (x, y, z up; `crate::voxel::to_map`/`to_block`).
use std::collections::HashSet;
use std::sync::{Mutex, RwLock};

/// Minecraft Zombies is on: the front end's map pick (MINECRAFT), else
/// `IW4L_BO2MC=1` (one switch for every crate: `bo2mc_switch`).
pub fn enabled() -> bool {
    bo2mc_switch::enabled()
}

/// The front end's map pick, set at START MATCH before the map loads:
/// on = MINECRAFT (the Minecraft world), off = NUKETOWN (plain Nuketown).
pub fn set_enabled(on: bool) {
    bo2mc_switch::set_enabled(on);
}

// ---------------------------------------------------------------- world side -> rules

// ---------------------------------------------------------------- the spawn room (fixed layout)
//
// The Minecraft world sits with its spawn point at map origin (see
// render_anim::minecraft_world): the spawn block's cell centre is map (0, 0)
// and the ground's top is map z = 0. The room is built around that cell, so
// its layout is the same constant in map coords on both sides, known before
// the world has loaded (the BO2 scripts place machines at level start).
// Cell offsets: `dx` along map +x (= block +x), `dz` along block +z (= map -y),
// `dy` up (block y; 0 = the first air layer above the floor).

/// Inner air: cells dx -ROOM_HALF_X..=ROOM_HALF_X (17), dz
/// -ROOM_HALF_Z..=ROOM_HALF_Z (13), dy 0..ROOM_HEIGHT (5). Walls are the
/// ring just outside, floor at dy -1, roof at dy ROOM_HEIGHT.
pub const ROOM_HALF_X: i32 = 8;
pub const ROOM_HALF_Z: i32 = 6;
pub const ROOM_HEIGHT: i32 = 5;

/// Map units per block (= crate::voxel::BLOCK).
const B: f32 = 36.0;

/// The player's body on the Minecraft world: Minecraft's 0.6 blocks wide
/// and 1.8 tall (BO2's is 30 x 70: it could not pass a doorway beside an
/// open door, whose panel takes 3/16 of the block).
pub const PLAYER_HALF_WIDTH: f32 = 0.3 * B;
pub const PLAYER_HEIGHT: f32 = 1.8 * B;

/// The map point at the floor of cell (dx, dz)'s centre.
pub fn cell_floor(dx: i32, dz: i32) -> [f32; 3] {
    [dx as f32 * B, -(dz as f32) * B, 0.0]
}

/// One door of the spawn room: its lower half's cell offset (dx, dy=0, dz)
/// in the wall ring, and the outward direction in cells (dx, dz).
#[derive(Clone, Copy, Debug)]
pub struct Door {
    pub cell: [i32; 3],
    pub outward: [i32; 2],
}

/// Two doors, centred on the long walls (the dz walls).
pub const DOORS: [Door; 2] = [
    Door { cell: [0, 0, -(ROOM_HALF_Z + 1)], outward: [0, -1] },
    Door { cell: [0, 0, ROOM_HALF_Z + 1], outward: [0, 1] },
];

/// A spot in the room: map point at floor level and yaw (degrees, map
/// convention: 0 = +x, 90 = +y) the thing faces.
pub type Spot = ([f32; 3], f32);

/// Where the player starts: the middle, facing the north door (map +y).
pub fn player_start() -> Spot {
    (cell_floor(0, 0), 90.0)
}

/// Twelve machine spots: six against each short wall (the dx walls), two
/// cells apart, facing into the room. Order: west wall north to south, then
/// east wall north to south.
pub fn perk_spots() -> Vec<Spot> {
    let mut out = Vec::new();
    for (dx, yaw) in [(-ROOM_HALF_X, 0.0), (ROOM_HALF_X, 180.0)] {
        for dz in [-5, -3, -1, 1, 3, 5] {
            out.push((cell_floor(dx, dz), yaw));
        }
    }
    out
}

/// The box: against the north long wall, between the door and the west
/// corner, facing into the room (map -y).
pub fn box_spot() -> Spot {
    (cell_floor(-4, -ROOM_HALF_Z), -90.0)
}

/// Pack-a-Punch: against the south long wall near the east corner, facing
/// into the room (map +y).
pub fn pap_spot() -> Spot {
    (cell_floor(4, ROOM_HALF_Z), 90.0)
}

/// Is cell offset (dx, dy, dz) inside the room's air box?
pub fn room_contains(c: [i32; 3]) -> bool {
    c[0].abs() <= ROOM_HALF_X && c[2].abs() <= ROOM_HALF_Z && (0..ROOM_HEIGHT).contains(&c[1])
}

/// Absolute block coords of the cell holding map point `p` (None until the
/// block world is active).
pub fn map_to_block(p: [f32; 3]) -> Option<[i32; 3]> {
    let origin = crate::voxel::origin()?;
    let b = crate::voxel::to_block(origin, p);
    Some([b[0].floor() as i32, b[1].floor() as i32, b[2].floor() as i32])
}

/// Map point of block `b`'s bottom centre (where something stands on it
/// when `b` is the air cell above the ground).
pub fn block_to_map(b: [i32; 3]) -> Option<[f32; 3]> {
    let origin = crate::voxel::origin()?;
    Some(crate::voxel::to_map(
        origin,
        [f64::from(b[0]) + 0.5, f64::from(b[1]), f64::from(b[2]) + 0.5],
    ))
}

/// Absolute block coords of room cell offset `c` (None until active).
pub fn room_cell_block(c: [i32; 3]) -> Option<[i32; 3]> {
    let origin = crate::voxel::origin()?;
    Some([
        origin[0].floor() as i32 + c[0],
        origin[1].floor() as i32 + c[1],
        origin[2].floor() as i32 + c[2],
    ])
}

/// Is map point `p` inside the room (its footprint, below the roof)?
pub fn room_contains_map(p: [f32; 3]) -> bool {
    let dx = (p[0] / B).round() as i32;
    let dz = (-p[1] / B).round() as i32;
    let dy = (p[2] / B).floor() as i32;
    room_contains([dx, dy, dz])
}

static ROOM_BUILT: Mutex<bool> = Mutex::new(false);
static PROTECTED: RwLock<Option<HashSet<[i32; 3]>>> = RwLock::new(None);

/// The world side built the room this world load; `protected` = absolute
/// block coords of every wall/floor/roof block (NOT the doors).
pub fn set_room_built(protected: HashSet<[i32; 3]>) {
    *PROTECTED.write().unwrap() = Some(protected);
    *ROOM_BUILT.lock().unwrap() = true;
}

pub fn room_built() -> bool {
    *ROOM_BUILT.lock().unwrap()
}

/// A world load ended: forget the room.
pub fn reset() {
    *PROTECTED.write().unwrap() = None;
    *ROOM_BUILT.lock().unwrap() = false;
    *UNBREAKABLE.write().unwrap() = None;
    *SPAWNS.write().unwrap() = Vec::new();
    *UNDERGROUND.lock().unwrap() = false;
    REQUESTS.lock().unwrap().clear();
    *PLAYER.write().unwrap() = None;
    ASKS.lock().unwrap().clear();
    PLAYER_EVENTS.lock().unwrap().clear();
    *FOOD.lock().unwrap() = (20.0, 5.0);
}

/// A block nobody may break or replace (the room's walls, floor, roof).
/// The doors are NOT protected: zombies break them.
pub fn is_protected(b: [i32; 3]) -> bool {
    PROTECTED.read().unwrap().as_ref().is_some_and(|p| p.contains(&b))
}

/// Blocks the world side refused to break (bedrock and the like), so the
/// rules stop planning through them.
static UNBREAKABLE: RwLock<Option<HashSet<[i32; 3]>>> = RwLock::new(None);

pub fn mark_unbreakable(b: [i32; 3]) {
    UNBREAKABLE.write().unwrap().get_or_insert_with(HashSet::new).insert(b);
}

/// The planning cost of clawing through block `b`, in seconds; None = it
/// can't be broken. The real time is the world side's (by the block's
/// hardness, see `Request::Claw`); planning uses one estimate for all.
pub fn dig_estimate(b: [i32; 3]) -> Option<f32> {
    if is_protected(b) || UNBREAKABLE.read().unwrap().as_ref().is_some_and(|u| u.contains(&b)) {
        return None;
    }
    Some(DIG_ESTIMATE_SECONDS)
}

pub const DIG_ESTIMATE_SECONDS: f32 = 2.0;

/// Seconds one zombie needs for a block of Minecraft hardness
/// `destroy_speed` (dirt 0.5, stone 1.5, planks 2, obsidian 50, bedrock -1):
/// None = never. A spawn room door takes `DOOR_SECONDS`.
pub fn claw_seconds(destroy_speed: f32) -> Option<f32> {
    (destroy_speed >= 0.0).then(|| destroy_speed.clamp(0.4, 12.0))
}

pub const DOOR_SECONDS: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default)]
struct Clock {
    ticks: f64,
    paused: bool,
}
static CLOCK: Mutex<Clock> = Mutex::new(Clock { ticks: 0.0, paused: false });

/// The world side's day clock this frame (0..24000; 13000 = night falls).
pub fn set_day(ticks: f64, paused: bool) {
    *CLOCK.lock().unwrap() = Clock { ticks, paused };
}

pub fn day_ticks() -> f64 {
    CLOCK.lock().unwrap().ticks
}

pub fn day_paused() -> bool {
    CLOCK.lock().unwrap().paused
}

static UNDERGROUND: Mutex<bool> = Mutex::new(false);

/// The local player's feet are below the world's generated ground there.
pub fn set_player_underground(on: bool) {
    *UNDERGROUND.lock().unwrap() = on;
}

pub fn player_underground() -> bool {
    *UNDERGROUND.lock().unwrap()
}

static SPAWNS: RwLock<Vec<[f32; 3]>> = RwLock::new(Vec::new());

/// Where zombies may rise now (map coords, feet on the ground, two blocks of
/// air above): natural ground 12-28 blocks around the player, never inside
/// the room; when the player is underground, the cells right around him.
pub fn set_spawn_candidates(points: Vec<[f32; 3]>) {
    *SPAWNS.write().unwrap() = points;
}

pub fn spawn_candidates() -> Vec<[f32; 3]> {
    SPAWNS.read().unwrap().clone()
}

// ---------------------------------------------------------------- playtest 1: hearts, armor, items
//
// Rules -> world: the player's BO2 health (hearts: 10 BO2 health = 1 heart;
// 100 = 10 hearts, Juggernog's 250 = 25, the ones past 10 golden) and his
// BO2 weapons as items. World -> rules (`ask`): heal from food, the armor he
// wears, the item he picked.

/// One of the player's BO2 weapons as a Minecraft item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeaponItem {
    /// BO2's weapon name (`m1911_zm`, `frag_grenade_zm`, `knife_zm`).
    pub name: String,
    /// Its weapon index (the item is `iw4:weapon/<index>`); 0 = none known.
    pub index: u32,
    /// What it is: "gun", "lethal" (grenades), "tactical" (monkeys),
    /// "mine" (claymores), "melee" (the knife, Bowie, Galvaknuckles).
    pub kind: &'static str,
    /// Rounds in the magazine (grenades: how many).
    pub clip: i32,
    /// Rounds in reserve.
    pub stock: i32,
    /// Pack-a-Punched.
    pub upgraded: bool,
    /// The one in his hands now.
    pub held: bool,
}

#[derive(Clone, Debug, Default)]
struct PlayerFacts {
    health: i32,
    max_health: i32,
    weapons: Vec<WeaponItem>,
}

static PLAYER: RwLock<Option<PlayerFacts>> = RwLock::new(None);

/// The rules side, each tick: his BO2 health and weapons.
pub fn set_player(health: i32, max_health: i32, weapons: Vec<WeaponItem>) {
    *PLAYER.write().unwrap() = Some(PlayerFacts {
        health,
        max_health,
        weapons,
    });
}

/// His health and the most it can be (BO2 units: 10 = one heart).
pub fn player_health() -> Option<(i32, i32)> {
    PLAYER.read().unwrap().as_ref().map(|p| (p.health, p.max_health))
}

/// His BO2 weapons as items, guns first in the order BO2 holds them.
pub fn player_weapons() -> Vec<WeaponItem> {
    PLAYER.read().unwrap().as_ref().map_or(Vec::new(), |p| p.weapons.clone())
}

/// World side -> rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// He picked this BO2 weapon's item (`WeaponItem::name`): a gun goes in
    /// his hands; the knife makes his attack a knife swing.
    Select(String),
    /// He picked a Minecraft item (no BO2 weapon out; his attack is the
    /// world side's).
    SelectBlock,
    /// Food healed him this much (BO2 units: 10 = one heart).
    Heal(i32),
    /// The armor he wears: Minecraft armor points (0..20) and toughness.
    Armor { points: f32, toughness: f32 },
}

static ASKS: Mutex<Vec<Ask>> = Mutex::new(Vec::new());

pub fn ask(a: Ask) {
    ASKS.lock().unwrap().push(a);
}

pub fn take_asks() -> Vec<Ask> {
    std::mem::take(&mut *ASKS.lock().unwrap())
}

/// Minecraft's armor formula on BO2 damage (10 BO2 health = 2 Minecraft
/// health points): what is left of `damage` through `points` of armor with
/// `toughness`.
pub fn armor_cut(damage: i32, points: f32, toughness: f32) -> i32 {
    if points <= 0.0 || damage <= 0 {
        return damage;
    }
    let mc = damage as f32 / 5.0;
    let effective = (points / 5.0).max(points - 4.0 * mc / (toughness + 8.0)).min(20.0);
    let left = mc * (1.0 - effective / 25.0);
    ((left * 5.0).round() as i32).max(1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn armor_cuts_like_minecraft() {
        // No armor: all of it. Full diamond (20, toughness 8) against a
        // 60-point zombie swing (12 Minecraft points): 12 * (1 - 17/25).
        assert_eq!(super::armor_cut(60, 0.0, 0.0), 60);
        assert_eq!(super::armor_cut(60, 20.0, 8.0), 19);
        // Leather (7): max(1.4, 7 - 6) = 1.4 -> 12 * (1 - 1.4/25) -> 57.
        assert_eq!(super::armor_cut(60, 7.0, 0.0), 57);
    }
}

// ---------------------------------------------------------------- rules -> world side

#[derive(Clone, Debug)]
pub enum Request {
    /// A zombie clawed block `block` for `seconds` this tick (several
    /// zombies add up). The world side shows the cracks, plays the hit
    /// sound, and breaks it (both halves of a door) when the work reaches
    /// its dig time.
    Claw { block: [i32; 3], seconds: f32 },
    /// Stop or start the day clock.
    PauseDay(bool),
    /// Move the day clock to this tick (24000 per day).
    SetDay(f64),
    /// Put items in the player's Minecraft inventory (the Carpenter).
    Give { item: String, count: u32 },
    /// A line of text on the screen for this many seconds (warnings).
    Notice { text: String, seconds: f32 },
}

static REQUESTS: Mutex<Vec<Request>> = Mutex::new(Vec::new());

pub fn push(request: Request) {
    REQUESTS.lock().unwrap().push(request);
}

/// Requests waiting for the world side.
pub fn pending() -> usize {
    REQUESTS.lock().unwrap().len()
}

pub fn take_requests() -> Vec<Request> {
    std::mem::take(&mut *REQUESTS.lock().unwrap())
}

// ---------------------------------------------------------------- world side -> rules: the player (fix list 1)

/// Minecraft damage to the local player the world side measured (falls,
/// drowning, lava, fire, starving), in BO2 health units (10 = one heart).
/// The rules side puts it through BO2's player damage, so last stand and
/// game over follow. (Healing from food goes as `Ask::Heal`, armor as
/// `Ask::Armor`.)
#[derive(Clone, Debug)]
pub enum PlayerEvent {
    /// `cause`: "fall", "drown", "lava", "in_fire", "on_fire", "starve".
    Damage { amount: i32, cause: &'static str },
}

static PLAYER_EVENTS: Mutex<Vec<PlayerEvent>> = Mutex::new(Vec::new());

pub fn push_player_event(event: PlayerEvent) {
    PLAYER_EVENTS.lock().unwrap().push(event);
}

pub fn take_player_events() -> Vec<PlayerEvent> {
    std::mem::take(&mut *PLAYER_EVENTS.lock().unwrap())
}

/// The inventory's character window (playtest 1): where his BO2 body
/// stands while the inventory is open (map point at its feet, facing yaw in
/// degrees), from the world side each frame; None when it is shut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puppet {
    pub origin: [f32; 3],
    pub yaw: f32,
}

static PUPPET: Mutex<Option<Puppet>> = Mutex::new(None);
/// The scene number the rules side's body model goes out as (the world side
/// draws that one in front of the world, as a view model).
static PUPPET_NUMBER: Mutex<Option<u32>> = Mutex::new(None);

pub fn set_puppet(p: Option<Puppet>) {
    *PUPPET.lock().unwrap() = p;
}

pub fn puppet() -> Option<Puppet> {
    *PUPPET.lock().unwrap()
}

pub fn set_puppet_number(n: Option<u32>) {
    *PUPPET_NUMBER.lock().unwrap() = n;
}

pub fn puppet_number() -> Option<u32> {
    *PUPPET_NUMBER.lock().unwrap()
}

static FOOD: Mutex<(f32, f32)> = Mutex::new((20.0, 5.0));

/// The hunger bar (each frame): food level 0..20 and saturation.
pub fn set_food(level: f32, saturation: f32) {
    *FOOD.lock().unwrap() = (level, saturation);
}

pub fn food() -> (f32, f32) {
    *FOOD.lock().unwrap()
}

