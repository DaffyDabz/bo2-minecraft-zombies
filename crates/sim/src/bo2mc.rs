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
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
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
    /// Doors side by side along the wall (+x from `cell` on a dz wall).
    pub width: i32,
}

impl Door {
    /// Its doors' lower halves (cell offsets), from `cell` along the wall.
    pub fn cells(&self) -> Vec<[i32; 3]> {
        let along = [self.outward[1].abs(), self.outward[0].abs()];
        (0..self.width).map(|i| [self.cell[0] + along[0] * i, self.cell[1], self.cell[2] + along[1] * i]).collect()
    }

    /// Whether door `i`'s hinge is its left (Minecraft's `hinge`, to the
    /// left of its facing, `outward`): a pair hinges on its outer sides.
    pub fn hinge_left(&self, i: i32) -> bool {
        if self.width < 2 {
            return true;
        }
        // The facing's left (counter-clockwise), along the wall.
        let left = self.outward[1] * self.outward[1].abs() - self.outward[0] * self.outward[0].abs();
        let outer = if i == 0 { -1 } else { 1 };
        left == outer
    }

    /// The map point between its doors at the floor, `cells` out.
    pub fn front(&self, cells: i32) -> [f32; 3] {
        let along = [self.outward[1].abs(), self.outward[0].abs()];
        let half = (self.width - 1) as f32 * 0.5;
        let dx = (self.cell[0] + self.outward[0] * cells) as f32 + along[0] as f32 * half;
        let dz = (self.cell[2] + self.outward[1] * cells) as f32 + along[1] as f32 * half;
        [dx * B, -dz * B, 0.0]
    }
}

/// Two doors, centred on the long walls (the dz walls). His 10-08: "Maybe
/// we should make those double doors": two wide, and open they take no
/// room (`bo2mc_world::set_blocks`), so nobody gets stuck in one.
pub const DOORS: [Door; 2] = [
    Door { cell: [0, 0, -(ROOM_HALF_Z + 1)], outward: [0, -1], width: 2 },
    Door { cell: [0, 0, ROOM_HALF_Z + 1], outward: [0, 1], width: 2 },
];

/// Zombies never break the room's doors (his ask 10-07): they are his to
/// open. Is absolute block `b` one of their halves?
pub fn zombie_proof(b: [i32; 3]) -> bool {
    DOORS.iter().chain([&VAULT_DOOR]).flat_map(|d| d.cells()).any(|cell| {
        room_cell_block(cell).is_some_and(|c| b == c || b == [c[0], c[1] + 1, c[2]])
    })
}

/// One of the room's windows, the zombies' way in (his ask 10-07): three
/// cells wide and two high over a one-block sill in a long wall, boarded
/// with six oak planks (BO2's six boards). A zombie tears them off one at
/// a time (`BOARD_SECONDS` each) and climbs through; he holds use inside
/// to nail them back, `BOARD_POINTS` a board, as in BO2.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    /// The middle cell's (dx, dz) in the wall ring.
    pub mid: [i32; 2],
    /// Into the room (cells dx, dz).
    pub inward: [i32; 2],
}

/// One window in each long wall, clear of the doors, the box and
/// Pack-a-Punch.
pub const WINDOWS: [Window; 2] = [
    Window { mid: [4, -(ROOM_HALF_Z + 1)], inward: [0, 1] },
    Window { mid: [-4, ROOM_HALF_Z + 1], inward: [0, -1] },
];
pub const WINDOW_BOARDS: usize = 6;
/// Seconds one zombie needs to tear a board off.
pub const BOARD_SECONDS: f32 = 1.5;
/// BO2's points for a board nailed back.
pub const BOARD_POINTS: i32 = 10;

impl Window {
    /// The six board cells (dx, dy, dz), in the order he nails them back:
    /// the bottom row, then the top.
    pub fn boards(&self) -> [[i32; 3]; WINDOW_BOARDS] {
        let along = [self.inward[1].abs(), self.inward[0].abs()];
        let mut out = [[0; 3]; WINDOW_BOARDS];
        let mut i = 0;
        for dy in 1..=2 {
            for k in -1..=1 {
                out[i] = [self.mid[0] + k * along[0], dy, self.mid[1] + k * along[1]];
                i += 1;
            }
        }
        out
    }

    /// Where he stands to nail the boards back: the floor inside it.
    pub fn inside(&self) -> [f32; 3] {
        cell_floor(self.mid[0] + self.inward[0], self.mid[1] + self.inward[1])
    }
}

/// Is room cell offset `c` a window's board cell? (window, board)
pub fn window_board(c: [i32; 3]) -> Option<(usize, usize)> {
    WINDOWS.iter().enumerate().find_map(|(w, win)| win.boards().iter().position(|b| *b == c).map(|i| (w, i)))
}

/// `window_board` for absolute block `b`.
pub fn window_board_block(b: [i32; 3]) -> Option<(usize, usize)> {
    let o = room_cell_block([0, 0, 0])?;
    window_board([b[0] - o[0], b[1] - o[1], b[2] - o[2]])
}

/// The boards up in each window, as the world side last counted them.
static BOARDS_UP: Mutex<Vec<u8>> = Mutex::new(Vec::new());

pub fn set_boards_up(counts: Vec<u8>) {
    *BOARDS_UP.lock().unwrap() = counts;
}

pub fn boards_up(window: usize) -> Option<u8> {
    BOARDS_UP.lock().unwrap().get(window).copied()
}

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

/// Pack-a-Punch: in the vault (his 10-07 ask), against its north wall's
/// middle, facing into it (map -y).
pub fn pap_spot() -> Spot {
    (vault_floor(0, -VAULT_HALF_Z), -90.0)
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

// ---------------------------------------------------------------- TranZit's bus (the arrival)
//
// The game starts on TranZit's bus, parked on a dirt road north of the
// house (past the yard, outside its north door). The driver stops, opens
// the doors, the players get off, the bus drives off east along the road.
// The house's doors stay locked until bought (`HOUSE_DOOR_COST`).

/// The arrival on the bus: on unless `IW4L_BO2MC_BUS=0`.
pub fn bus_on() -> bool {
    std::env::var("IW4L_BO2MC_BUS").map_or(true, |v| v != "0")
}

/// The road (cells): east-west, north of the yard; the bus leaves east.
pub const ROAD_X: std::ops::RangeInclusive<i32> = -24..=64;
pub const ROAD_Z: std::ops::RangeInclusive<i32> = -17..=-11;
/// The parked bus's footprint (cells). It is solid as the model's own
/// boxes (`t6::transit`, as TranZit's bus): his 10-08 "invisible blocks at
/// spawn" were the barrier blocks it stood in before, a block's grid that
/// stood 15 units off its sides and made each door a 36-unit gap beside
/// the 52-unit door he saw.
pub const BUS_X: std::ops::RangeInclusive<i32> = -9..=5;
pub const BUS_Z: std::ops::RangeInclusive<i32> = -16..=-12;
/// The bus's aisle floor over the road (`t6::transit::BUS_FLOOR`).
const BUS_FLOOR: f32 = 40.0;
/// The bus model's origin (map) parked: it faces +x (yaw 0), its doors
/// on its -y side, towards the house. His 10-08: "the front of the bus is
/// like pushed back": its windscreen (x 375 from the origin) sits on the
/// front wall's inside face (cell 5's, map x 162), its back wall (x -89)
/// on the back wall's (cell -9's, map x -306).
pub const BUS_ORIGIN: [f32; 3] = [-213.0, 505.5, 0.0];
/// What it costs to open the house (either door opens both).
pub const HOUSE_DOOR_COST: i32 = 1000;

/// Where the players start: standing in the bus, facing its doors.
pub fn bus_starts() -> Vec<Spot> {
    [[0, -13], [1, -13], [-1, -13], [-2, -13], [0, -14], [1, -14], [-1, -14], [-2, -14]]
        .iter()
        .map(|&[dx, dz]| {
            let p = cell_floor(dx, dz);
            // Just over its floor: a box he starts inside never holds him.
            ([p[0], p[1], BUS_FLOOR + 1.0], -90.0)
        })
        .collect()
}

/// Is map point `p` inside the parked bus (over its floor)?
pub fn in_bus(p: [f32; 3]) -> bool {
    let dx = (p[0] / B).round() as i32;
    let dz = (-p[1] / B).round() as i32;
    BUS_X.contains(&dx) && BUS_Z.contains(&dz) && p[2] > B * 0.5
}

/// The house's doors (both halves, absolute blocks) while locked: kept
/// from breaking and from opening until bought.
static LOCKED_DOORS: Mutex<Vec<[i32; 3]>> = Mutex::new(Vec::new());

/// The world side locks the house's doors (after `set_room_built`).
pub fn lock_doors(blocks: Vec<[i32; 3]>) {
    let mut p = PROTECTED.write().unwrap();
    let p = p.get_or_insert_with(HashSet::new);
    p.extend(blocks.iter().copied());
    *LOCKED_DOORS.lock().unwrap() = blocks;
}

pub fn doors_locked() -> bool {
    !LOCKED_DOORS.lock().unwrap().is_empty()
}

/// Bought: the doors open (zombies still never break them: `zombie_proof`).
pub fn unlock_doors() {
    HOUSE_BOUGHT.store(true, Ordering::Relaxed);
    let blocks = std::mem::take(&mut *LOCKED_DOORS.lock().unwrap());
    if let Some(p) = PROTECTED.write().unwrap().as_mut() {
        for b in blocks {
            p.remove(&b);
        }
    }
}

/// The house was bought this game (a return from the Nether finds it open).
static HOUSE_BOUGHT: AtomicBool = AtomicBool::new(false);

pub fn house_bought() -> bool {
    HOUSE_BOUGHT.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------- the vault
//
// His 10-07 ask: a second room across the road, on the far side of the bus,
// bought for `VAULT_DOOR_COST`: both portals in its middle, Pack-a-Punch,
// the other maps' boxes and the rare weapons for sale.

/// The vault's centre cell (dz; dx 0): north of the road.
pub const VAULT_Z: i32 = -29;
pub const VAULT_HALF_X: i32 = 8;
pub const VAULT_HALF_Z: i32 = 5;
/// Its one door: the middle of its south wall, facing the road.
pub const VAULT_DOOR: Door = Door { cell: [0, 0, VAULT_Z + VAULT_HALF_Z + 1], outward: [0, 1], width: 1 };
pub const VAULT_DOOR_COST: i32 = 3000;

/// The vault's lit Nether portal: its frame's west column dx, the frame
/// standing across x at `VAULT_Z` (4 wide, 5 tall, the portal 2 x 3).
pub const VAULT_NETHER_X: i32 = -6;
/// The vault's End portal frame: the ring's west dx and north dz (5 x 5,
/// corners empty), in the floor; twelve eyes open it.
pub const VAULT_END_X: i32 = 2;
pub const VAULT_END_Z: i32 = VAULT_Z - 2;

/// Map point at the floor of vault cell (dx, dz) (offsets from its centre).
pub fn vault_floor(dx: i32, dz: i32) -> [f32; 3] {
    cell_floor(dx, VAULT_Z + dz)
}

/// Is room cell offset (dx, dy, dz) inside the vault's air box?
pub fn vault_contains(c: [i32; 3]) -> bool {
    c[0].abs() <= VAULT_HALF_X && (c[2] - VAULT_Z).abs() <= VAULT_HALF_Z && (0..ROOM_HEIGHT).contains(&c[1])
}

/// The vault door's halves while locked.
static VAULT_LOCKED: Mutex<Vec<[i32; 3]>> = Mutex::new(Vec::new());
static VAULT_BOUGHT: AtomicBool = AtomicBool::new(false);

/// The world side locks the vault's door (after `set_room_built`).
pub fn lock_vault(blocks: Vec<[i32; 3]>) {
    let mut p = PROTECTED.write().unwrap();
    let p = p.get_or_insert_with(HashSet::new);
    p.extend(blocks.iter().copied());
    *VAULT_LOCKED.lock().unwrap() = blocks;
}

pub fn vault_locked() -> bool {
    !VAULT_LOCKED.lock().unwrap().is_empty()
}

pub fn unlock_vault() {
    VAULT_BOUGHT.store(true, Ordering::Relaxed);
    let blocks = std::mem::take(&mut *VAULT_LOCKED.lock().unwrap());
    if let Some(p) = PROTECTED.write().unwrap().as_mut() {
        for b in blocks {
            p.remove(&b);
        }
    }
}

pub fn vault_bought() -> bool {
    VAULT_BOUGHT.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------- the chalks and their signs
//
// His 10-07 asks: a Minecraft sign with the name and price above every
// chalk; the house keeps only the chalks that touch nothing (a window, a
// door, a perk machine's buy spot, another chalk), the rest go on the
// house's outside short walls (Nuketown's wall buys are outdoors); the
// vault sells the rare weapons, and coming-soon signs name what is not
// built yet.

/// Which wall a chalk or sign hangs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChalkWall {
    /// The house's north and south walls, inside (`at` = dx).
    HouseNorth,
    HouseSouth,
    /// The house's west and east walls, outside (`at` = dz).
    HouseWest,
    HouseEast,
    /// The vault's walls, inside (`at` = dz from `VAULT_Z` for west and
    /// east, dx for north).
    VaultWest,
    VaultEast,
    VaultNorth,
}

/// A wall buy (`weapon` set) or a coming-soon sign (`weapon` empty).
#[derive(Clone, Copy, Debug)]
pub struct Chalk {
    pub weapon: &'static str,
    pub name: &'static str,
    pub cost: i32,
    pub wall: ChalkWall,
    pub at: i32,
}

const fn chalk(weapon: &'static str, name: &'static str, cost: i32, wall: ChalkWall, at: i32) -> Chalk {
    Chalk { weapon, name, cost, wall, at }
}

/// Every wall buy (Nuketown's twelve, the frag grenades, the vault's rare
/// weapons) and every coming-soon sign. Costs are BO2's (the vault's are
/// his 10-07 picks); the game's own cost table still decides the price.
pub const CHALKS: [Chalk; 19] = {
    use ChalkWall::*;
    [
        chalk("rottweil72_zm", "Olympia", 500, HouseNorth, -2),
        chalk("m14_zm", "M14", 500, HouseSouth, 2),
        chalk("mp5k_zm", "MP5K", 1000, HouseSouth, 4),
        chalk("beretta93r_zm", "B23R", 1000, HouseSouth, 6),
        chalk("870mcs_zm", "870 MCS", 1500, HouseWest, -4),
        chalk("tazer_knuckles_zm", "Galvaknuckles", 6000, HouseWest, -1),
        chalk("bowie_knife_zm", "Bowie Knife", 3000, HouseWest, 2),
        chalk("ak74u_zm", "AK74u", 1200, HouseWest, 5),
        chalk("m16_zm", "M16", 1200, HouseEast, -4),
        chalk("claymore_zm", "Claymore", 1000, HouseEast, -1),
        chalk("sticky_grenade_zm", "Semtex", 250, HouseEast, 2),
        chalk("frag_grenade_zm", "Frag Grenade", 250, HouseEast, 5),
        chalk("blundergat_zm", "Blundergat", 10000, VaultWest, -4),
        // His 10-07: "a spork, and then there's also a golden spork" (BO2's
        // Spork is Mob of the Dead's spoon).
        chalk("spoon_zm_alcatraz", "Spork", 5000, VaultWest, 0),
        chalk("spork_zm_alcatraz", "Golden Spork", 15000, VaultWest, 4),
        chalk("", "Hell's Retriever", 20000, VaultEast, -3),
        chalk("", "Thunder Gun", 20000, VaultEast, 2),
        chalk("", "Mob of the Dead box", 950, VaultNorth, -5),
        chalk("", "Origins box", 950, VaultNorth, 5),
    ]
};

/// The bread on the house's north wall (his 10-08: "bread on the wall for a
/// hundred bucks for twelve"): its cell (dx, dz), between the box and the
/// corner perk machine, what it costs and how many loaves.
pub const BREAD_CELL: (i32, i32) = (-6, -ROOM_HALF_Z);
pub const BREAD_COST: i32 = 100;
pub const BREAD_COUNT: u32 = 12;

/// The chalk for `weapon`, if it has a wall.
pub fn chalk_of(weapon: &str) -> Option<&'static Chalk> {
    CHALKS.iter().find(|c| !c.weapon.is_empty() && c.weapon == weapon)
}

/// Height of a chalk's middle above the floor (his eye: the wall buy is
/// used by looking at it from right in front).
const CHALK_Z: f32 = 60.0;
/// The sign's row (cells above the floor): above the chalk.
const SIGN_DY: i32 = 3;

impl Chalk {
    /// Map point on the wall's face and the yaw facing away from the wall.
    pub fn spot(&self) -> ([f32; 3], f32) {
        let (hx, hz) = (ROOM_HALF_X as f32, ROOM_HALF_Z as f32);
        let vx = VAULT_HALF_X as f32;
        let at = self.at as f32;
        let (x, y, yaw) = match self.wall {
            ChalkWall::HouseNorth => (at * B, (hz + 0.5) * B - 1.0, -90.0),
            ChalkWall::HouseSouth => (at * B, -(hz + 0.5) * B + 1.0, 90.0),
            ChalkWall::HouseWest => (-(hx + 1.5) * B - 1.0, -at * B, 180.0),
            ChalkWall::HouseEast => ((hx + 1.5) * B + 1.0, -at * B, 0.0),
            ChalkWall::VaultWest => (-(vx + 0.5) * B + 1.0, -(VAULT_Z + self.at) as f32 * B, 0.0),
            ChalkWall::VaultEast => ((vx + 0.5) * B - 1.0, -(VAULT_Z + self.at) as f32 * B, 180.0),
            ChalkWall::VaultNorth => (at * B, -(VAULT_Z - VAULT_HALF_Z) as f32 * B + 0.5 * B - 1.0, -90.0),
        };
        let floor = cell_floor(0, 0)[2];
        ([x, y, floor + CHALK_Z], yaw)
    }

    /// The sign's cell (offset from the room's spawn cell) and the way its
    /// text faces (it hangs on the wall behind it).
    pub fn sign(&self) -> ([i32; 3], &'static str) {
        let (hx, hz, vx) = (ROOM_HALF_X, ROOM_HALF_Z, VAULT_HALF_X);
        // No chalk under it (a coming-soon sign): at his eye; over a chalk
        // (the vault's Blundergat and Spork have theirs too), above it.
        let dy = if self.weapon.is_empty() { 1 } else { SIGN_DY };
        match self.wall {
            ChalkWall::HouseNorth => ([self.at, dy, -hz], "south"),
            ChalkWall::HouseSouth => ([self.at, dy, hz], "north"),
            ChalkWall::HouseWest => ([-(hx + 2), dy, self.at], "west"),
            ChalkWall::HouseEast => ([hx + 2, dy, self.at], "east"),
            ChalkWall::VaultWest => ([-vx, dy, VAULT_Z + self.at], "east"),
            ChalkWall::VaultEast => ([vx, dy, VAULT_Z + self.at], "west"),
            ChalkWall::VaultNorth => ([self.at, dy, VAULT_Z - VAULT_HALF_Z], "south"),
        }
    }

    /// The sign's lines.
    pub fn lines(&self) -> Vec<String> {
        let cost = self.cost.to_string();
        if self.weapon.is_empty() {
            vec!["Coming soon".to_owned(), self.name.to_owned(), cost]
        } else {
            vec![self.name.to_owned(), cost]
        }
    }
}

// ---------------------------------------------------------------- the Nether and the End
//
// A portal takes the players to the Nether (and an End portal to the End).
// The house stands there too, around that dimension's spawn; there is no
// day or night there, so the waves never stop.

pub const OVERWORLD: u8 = 0;
pub const NETHER: u8 = 1;
pub const END: u8 = 2;

static DIMENSION: AtomicU8 = AtomicU8::new(OVERWORLD);

/// The dimension the players are in.
pub fn dimension() -> u8 {
    DIMENSION.load(Ordering::Relaxed)
}

pub fn dimension_name(d: u8) -> &'static str {
    match d {
        NETHER => "the Nether",
        END => "the End",
        _ => "the overworld",
    }
}

/// No day and no night (the Nether, the End): round after round, the
/// waves never wait.
pub fn endless() -> bool {
    dimension() != OVERWORLD
}

/// A trip through a portal: the room, its blocks and the spawn spots of the
/// dimension left behind are forgotten until the room stands in the new one.
pub fn travelled(to: u8) {
    let from = dimension();
    let room = Parked {
        protected: PROTECTED.write().unwrap().take(),
        locked: std::mem::take(&mut *LOCKED_DOORS.lock().unwrap()),
        vault: std::mem::take(&mut *VAULT_LOCKED.lock().unwrap()),
        built: std::mem::replace(&mut *ROOM_BUILT.lock().unwrap(), false),
        unbreakable: UNBREAKABLE.write().unwrap().take(),
        dig_cost: DIG_COST.write().unwrap().take(),
        spawns: std::mem::take(&mut *SPAWNS.write().unwrap()),
    };
    *UNDERGROUND.lock().unwrap() = false;
    // The overworld's house waits, as it was, for the way back; the
    // Nether's and the End's are left behind.
    if from == OVERWORLD {
        *PARKED.lock().unwrap() = Some(room);
    }
    if to == OVERWORLD
        && let Some(room) = PARKED.lock().unwrap().take()
    {
        *PROTECTED.write().unwrap() = room.protected;
        *LOCKED_DOORS.lock().unwrap() = room.locked;
        // Bought in the Nether or the End: the overworld's vault door opens too.
        if vault_bought() {
            if let Some(p) = PROTECTED.write().unwrap().as_mut() {
                for b in &room.vault {
                    p.remove(b);
                }
            }
        } else {
            *VAULT_LOCKED.lock().unwrap() = room.vault;
        }
        *ROOM_BUILT.lock().unwrap() = room.built;
        *UNBREAKABLE.write().unwrap() = room.unbreakable;
        *DIG_COST.write().unwrap() = room.dig_cost;
        *SPAWNS.write().unwrap() = room.spawns;
    }
    DIMENSION.store(to, Ordering::Relaxed);
}

/// The overworld's room as the players left it through a portal.
struct Parked {
    protected: Option<HashSet<[i32; 3]>>,
    locked: Vec<[i32; 3]>,
    vault: Vec<[i32; 3]>,
    built: bool,
    unbreakable: Option<HashSet<[i32; 3]>>,
    dig_cost: Option<HashMap<[i32; 3], f32>>,
    spawns: Vec<[f32; 3]>,
}

static PARKED: Mutex<Option<Parked>> = Mutex::new(None);

/// A world load ended: forget the room.
pub fn reset() {
    DIMENSION.store(OVERWORLD, Ordering::Relaxed);
    *PARKED.lock().unwrap() = None;
    HOUSE_BOUGHT.store(false, Ordering::Relaxed);
    VAULT_BOUGHT.store(false, Ordering::Relaxed);
    *PROTECTED.write().unwrap() = None;
    LOCKED_DOORS.lock().unwrap().clear();
    VAULT_LOCKED.lock().unwrap().clear();
    *ROOM_BUILT.lock().unwrap() = false;
    *UNBREAKABLE.write().unwrap() = None;
    *DIG_COST.write().unwrap() = None;
    *SPAWNS.write().unwrap() = Vec::new();
    *UNDERGROUND.lock().unwrap() = false;
    REQUESTS.lock().unwrap().clear();
    *PLAYER.write().unwrap() = None;
    ASKS.lock().unwrap().clear();
    PLAYER_EVENTS.lock().unwrap().clear();
    *FOOD.lock().unwrap() = (20.0, 5.0);
    SOULS_FOG.store(false, Ordering::Relaxed);
    *SOULS_WOLVES.lock().unwrap() = (0, None);
    GAME_MODE.store(SURVIVAL, Ordering::Relaxed);
    FLYING.store(false, Ordering::Relaxed);
}

// ---------------------------------------------------------------- game modes (the chat's /gamemode)

pub const SURVIVAL: u8 = 0;
pub const CREATIVE: u8 = 1;
pub const ADVENTURE: u8 = 2;
pub const SPECTATOR: u8 = 3;

static GAME_MODE: AtomicU8 = AtomicU8::new(SURVIVAL);
/// Creative's flight (a double tap of Jump turns it on and off).
static FLYING: AtomicBool = AtomicBool::new(false);
/// When Jump was last pressed, for the double tap (sim milliseconds).
static LAST_JUMP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(i32::MIN / 2);

/// Minecraft's game mode: survival (0), creative (1), adventure (2),
/// spectator (3). Survival whenever Minecraft Zombies is off.
pub fn game_mode() -> u8 {
    if enabled() { GAME_MODE.load(Ordering::Relaxed) } else { SURVIVAL }
}

pub fn set_game_mode(mode: u8) {
    GAME_MODE.store(mode.min(SPECTATOR), Ordering::Relaxed);
    // Spectators always fly; creative starts on the ground.
    FLYING.store(mode == SPECTATOR, Ordering::Relaxed);
}

/// Nothing hurts the player: creative and spectator.
pub fn invulnerable() -> bool {
    matches!(game_mode(), CREATIVE | SPECTATOR)
}

/// The player flies: a spectator always, creative after a double jump.
pub fn flying() -> bool {
    match game_mode() {
        SPECTATOR => true,
        CREATIVE => FLYING.load(Ordering::Relaxed),
        _ => false,
    }
}

pub fn set_flying(on: bool) {
    FLYING.store(on, Ordering::Relaxed);
}

static JUMP_HELD: AtomicBool = AtomicBool::new(false);

/// Creative: Jump is `down` at `time` (ms); a second press within 300 ms
/// turns flight on or off, as Minecraft's double tap does.
pub fn creative_jump(down: bool, time: i32) {
    let was = JUMP_HELD.swap(down, Ordering::Relaxed);
    if game_mode() != CREATIVE || !down || was {
        return;
    }
    let last = LAST_JUMP.swap(time, Ordering::Relaxed);
    if time.wrapping_sub(last) <= 300 {
        FLYING.store(!FLYING.load(Ordering::Relaxed), Ordering::Relaxed);
        LAST_JUMP.store(i32::MIN / 2, Ordering::Relaxed);
    }
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
    if zombie_proof(b) {
        return None;
    }
    if window_board_block(b).is_some() {
        return Some(BOARD_SECONDS);
    }
    if is_protected(b) || UNBREAKABLE.read().unwrap().as_ref().is_some_and(|u| u.contains(&b)) {
        return None;
    }
    Some(DIG_COST.read().unwrap().as_ref().and_then(|c| c.get(&b).copied()).unwrap_or(DIG_ESTIMATE_SECONDS))
}

/// Natural ground the world side has not priced (mostly dirt and stone).
pub const DIG_ESTIMATE_SECONDS: f32 = 3.0;

/// Claw seconds of the blocks the world side placed or changed (his
/// walls, the room's doors), so the zombies plan through the weakest way
/// in rather than straight through obsidian.
static DIG_COST: RwLock<Option<HashMap<[i32; 3], f32>>> = RwLock::new(None);

/// What block `b` now costs to claw through; None = no block priced there.
pub fn set_dig_cost(b: [i32; 3], seconds: Option<f32>) {
    let mut costs = DIG_COST.write().unwrap();
    match seconds {
        Some(s) => {
            costs.get_or_insert_with(HashMap::new).insert(b, s);
        }
        None => {
            if let Some(c) = costs.as_mut() {
                c.remove(&b);
            }
        }
    }
}

/// Seconds one zombie needs for a block of Minecraft hardness
/// `destroy_speed`: twice the hardness, three times for stone-like blocks
/// (`stone`, the pickaxe's). Leaves 0.5 s, dirt 1, planks and logs 4,
/// stone 4.5, cobblestone and bricks 6, deepslate 9, iron 15, obsidian 40;
/// bedrock (-1) never. A wooden door takes `DOOR_SECONDS`.
pub fn claw_seconds(destroy_speed: f32, stone: bool) -> Option<f32> {
    (destroy_speed >= 0.0).then(|| (destroy_speed * if stone { 3.0 } else { 2.0 }).clamp(0.5, 40.0))
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

/// The local player in Minecraft water: his body (swims) and his eyes (under).
static WATER: Mutex<(bool, bool)> = Mutex::new((false, false));

pub fn set_player_water(body: bool, eye: bool) {
    *WATER.lock().unwrap() = (body, eye);
}

pub fn player_water() -> (bool, bool) {
    *WATER.lock().unwrap()
}

/// Minecraft swimming on BO2's movement (his ask 10-07: "we should be able to
/// swim"): water slows him to half speed, he sinks slowly, Jump swims up,
/// Crouch dives, the look and move keys steer him; at the surface Jump keeps
/// him afloat and Jump with Forward hops him out onto the bank.
pub const SWIM_GRAVITY: i32 = 120;
pub const SWIM_SPEED: f32 = 95.0;
pub const SWIM_UP: f32 = 110.0;
pub const SWIM_SINK: f32 = 60.0;
pub const SWIM_HOP: f32 = 300.0;

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
    perks: Vec<String>,
}

static PLAYER: RwLock<Option<PlayerFacts>> = RwLock::new(None);

/// The rules side, each tick: his BO2 health and weapons.
pub fn set_player(health: i32, max_health: i32, weapons: Vec<WeaponItem>, perks: Vec<String>) {
    *PLAYER.write().unwrap() = Some(PlayerFacts {
        health,
        max_health,
        weapons,
        perks,
    });
}

/// Whether he has this BO2 perk (`specialty_rof`). His 10-08: "make sure
/// every single perk works. And it works in a way that works with
/// Minecraft": the world side reads them (Speed Cola mines and places
/// faster, Double Tap swings faster, Deadshot's swings crit, Stamin-Up
/// sprints without the hunger, Quick Revive heals twice as fast, PhD Flopper
/// takes no fall or creeper damage, Vulture Aid loots, Electric Cherry
/// shocks the mobs too).
pub fn player_has_perk(perk: &str) -> bool {
    PLAYER.read().unwrap().as_ref().is_some_and(|p| p.perks.iter().any(|q| q == perk))
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
    /// He swung a Minecraft item (a sword, a tool, his fist): its attack
    /// damage (Minecraft points, after the attack cooldown) lands on the
    /// zombie he looks at within `reach` (map units).
    Swing { damage: f32, reach: f32 },
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
    /// The bus drove off: its barrier blocks go.
    BusGone,
    /// The house was bought at door `door` (of `DOORS`): it swings open.
    OpenDoor { door: usize },
    /// The vault was bought: its door swings open.
    OpenVault,
    /// An item dropped in the world at block `at` (a Nether zombie's blaze
    /// rod).
    Drop { item: String, count: u32, at: [i32; 3] },
    /// The souls round: this many angry wolves around the player.
    SoulsWolves { count: u32 },
    /// A Minecraft lightning bolt at map point `at` in `delay` seconds (a
    /// hellhound's arrival).
    Lightning { at: [f32; 3], delay: f32 },
    /// He nailed a board back into window `window` (of `WINDOWS`).
    Rebuild { window: usize },
    /// A Minecraft sound event at map point `at` in place of a BO2 alias
    /// no zone carries (the hellhounds' bark is a wolf's growl).
    Sound { event: String, at: [f32; 3], volume: f32, pitch: f32 },
    /// Electric Cherry's reload shock at map point `at`: the Minecraft mobs
    /// within `radius` map units take `damage` Minecraft points.
    Shock { at: [f32; 3], radius: f32, damage: f32 },
}

// ------------------------------------------------------------ souls rounds

static SOULS_FOG: AtomicBool = AtomicBool::new(false);
/// The souls round's wolves the world side sees alive, and where the last
/// one died (a map point, taken once).
static SOULS_WOLVES: Mutex<(usize, Option<[f32; 3]>)> = Mutex::new((0, None));

/// A souls round is on: the world side thickens the fog.
pub fn set_souls_fog(on: bool) {
    SOULS_FOG.store(on, Ordering::Relaxed);
}

pub fn souls_fog() -> bool {
    SOULS_FOG.load(Ordering::Relaxed)
}

pub fn set_souls_wolves(alive: usize, died_at: Option<[f32; 3]>) {
    let mut s = SOULS_WOLVES.lock().unwrap();
    s.0 = alive;
    if died_at.is_some() {
        s.1 = died_at;
    }
}

pub fn souls_wolves() -> usize {
    SOULS_WOLVES.lock().unwrap().0
}

pub fn take_souls_wolf_death() -> Option<[f32; 3]> {
    SOULS_WOLVES.lock().unwrap().1.take()
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
    /// The chat's /kill.
    Kill,
    /// The chat's /tp: to this map point (feet).
    Teleport { to: [f32; 3] },
    /// The chat's /round: round n, now.
    Round(i32),
    /// The chat's /points: this many more (or, with `set`, exactly this many).
    Points { amount: i32, set: bool },
    /// The chat moved the clock (/time set, /time add): between rounds the
    /// next night is the one it lands in, no morning first.
    TimeMoved,
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

