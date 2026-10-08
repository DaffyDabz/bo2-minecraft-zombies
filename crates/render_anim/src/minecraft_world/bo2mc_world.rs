//! Minecraft Zombies, the world side of `sim::bo2mc`: the spawn room built
//! in blocks around the world spawn (its walls, floor and roof kept from
//! being broken), and the requests the Black Ops II rules make of the world.
use std::collections::{BTreeMap, HashMap, HashSet};

use glam::DVec3;

use minecraft_terrain::scene::Block;
use minecraft_terrain::sign_render::{SignFacing, WallSign};
use sim::bo2mc::{DOORS, ROOM_HALF_X, ROOM_HALF_Z, ROOM_HEIGHT};

use super::{Loaded, ShapeCache};

mod chat;
use crate::minecraft_entities::Entities;

pub(super) type BlockPos = (i32, i32, i32);

/// Blocks of flattened yard around the room's walls.
const YARD: i32 = 3;
/// How far below the floor the ground is filled to reach solid ground.
const FILL_DEPTH: i32 = 32;
/// How high above the floor the room's footprint and yard are cleared.
const CLEAR_HEIGHT: i32 = 64;
/// Ceiling lights: every four cells across the roof.
const LIGHT_SPACING: i32 = 4;

const FLOOR: &str = "minecraft:polished_andesite";
const WALL: &str = "minecraft:stone_bricks";
const LIGHT: &str = "minecraft:glowstone";
const DOOR: &str = "minecraft:oak_door";
/// A window's boards (`sim::bo2mc::WINDOWS`): glass (his 10-07 ask), which
/// breaks with Minecraft's glass smash.
const BOARD: &str = "minecraft:glass";

/// The world side's own state for one world load.
#[derive(Default)]
pub(super) struct Bo2mcWorld {
    /// The spawn room stands.
    pub built: bool,
    /// Where it stands (the world spawn block), once built.
    room_at: Option<BlockPos>,
    /// The first spawn put the player in the bus (TranZit's arrival).
    pub arrived: bool,
    /// A SetDay playing out: from, to (clock ticks) and seconds so far.
    day_move: Option<(f64, f64, f64)>,
    /// The notice on screen and its seconds left.
    notice: Option<(String, f32)>,
    use_was_down: bool,
    underground: bool,
    underground_clock: f64,
    spawns_clock: f64,
    /// When each block being clawed last sounded.
    claw_sounds: HashMap<BlockPos, f64>,
    /// Seconds since the room stood (the world test's clock) and to the
    /// next state line in the log.
    test_clock: f64,
    log_clock: f64,
    /// The clock last frame and when dawn came, to log the day's length.
    last_tod: Option<f64>,
    dawn_at: Option<f64>,
    /// Survival: hunger, air, fire, the fall, eating.
    vitals: Vitals,
    /// The item he picked last, as told to the rules side (a BO2 weapon's
    /// name, None for a Minecraft item).
    pub picked: Option<Option<String>>,
    /// The gun BO2 had up last frame, and when each pick was asked for
    /// (playtest 1b: a gun BO2 raises itself, as after a buy, takes the
    /// hotbar selection; one he picked does not bounce it back).
    pub held_seen: Option<String>,
    pub asked: Vec<(String, f64)>,
    /// Survival (playtest 1): each placed furnace's and chest's contents,
    /// the one whose screen is open, and the furnaces' tick clock.
    pub furnaces: HashMap<BlockPos, minecraftoss_player::furnace::Furnace>,
    pub chests: HashMap<BlockPos, minecraftoss_player::chest::Chest>,
    pub open_container: Option<BlockPos>,
    furnace_clock: f64,
    /// The armor last told to the rules side.
    armor_told: Option<(u8, f32)>,
    spawn_count: usize,
    /// A portal trip asked for (the dimension), and the dimension to load
    /// ahead; seconds stood in a Nether portal.
    pub travel: Option<u8>,
    pub preload: Option<u8>,
    portal_clock: f64,
    /// The souls round's wolves alive last frame, and where.
    souls_wolves: HashMap<u64, [f64; 3]>,
    /// Each souls wolf's closest approach to him in blocks, when it last
    /// got a block closer, and when it was last at him.
    wolf_progress: HashMap<u64, (f64, f64, f64)>,
    /// Whether the souls fog was in last frame (its wolves die as it lifts).
    souls_fog_was: bool,
    /// Seconds to the souls round's next storm bolt.
    storm: f64,
    /// Oak wall signs standing in this world, by cell (`place_sign`).
    /// Drawn with the mobs each frame, so no remesh loses them.
    signs: BTreeMap<BlockPos, WallSign>,
    /// The dimensions (bits) whose house has its bread hung on the wall.
    bread_shown: u8,
    /// Minecraft's compass pictures, stacked (None: not read yet; Some(None):
    /// the pack has none).
    compass_frames: Option<Option<std::sync::Arc<Vec<u8>>>>,
}

/// Seconds in a Nether portal before it takes him (vanilla's 80 ticks).
const PORTAL_SECONDS: f64 = 4.0;

/// The facing of a door whose outward direction is `outward` (cells).
fn facing(outward: [i32; 2]) -> &'static str {
    match outward {
        [0, -1] => "north",
        [0, 1] => "south",
        [1, 0] => "east",
        _ => "west",
    }
}

/// An oak door's half (`upper`) facing `facing`, hinged on its left or
/// right.
pub(super) fn door(facing: &str, upper: bool, open: bool, hinge_left: bool) -> Block {
    Block::new(DOOR)
        .with("facing", facing)
        .with("half", if upper { "upper" } else { "lower" })
        .with("hinge", if hinge_left { "left" } else { "right" })
        .with("open", if open { "true" } else { "false" })
        .with("powered", "false")
}

/// What the room has at cell offset (dx, dy, dz) for dy >= 0, inside its
/// footprint (walls included): a wall, roof or door block, or air.
fn room_cell(dx: i32, dy: i32, dz: i32) -> Option<Block> {
    let (wall_x, wall_z) = (ROOM_HALF_X + 1, ROOM_HALF_Z + 1);
    if dy > ROOM_HEIGHT {
        return None;
    }
    if dy == ROOM_HEIGHT {
        let light = dx.rem_euclid(LIGHT_SPACING) == 0
            && dz.rem_euclid(LIGHT_SPACING) == 0
            && dx.abs() < wall_x
            && dz.abs() < wall_z;
        return Some(Block::new(if light { LIGHT } else { WALL }));
    }
    for d in DOORS {
        for (i, c) in d.cells().into_iter().enumerate() {
            if c[0] == dx && c[2] == dz && (dy == c[1] || dy == c[1] + 1) {
                return Some(door(facing(d.outward), dy != c[1], false, d.hinge_left(i as i32)));
            }
        }
    }
    if sim::bo2mc::window_board([dx, dy, dz]).is_some() {
        return Some(Block::new(BOARD));
    }
    if dx.abs() == wall_x || dz.abs() == wall_z {
        return Some(Block::new(WALL));
    }
    None
}

/// What the vault has at cell (dx, dy, rel) for dy >= 0, `rel` = dz from
/// its centre row (`sim::bo2mc::VAULT_Z`): wall, roof, light, door, the
/// Nether portal's frame and portal (the Overworld only), or air.
fn vault_cell(dx: i32, dy: i32, rel: i32) -> Option<Block> {
    use sim::bo2mc::{VAULT_DOOR, VAULT_HALF_X, VAULT_HALF_Z, VAULT_NETHER_X, VAULT_Z};
    let (wall_x, wall_z) = (VAULT_HALF_X + 1, VAULT_HALF_Z + 1);
    if dy > ROOM_HEIGHT {
        return None;
    }
    if dy == ROOM_HEIGHT {
        let light = dx.rem_euclid(LIGHT_SPACING) == 0 && rel.rem_euclid(LIGHT_SPACING) == 0 && dx.abs() < wall_x && rel.abs() < wall_z;
        return Some(Block::new(if light { LIGHT } else { WALL }));
    }
    let d = VAULT_DOOR;
    if d.cell[0] == dx && d.cell[2] - VAULT_Z == rel && (dy == 0 || dy == 1) {
        return Some(door(facing(d.outward), dy == 1, sim::bo2mc::vault_bought(), true));
    }
    if dx.abs() == wall_x || rel.abs() == wall_z {
        return Some(Block::new(WALL));
    }
    let overworld = sim::bo2mc::dimension() == sim::bo2mc::OVERWORLD;
    if overworld && rel == 0 && (VAULT_NETHER_X..=VAULT_NETHER_X + 3).contains(&dx) && dy <= 3 {
        let frame = dx == VAULT_NETHER_X || dx == VAULT_NETHER_X + 3 || dy == 3;
        return Some(if frame { Block::new("minecraft:obsidian") } else { Block::new("minecraft:nether_portal").with("axis", "x") });
    }
    None
}

/// The vault's floor layer (dy -1) at (dx, rel): the floor, the Nether
/// portal frame's foot, the End portal frame's ring (twelve empty frames,
/// facing in) or the open pit inside it (None = air).
fn vault_floor_cell(dx: i32, rel: i32) -> Option<Block> {
    use sim::bo2mc::{VAULT_END_X, VAULT_END_Z, VAULT_NETHER_X, VAULT_Z};
    if sim::bo2mc::dimension() == sim::bo2mc::OVERWORLD {
        if rel == 0 && (VAULT_NETHER_X..=VAULT_NETHER_X + 3).contains(&dx) {
            return Some(Block::new("minecraft:obsidian"));
        }
        let (ex, ez) = (dx - VAULT_END_X, rel + VAULT_Z - VAULT_END_Z);
        if (0..=4).contains(&ex) && (0..=4).contains(&ez) {
            let edge_x = ex == 0 || ex == 4;
            let edge_z = ez == 0 || ez == 4;
            if edge_x && edge_z {
                return Some(Block::new(FLOOR));
            }
            if !edge_x && !edge_z {
                return None;
            }
            let facing = match (ex, ez) {
                (0, _) => "east",
                (4, _) => "west",
                (_, 0) => "south",
                _ => "north",
            };
            return Some(Block::new("minecraft:end_portal_frame").with("eye", "false").with("facing", facing));
        }
    }
    Some(Block::new(FLOOR))
}

/// The vault's blocks (his 10-07 ask: across the road, on the far side of
/// the bus): its walls, floor, roof and lights, its door, a lit Nether
/// portal and an empty End portal frame in its middle (the Overworld's),
/// the ground filled under it and a yard cleared round it to the road;
/// with no road (the Nether, the End, no bus), a passage from the house's
/// north door instead.
fn vault_edits(
    world: &Loaded,
    spawn: BlockPos,
    (top_fill, under_fill): (&str, &str),
    clear_height: i32,
    edits: &mut Vec<(BlockPos, Option<Block>)>,
    protected: &mut HashSet<[i32; 3]>,
) {
    use sim::bo2mc::{VAULT_HALF_X, VAULT_HALF_Z, VAULT_Z};
    let range = world.stream.states.vertical_range();
    let (wall_x, wall_z) = (VAULT_HALF_X + 1, VAULT_HALF_Z + 1);
    let yard = 2;
    let road = sim::bo2mc::dimension() == sim::bo2mc::OVERWORLD && sim::bo2mc::bus_on();
    let south = if road { *sim::bo2mc::ROAD_Z.start() - 1 } else { VAULT_Z + wall_z + yard };
    let mut cells: Vec<(i32, i32)> = (-(wall_x + yard)..=wall_x + yard)
        .flat_map(|dx| (VAULT_Z - wall_z - yard..=south).map(move |dz| (dx, dz)))
        .collect();
    if !road {
        cells.extend((-1..=1).flat_map(|dx| (south + 1..=-(ROOM_HALF_Z + 2)).map(move |dz| (dx, dz))));
    }
    for (dx, dz) in cells {
        let rel = dz - VAULT_Z;
        let inside = dx.abs() <= wall_x && rel.abs() <= wall_z;
        let height = if dz > south { 2 } else { clear_height };
        for dy in 0..=height {
            let pos = (spawn.0 + dx, spawn.1 + dy, spawn.2 + dz);
            if !range.contains(&pos.1) {
                break;
            }
            let want = if inside { vault_cell(dx, dy, rel) } else { None };
            if let Some(block) = &want {
                if block.id.path != "oak_door" {
                    protected.insert([pos.0, pos.1, pos.2]);
                }
                edits.push((pos, want));
            } else if !is_air(world, pos) {
                edits.push((pos, None));
            }
        }
        for dy in (-FILL_DEPTH..=-1).rev() {
            let pos = (spawn.0 + dx, spawn.1 + dy, spawn.2 + dz);
            if !range.contains(&pos.1) {
                break;
            }
            if inside && dy == -1 {
                let block = vault_floor_cell(dx, rel);
                if block.is_some() {
                    protected.insert([pos.0, pos.1, pos.2]);
                }
                edits.push((pos, block));
                continue;
            }
            if is_ground(world, pos) {
                break;
            }
            let fill = match (inside, dy) {
                (false, -1) => top_fill,
                (false, _) => under_fill,
                (true, _) => "minecraft:stone",
            };
            edits.push((pos, Some(Block::new(fill))));
        }
    }
}

/// Whether a block is ground the fill can stop on: a full collision block,
/// not leaves or a log (a tree is not the ground).
fn is_ground(world: &Loaded, pos: BlockPos) -> bool {
    let Some(state) = world.scene.state_at(pos) else {
        return false;
    };
    let blocks = &world.registries.blocks;
    let info = blocks.state(state);
    if !info.collision_full_block || info.fluid.is_some() {
        return false;
    }
    world.stream.states.block(state).is_none_or(|b| {
        let path = b.id.path.as_str();
        !(path.ends_with("_leaves") || path.ends_with("_log") || path.ends_with("_wood"))
    })
}

/// A souls wolf closer than this (blocks) to him is never stuck.
const WOLF_NEAR: f64 = 3.0;
/// Seconds a souls wolf may go without getting a block closer to him.
const WOLF_STUCK_SECS: f64 = 15.0;
/// Seconds a souls wolf may go without reaching him at all (one that
/// creeps closer along a wall never stops making progress).
const WOLF_LATE_SECS: f64 = 30.0;

/// A spot three to six blocks from his feet where a wolf can stand (air
/// for feet and head, solid ground, no fluid) with open air all the way
/// back to him, so it lands on his side of every wall. `seed` picks where
/// round the ring the search starts.
fn wolf_landing(world: &Loaded, feet: [f64; 3], seed: u32) -> Option<[f64; 3]> {
    let blocks = &world.registries.blocks;
    let solid = |pos: BlockPos| world.scene.state_at(pos).is_some_and(|st| !blocks.is_air(st) && blocks.state(st).fluid.is_none());
    let block = |p: DVec3| (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    let from = DVec3::from_array(feet);
    const TURNS: u32 = 16;
    for radius in [5.0, 4.0, 6.0, 3.0, 2.0] {
        for k in 0..TURNS {
            let a = f64::from((seed + k) % TURNS) / f64::from(TURNS) * std::f64::consts::TAU;
            for dy in [0, 1, -1] {
                let at = from + DVec3::new(a.cos() * radius, f64::from(dy), a.sin() * radius);
                let (x, y, z) = block(at);
                if !(is_air(world, (x, y, z)) && is_air(world, (x, y + 1, z)) && solid((x, y - 1, z))) {
                    continue;
                }
                let to = DVec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5);
                let steps = (from.distance(to) * 3.0).ceil() as i32;
                let open = (1..steps).all(|i| {
                    let p = from.lerp(to, f64::from(i) / f64::from(steps));
                    let (bx, by, bz) = block(p + DVec3::new(0.0, 0.5, 0.0));
                    swimmable(world, (bx, by, bz)) && swimmable(world, (bx, by + 1, bz))
                });
                if open {
                    return Some(to.to_array());
                }
            }
        }
    }
    None
}

/// Whether a wolf can pass the block at `pos`: air, or water it swims.
fn swimmable(world: &Loaded, pos: BlockPos) -> bool {
    is_air(world, pos)
        || world.scene.state_at(pos).is_some_and(|state| {
            let blocks = &world.registries.blocks;
            blocks.state(state).fluid.as_ref().is_some_and(|fl| fl.kind == minecraftoss_core::block::FluidKind::Water)
        })
}

/// Whether the block at `pos` is air (no block, no fluid).
fn is_air(world: &Loaded, pos: BlockPos) -> bool {
    world.scene.state_at(pos).is_none_or(|state| {
        let blocks = &world.registries.blocks;
        blocks.is_air(state) && blocks.state(state).fluid.is_none()
    })
}

/// Sets blocks as one edit: the scene, collision, the section meshes and
/// light, the stored chunks and the mobs' level.
pub(super) fn set_blocks(
    world: &mut Loaded,
    shapes: &mut ShapeCache,
    mut entities: Option<&mut Entities>,
    edits: Vec<(BlockPos, Option<Block>)>,
) {
    if edits.is_empty() {
        return;
    }
    let mut positions = Vec::with_capacity(edits.len());
    for (pos, block) in edits {
        // An open door takes no room (his 10-08: "when you open them, make
        // it so they're completely walked through, like you can't get
        // stuck in them").
        let open_door = block.as_ref().is_some_and(|b| is_door(b) && b.properties.get("open").is_some_and(|o| o == "true"));
        let shape = block
            .as_ref()
            .filter(|_| !open_door)
            .and_then(|b| world.stream.states.state_of(b))
            .map_or(0, |state| shapes.shape_id(&world.registries, state));
        let placed = block.is_some();
        note_dig_cost(world, pos, block.as_ref());
        world.scene.set(pos, block);
        sim::voxel::set_block_shape(pos.0, pos.1, pos.2, shape);
        if let Some(entities) = entities.as_deref_mut() {
            if placed {
                entities.placed(&world.scene, pos);
            } else {
                entities.broke(&world.scene, &[pos]);
            }
        }
        positions.push(pos);
    }
    world.stream.record_edits(&world.scene, &positions);
    world.stream.mark_edited(&world.scene, &positions);
}

impl Bo2mcWorld {
    /// Builds the spawn room around the world spawn block `spawn` (the
    /// first air cell above the ground at map origin): clears its footprint
    /// and yard, fills the ground under them, lays the floor, walls, roof,
    /// ceiling lights and the two doors, and publishes the protected blocks.
    pub(super) fn build_room(
        &mut self,
        world: &mut Loaded,
        shapes: &mut ShapeCache,
        mut entities: Option<&mut Entities>,
        spawn: BlockPos,
    ) {
        let (wall_x, wall_z) = (ROOM_HALF_X + 1, ROOM_HALF_Z + 1);
        let (yard_x, yard_z) = (wall_x + YARD, wall_z + YARD);
        let mut edits = Vec::new();
        let mut protected = HashSet::new();
        let range = world.stream.states.vertical_range();
        // The Nether's and the End's house: a pocket cut in the rock under
        // the Nether's roof, and ground of the dimension's own stone.
        let dimension = sim::bo2mc::dimension();
        let clear_height = if dimension == sim::bo2mc::OVERWORLD { CLEAR_HEIGHT } else { ROOM_HEIGHT + 3 };
        let (top_fill, under_fill) = match dimension {
            sim::bo2mc::NETHER => ("minecraft:netherrack", "minecraft:netherrack"),
            sim::bo2mc::END => ("minecraft:end_stone", "minecraft:end_stone"),
            _ => ("minecraft:grass_block", "minecraft:dirt"),
        };
        for dx in -yard_x..=yard_x {
            for dz in -yard_z..=yard_z {
                let in_room = dx.abs() <= wall_x && dz.abs() <= wall_z;
                // Above the floor: the room, or the yard's open air.
                for dy in 0..=clear_height {
                    let pos = (spawn.0 + dx, spawn.1 + dy, spawn.2 + dz);
                    if !range.contains(&pos.1) {
                        break;
                    }
                    let want = if in_room { room_cell(dx, dy, dz) } else { None };
                    if let Some(block) = &want {
                        if block.id.path != "oak_door" {
                            protected.insert([pos.0, pos.1, pos.2]);
                        }
                        edits.push((pos, want));
                    } else if !is_air(world, pos) {
                        edits.push((pos, None));
                    }
                }
                // The floor, and the ground under the room and the yard
                // filled down to solid ground.
                for dy in (-FILL_DEPTH..=-1).rev() {
                    let pos = (spawn.0 + dx, spawn.1 + dy, spawn.2 + dz);
                    if !range.contains(&pos.1) {
                        break;
                    }
                    if in_room && dy == -1 && dx == 0 && dz == 0 && farm_kit() {
                        edits.push((pos, Some(Block::new("minecraft:grass_block"))));
                        continue;
                    }
                    if in_room && dy == -1 {
                        protected.insert([pos.0, pos.1, pos.2]);
                        edits.push((pos, Some(Block::new(FLOOR))));
                        continue;
                    }
                    if is_ground(world, pos) {
                        break;
                    }
                    let fill = match (in_room, dy) {
                        (false, -1) => top_fill,
                        (false, _) => under_fill,
                        (true, _) => "minecraft:stone",
                    };
                    edits.push((pos, Some(Block::new(fill))));
                }
            }
        }
        // TranZit's arrival: a dirt road north of the yard (the parked
        // bus on it is solid in the sim, as its model's own boxes).
        if sim::bo2mc::bus_on() && dimension == sim::bo2mc::OVERWORLD {
            for dx in sim::bo2mc::ROAD_X {
                for dz in sim::bo2mc::ROAD_Z {
                    for dy in 0..=CLEAR_HEIGHT {
                        let pos = (spawn.0 + dx, spawn.1 + dy, spawn.2 + dz);
                        if !range.contains(&pos.1) {
                            break;
                        }
                        if !is_air(world, pos) {
                            edits.push((pos, None));
                        }
                    }
                    for dy in (-FILL_DEPTH..=-1).rev() {
                        let pos = (spawn.0 + dx, spawn.1 + dy, spawn.2 + dz);
                        if !range.contains(&pos.1) {
                            break;
                        }
                        if dy == -1 {
                            edits.push((pos, Some(Block::new("minecraft:dirt_path"))));
                            continue;
                        }
                        if is_ground(world, pos) {
                            break;
                        }
                        edits.push((pos, Some(Block::new("minecraft:dirt"))));
                    }
                }
            }
        }
        // The vault across the road (his 10-07 ask).
        vault_edits(world, spawn, (top_fill, under_fill), clear_height, &mut edits, &mut protected);
        // The way back: a lit Nether portal across the house's west end, or
        // the End's exit portal in its floor at the east end.
        match dimension {
            sim::bo2mc::NETHER => {
                for dz in -2..=1 {
                    for dy in -1..=3 {
                        let pos = (spawn.0 - 6, spawn.1 + dy, spawn.2 + dz);
                        let frame = dz == -2 || dz == 1 || dy == -1 || dy == 3;
                        let block = if frame {
                            Block::new("minecraft:obsidian")
                        } else {
                            Block::new("minecraft:nether_portal").with("axis", "z")
                        };
                        protected.insert([pos.0, pos.1, pos.2]);
                        edits.push((pos, Some(block)));
                    }
                }
            }
            sim::bo2mc::END => {
                for dx in 3..=7 {
                    for dz in -2..=2 {
                        let pos = (spawn.0 + dx, spawn.1 - 1, spawn.2 + dz);
                        let ring = dx == 3 || dx == 7 || dz.abs() == 2;
                        let corner = (dx == 3 || dx == 7) && dz.abs() == 2;
                        let block = if corner {
                            continue;
                        } else if ring {
                            let facing = match (dx, dz) {
                                (3, _) => "east",
                                (7, _) => "west",
                                (_, -2) => "south",
                                _ => "north",
                            };
                            Block::new("minecraft:end_portal_frame").with("eye", "true").with("facing", facing)
                        } else {
                            Block::new("minecraft:end_portal")
                        };
                        protected.insert([pos.0, pos.1, pos.2]);
                        edits.push((pos, Some(block)));
                    }
                }
            }
            _ => {}
        }
        let count = edits.len();
        set_blocks(world, shapes, entities.as_deref_mut(), edits);
        diag::info!(
            World,
            "bo2mc: spawn room built at {spawn:?}: {count} blocks set, {} protected",
            protected.len()
        );
        sim::bo2mc::set_room_built(protected);
        // The house is bought: its doors locked until then.
        if sim::bo2mc::bus_on() && dimension == sim::bo2mc::OVERWORLD && !sim::bo2mc::house_bought() {
            let mut locked = Vec::new();
            for c in sim::bo2mc::DOORS.iter().flat_map(|d| d.cells()) {
                for dy in 0..2 {
                    locked.push([spawn.0 + c[0], spawn.1 + c[1] + dy, spawn.2 + c[2]]);
                }
            }
            sim::bo2mc::lock_doors(locked);
        }
        if !sim::bo2mc::vault_bought() {
            let d = sim::bo2mc::VAULT_DOOR;
            let locked = (0..2).map(|dy| [spawn.0 + d.cell[0], spawn.1 + d.cell[1] + dy, spawn.2 + d.cell[2]]).collect();
            sim::bo2mc::lock_vault(locked);
        }
        self.room_at = Some(spawn);
        self.built = true;
        // A sign above every chalk (his 10-07 ask), and the coming-soon
        // signs in the vault.
        self.signs.clear();
        for c in sim::bo2mc::CHALKS {
            let ([dx, dy, dz], facing) = c.sign();
            let lines = c.lines();
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            self.place_sign((spawn.0 + dx, spawn.1 + dy, spawn.2 + dz), facing, &lines);
        }
        diag::info!(World, "bo2mc: {} chalk and coming-soon signs up", self.signs.len());
        // The bread on the north wall (his 10-08): a loaf hung just off the
        // wall at his eye, never picked up, with its price on a sign above.
        let (bx, bz) = sim::bo2mc::BREAD_CELL;
        let count = sim::bo2mc::BREAD_COUNT.to_string();
        let cost = sim::bo2mc::BREAD_COST.to_string();
        self.place_sign((spawn.0 + bx, spawn.1 + 2, spawn.2 + bz), "south", &["Bread", &format!("x{count}"), &cost]);
        if let Some(e) = entities
            && self.bread_shown & (1 << dimension) == 0
        {
            self.bread_shown |= 1 << dimension;
            let at = [f64::from(spawn.0 + bx) + 0.5, f64::from(spawn.1) + 1.2, f64::from(spawn.2 + bz) + 0.2];
            e.show_item("minecraft:bread", at);
        }
    }

    /// Test: the blocks round the house (yard, road, vault) whose collision
    /// in the sim is not the shown block's.
    fn collision_census(&mut self, f: &mut Frame<'_>) {
        let Some(spawn) = self.room_at else { return };
        let (mut checked, mut wrong, mut unloaded) = (0, 0, 0);
        for dx in -30..=70 {
            for dz in -26..=14 {
                for dy in -3..=10 {
                    let pos = (spawn.0 + dx, spawn.1 + dy, spawn.2 + dz);
                    let Some(have) = sim::voxel::block_boxes(pos.0, pos.1, pos.2) else {
                        unloaded += 1;
                        continue;
                    };
                    checked += 1;
                    let block = minecraft_terrain::scene::Scene::block(&f.world.scene, pos).cloned();
                    if block.as_ref().is_some_and(is_door) {
                        continue;
                    }
                    let want: Vec<[f32; 6]> = block
                        .as_ref()
                        .and_then(|b| f.world.stream.states.state_of(b))
                        .map(|state| f.world.registries.blocks.collision_boxes(state).iter().map(|b| b.map(|v| v as f32)).collect())
                        .unwrap_or_default();
                    if want != have {
                        wrong += 1;
                        if wrong <= 40 {
                            let name = block.as_ref().map_or("air".to_owned(), |b| b.id.path.to_string());
                            diag::info!(World, "bo2mc collision census: cell ({dx}, {dy}, {dz}) at {pos:?} shows {name} ({} boxes) but collides with {} boxes", want.len(), have.len());
                        }
                    }
                }
            }
        }
        diag::info!(World, "bo2mc collision census: {checked} blocks checked, {wrong} wrong, {unloaded} not loaded");
        sim::voxel::PROBE_EXTRA.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The compass at the top right (his 10-08): which of Minecraft's 32
    /// pictures shows, its needle on the house's mystery box, worked out as
    /// Minecraft's compass does (`CompassAngleState`).
    fn compass(&mut self, f: &mut Frame<'_>) {
        let frames = self.compass_frames.get_or_insert_with(|| {
            let mut all = Vec::new();
            for i in 0..frame::minecraft_ui::COMPASS_FRAMES {
                let id = minecraft_terrain::pack::ResourceId::parse(&format!("minecraft:item/compass_{i:02}")).ok()?;
                let bytes = f.world.packs.texture(&id).ok()??;
                let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
                if img.width() != img.height() || (i > 0 && img.width() as usize * img.height() as usize * 4 * i as usize != all.len()) {
                    return None;
                }
                all.extend_from_slice(img.as_raw());
            }
            diag::info!(World, "bo2mc: compass pictures read ({} bytes)", all.len());
            Some(std::sync::Arc::new(all))
        });
        let Some(frames) = frames.clone() else {
            f.ui.compass = None;
            return;
        };
        let (box_at, _) = sim::bo2mc::box_spot();
        let target = sim::voxel::to_block(f.origin, box_at);
        let to_target = (target[2] - f.feet[2]).atan2(target[0] - f.feet[0]) / std::f64::consts::TAU;
        let facing = (f64::from(f.yaw) / 360.0).rem_euclid(1.0);
        let angle = (0.5 - (facing - 0.25 - to_target)).rem_euclid(1.0);
        let n = frame::minecraft_ui::COMPASS_FRAMES;
        let shown = (16 + (angle * f64::from(n)).round() as u32) % n;
        f.ui.compass = Some((frames, shown));
    }

    /// Puts an oak wall sign in cell `pos`, hung on the wall behind it
    /// (the block at `pos` minus `facing`), its text facing `facing`
    /// ("north", "south", "east" or "west"; anything else is south). Up to
    /// four lines, dark Minecraft text, a line too wide shrunk to fit. A
    /// sign already in that cell is replaced. No collision.
    pub(super) fn place_sign(&mut self, pos: BlockPos, facing: &str, lines: &[&str]) {
        let facing = SignFacing::from_name(facing).unwrap_or(SignFacing::South);
        let lines = lines.iter().take(minecraft_terrain::sign_render::MAX_LINES).map(|l| (*l).to_owned()).collect();
        self.signs.insert(pos, WallSign { pos, facing, lines });
    }

    /// Takes down the sign in cell `pos`, if one hangs there.
    #[allow(dead_code)]
    pub(super) fn remove_sign(&mut self, pos: BlockPos) {
        self.signs.remove(&pos);
    }

    /// Draws this world's signs into a cut-out entity mesh.
    pub(super) fn append_signs(
        &self,
        mesh: &mut minecraft_terrain::mesh::ChunkMesh,
        atlas: &minecraft_terrain::mesh::Atlas,
        light: &minecraft_terrain::lighting::SkyLight,
    ) {
        minecraft_terrain::sign_render::append_wall_signs(mesh, self.signs.values(), atlas, light);
    }

    /// The starting inventory: oak planks and two doors, at the right end
    /// of the hotbar (the guns take the first slots).
    pub(super) fn starting_items(entities: &mut Entities) {

        // The guns take the first hotbar slots; the kit fills from the end.
        let kit = [
            (8, "minecraft:wooden_sword", 1),
            (7, "minecraft:wooden_pickaxe", 1),
            (6, "minecraft:wooden_axe", 1),
            (5, "minecraft:wooden_shovel", 1),
            (4, "minecraft:oak_planks", 16),
            (3, "minecraft:oak_door", 2),
        ];
        for (slot, item, count) in kit {
            let stack = entities.inventory.recipes.stack(item, count);
            if entities.inventory.slots[slot].is_none() {
                entities.inventory.slots[slot] = Some(stack);
            } else {
                let _ = entities.inventory.add_item(stack, entities.selected);
            }
        }
    }
}

/// IW4L_BO2MC_FARM_KIT=1 (tests): grass to farm under the spawn (not
/// protected), a snow layer on it, and a hoe, seeds, bone meal and wheat
/// on the hotbar.
pub(super) fn farm_kit() -> bool {
    std::env::var("IW4L_BO2MC_FARM_KIT").is_ok_and(|v| v == "1")
}

/// Seconds a SetDay forward takes, so the sky moves rather than cuts.
const SET_DAY_SECONDS: f64 = 4.0;
/// How often the underground test and the zombie spawn spots refresh.
const UNDERGROUND_SECONDS: f64 = 0.5;
const SPAWNS_SECONDS: f64 = 1.0;
/// Zombie spawn spots: natural ground this many blocks around the player.
const SPAWN_NEAR: f64 = 12.0;
const SPAWN_FAR: f64 = 28.0;
const SPAWN_COUNT: usize = 30;
/// A claw's hit sound at most this often per block.
const CLAW_SOUND_SECONDS: f64 = 0.45;
/// Reach of the use key on a door, in blocks.
const USE_REACH: f64 = 4.5;

/// bo2mc: how fast the clock runs at this time of day. The day part (dawn
/// 23000 round to nightfall 13000: sunrise, day, sunset) lasts
/// `IW4L_BO2MC_DAY_MINUTES` real minutes (default 6, Minecraft's is 11.7);
/// the night runs at Minecraft's rate (the rounds hold it).
/// `IW4L_BO2MC_DAY_SPEED` multiplies both (tests).
pub(super) fn clock_rate(time_of_day: f64) -> f64 {
    static MINUTES: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    let minutes = *MINUTES.get_or_init(|| {
        std::env::var("IW4L_BO2MC_DAY_MINUTES").ok().and_then(|s| s.trim().parse().ok()).filter(|m: &f64| *m > 0.0).unwrap_or(6.0)
    });
    let day_part = time_of_day >= 23_000.0 || time_of_day < 13_000.0;
    let rate = if day_part { 14_000.0 / (minutes * 60.0 * 20.0) } else { 1.0 };
    rate * super::day_speed()
}

/// Minecraft's survival state of the player (bo2mc keeps BO2's health:
/// what changes it goes to the rules side as player events).
struct Vitals {
    food: minecraftoss_player::survival::FoodData,
    clock: f64,
    air: i32,
    fire_ticks: i32,
    hurt_ticks: i32,
    fall_peak: Option<f64>,
    last_feet: Option<[f64; 3]>,
    /// The swim state last logged (1 body in water, 2 eyes in water).
    swim_state: u8,
    was_on_ground: bool,
    eat_ticks: u32,
    eat_slot: Option<usize>,
}

impl Default for Vitals {
    fn default() -> Self {
        Self {
            food: Default::default(),
            clock: 0.0,
            air: 300,
            fire_ticks: 0,
            hurt_ticks: 0,
            fall_peak: None,
            last_feet: None,
            swim_state: 0,
            was_on_ground: true,
            eat_ticks: 0,
            eat_slot: None,
        }
    }
}

/// BO2 health to Minecraft health points: 10 is 2 (a heart), 100 is 20.
fn to_hp(bo2: i32) -> f32 {
    bo2.max(0) as f32 * 0.2
}

/// What a BO2 weapon is as a hotbar item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bo2Kind {
    Gun,
    Grenade,
    Knife,
}

/// A BO2 weapon as an item, by its script name and inventory type: the
/// knife, an offhand (grenades, monkey bombs), a gun; None for the
/// passing weapons (perk bottles, fists, the syrette).
pub(crate) fn bo2_kind(name: &str, inventory_type: i32) -> Option<Bo2Kind> {
    if ["perk_bottle", "fists", "syrette", "knuckle", "bottle"].iter().any(|k| name.contains(k)) {
        return None;
    }
    if ["knife", "bowie", "tazer", "sickle"].iter().any(|k| name.contains(k)) {
        return Some(Bo2Kind::Knife);
    }
    match inventory_type {
        0 => Some(Bo2Kind::Gun),
        1 => Some(Bo2Kind::Grenade),
        _ => None,
    }
}

/// A weapon's ammo, stock and clip, as the HUD counts an offhand's.
pub(crate) fn ammo_total(ps: &playerstate_iw4::PlayerState, weapon: u32, ammo_index: i32, clip_index: i32) -> i32 {
    use weapon_iw4::{ammo_row_present, ammo_table_key, clip_row_present, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand};
    let (ammo, clip) = (ammo_table_key(ammo_index, weapon), clip_table_key(clip_index, weapon));
    let mut total = 0;
    if ammo_row_present(&ps.ammo, ammo) {
        total += get_ammo_not_in_clip(&ps.ammo, ammo);
    }
    if clip_row_present(&ps.ammoclip, clip) {
        total += get_clip_for_hand(&ps.ammoclip, clip, 0);
    }
    total
}

/// Survival mining (vanilla `getDestroySpeed` / `isCorrectToolForDrops`
/// by the block tags): the held item's speed on a block state, and
/// whether it is the right tool for its drops. A block in
/// `mineable/pickaxe` needs a pickaxe (of the `needs_*_tool` tier).
pub(crate) fn tool_speed(registries: &minecraftoss_core::registries::Registries, state: minecraftoss_core::BlockStateId, item: &str) -> (f32, bool) {
    let path = item.strip_prefix("minecraft:").unwrap_or(item);
    let tag = |name: &str| registries.block_tags.id(name).is_some_and(|t| registries.block_in_tag(state, t));
    let (tier_speed, level) = match path.split('_').next().unwrap_or("") {
        "wooden" => (2.0, 0),
        "stone" => (4.0, 1),
        "iron" => (6.0, 2),
        "golden" => (12.0, 0),
        "diamond" => (8.0, 3),
        "netherite" => (9.0, 4),
        _ => (1.0, -1),
    };
    let kind = ["pickaxe", "axe", "shovel", "hoe", "sword"].into_iter().find(|k| path.ends_with(&format!("_{k}")));
    let needs = if tag("minecraft:needs_diamond_tool") {
        3
    } else if tag("minecraft:needs_iron_tool") {
        2
    } else if tag("minecraft:needs_stone_tool") {
        1
    } else {
        0
    };
    let pickaxe_block = tag("minecraft:mineable/pickaxe");
    let speed = match kind {
        Some("sword") if registries.block_tags.id("minecraft:sword_efficient").is_some() && tag("minecraft:sword_efficient") => 1.5,
        Some("sword") => 1.0,
        Some(k) if tag(&format!("minecraft:mineable/{k}")) => tier_speed,
        _ => 1.0,
    };
    let correct = !pickaxe_block || (kind == Some("pickaxe") && level >= needs);
    (speed, correct)
}

/// What one frame of Minecraft Zombies works with.
pub(super) struct Frame<'a> {
    pub world: &'a mut Loaded,
    pub shapes: &'a mut ShapeCache,
    pub entities: Option<&'a mut Entities>,
    pub mining: &'a mut crate::minecraft_mining::Mining,
    pub sounds: Option<&'a mut crate::minecraft_sounds::Sounds>,
    pub day: &'a mut minecraft_terrain::day_cycle::DayCycle,
    pub ui: &'a mut frame::MinecraftUi,
    pub origin: [f64; 3],
    pub feet: [f64; 3],
    /// The local player's eye in blocks, and his Minecraft yaw and pitch.
    pub eye: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub alive: bool,
    /// The use key is down this frame.
    pub use_down: bool,
    /// The place/eat button is held (right click, left trigger).
    pub right_down: bool,
    pub on_ground: bool,
    /// BO2 health and its most (100 = 20 Minecraft points).
    pub health: i32,
    pub max_health: i32,
    pub dt: f64,
    pub now: f64,
}

fn map_point(origin: [f64; 3], b: [f64; 3]) -> bevy::prelude::Vec3 {
    bevy::prelude::Vec3::from_array(sim::voxel::to_map(origin, b))
}

fn centre(pos: BlockPos) -> [f64; 3] {
    [f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5]
}

/// A block whose screen opens on the place button: 1 a furnace, 2 a chest.
pub(super) fn container_kind(path: &str) -> Option<u8> {
    match path {
        "furnace" => Some(1),
        "chest" | "barrel" | "trapped_chest" => Some(2),
        _ => None,
    }
}

fn is_door(block: &Block) -> bool {
    block.id.path.ends_with("_door")
}

/// Seconds one zombie needs to claw through `block` (None = never): a
/// wooden door `DOOR_SECONDS`, anything else by its hardness and whether
/// it is stone-like (`sim::bo2mc::claw_seconds`).
fn claw_time(world: &Loaded, state: minecraftoss_core::BlockStateId, block: &Block) -> Option<f32> {
    if is_door(block) && block.id.path != "iron_door" {
        return Some(sim::bo2mc::DOOR_SECONDS);
    }
    let info = world.registries.blocks.state(state);
    let stone = world
        .registries
        .block_tags
        .id("minecraft:mineable/pickaxe")
        .is_some_and(|t| world.registries.block_in_tag(state, t));
    sim::bo2mc::claw_seconds(info.destroy_speed, stone)
}

/// Minecraft monsters walled off from the player claw through like the
/// zombies (`claw`, the same block health): the block toward him at head
/// height, then at the feet; the one under them when he is right below.
pub(super) fn digger_claws(diggers: &[glam::DVec3], feet: [f64; 3], dt: f32) {
    let solid = |b: [i32; 3]| sim::voxel::block_boxes(b[0], b[1], b[2]).is_some_and(|boxes| !boxes.is_empty());
    for p in diggers {
        let cell = [p.x.floor() as i32, (p.y + 0.01).floor() as i32, p.z.floor() as i32];
        let (dx, dy, dz) = (feet[0] - p.x, feet[1] - p.y, feet[2] - p.z);
        let picks = if dy <= -2.0 && dx.hypot(dz) < 1.5 {
            vec![[cell[0], cell[1] - 1, cell[2]]]
        } else {
            let (sx, sz) = if dx.abs() >= dz.abs() { (dx.signum() as i32, 0) } else { (0, dz.signum() as i32) };
            vec![[cell[0] + sx, cell[1] + 1, cell[2] + sz], [cell[0] + sx, cell[1], cell[2] + sz]]
        };
        if let Some(block) = picks.into_iter().find(|&b| solid(b)) {
            sim::bo2mc::push(sim::bo2mc::Request::Claw { block, seconds: dt });
        }
    }
}

/// Tells the zombies' planner what the block now at `pos` costs to claw
/// through (nothing there, or a fluid: no price).
pub(super) fn note_dig_cost(world: &Loaded, pos: BlockPos, block: Option<&Block>) {
    let cost = block.and_then(|b| {
        let state = world.stream.states.state_of(b)?;
        if world.registries.blocks.state(state).fluid.is_some() {
            return None;
        }
        claw_time(world, state, b)
    });
    sim::bo2mc::set_dig_cost([pos.0, pos.1, pos.2], cost);
}

/// Both halves of the door at `pos`: (lower, upper).
fn door_halves(block: &Block, pos: BlockPos) -> (BlockPos, BlockPos) {
    if block.properties.get("half").is_some_and(|h| h == "upper") {
        ((pos.0, pos.1 - 1, pos.2), pos)
    } else {
        (pos, (pos.0, pos.1 + 1, pos.2))
    }
}

/// The horizontal direction the player looks, as a block facing.
fn look_facing(yaw: f32) -> &'static str {
    // Minecraft yaw: 0 = +Z (south), 90 = -X (west).
    let y = yaw.rem_euclid(360.0);
    if !(45.0..315.0).contains(&y) {
        "south"
    } else if y < 135.0 {
        "west"
    } else if y < 225.0 {
        "north"
    } else {
        "east"
    }
}

fn state_info<'w>(world: &'w Loaded, pos: BlockPos) -> Option<(minecraftoss_core::BlockStateId, &'w minecraftoss_core::block::StateInfo)> {
    let state = world.scene.state_at(pos)?;
    Some((state, world.registries.blocks.state(state)))
}

/// Air a body fits in: no collision, no fluid.
fn open(world: &Loaded, pos: BlockPos) -> bool {
    match state_info(world, pos) {
        Some((state, info)) => info.fluid.is_none() && world.registries.blocks.collision_boxes(state).is_empty(),
        None => false,
    }
}

/// Ground a zombie stands on: a full block, not a fluid.
fn floor(world: &Loaded, pos: BlockPos) -> bool {
    state_info(world, pos).is_some_and(|(_, info)| info.collision_full_block && info.fluid.is_none())
}

/// A cell a zombie can stand in: ground below, two open cells.
fn standable(world: &Loaded, pos: BlockPos) -> bool {
    floor(world, (pos.0, pos.1 - 1, pos.2)) && open(world, pos) && open(world, (pos.0, pos.1 + 1, pos.2))
}

impl Bo2mcWorld {
    /// One frame: the rules' requests carried out, the clock moved and
    /// published, doors on the use key, and every half second whether the
    /// player is underground and where zombies may rise.
    pub(super) fn frame(&mut self, f: &mut Frame<'_>) {
        // IW4L_BO2MC_WORLD_TEST=1: the rules' requests made up, to check
        // the world side alone: a notice and the Carpenter's wood at 3 s,
        // claws on the north door from 6 s, day set to dawn at 20 s.
        let before = self.test_clock;
        self.test_clock += f.dt;
        // IW4L_BO2MC_TEST_COLLISION=secs: every block round the house whose
        // collision differs from the block shown, logged once.
        if let Ok(at) = std::env::var("IW4L_BO2MC_TEST_COLLISION")
            && let Ok(at) = at.parse::<f64>()
            && before < at
            && self.test_clock >= at
        {
            self.collision_census(f);
        }
        if std::env::var("IW4L_BO2MC_WORLD_TEST").is_ok_and(|v| v == "1") {
            let crossed = |t: f64| before < t && self.test_clock >= t;
            if crossed(3.0) {
                sim::bo2mc::push(sim::bo2mc::Request::Notice { text: "Get above ground! Night is coming.".into(), seconds: 6.0 });
                sim::bo2mc::push(sim::bo2mc::Request::Give { item: "minecraft:oak_planks".into(), count: 32 });
                sim::bo2mc::push(sim::bo2mc::Request::Give { item: "minecraft:oak_door".into(), count: 2 });
            }
            if (6.0..16.0).contains(&self.test_clock)
                && let Some(door) = sim::bo2mc::room_cell_block(DOORS[0].cell)
            {
                sim::bo2mc::push(sim::bo2mc::Request::Claw { block: door, seconds: f.dt as f32 });
            }
            if crossed(20.0) {
                sim::bo2mc::push(sim::bo2mc::Request::SetDay(23_000.0));
            }
        }
        self.log_clock -= f.dt;
        if self.log_clock <= 0.0 {
            self.log_clock = 5.0;
            diag::info!(
                World,
                "bo2mc: day {:.0} paused {} underground {} spawn spots {}",
                f.day.ticks.rem_euclid(24_000.0),
                f.day.paused,
                self.underground,
                self.spawn_count
            );
        }
        for request in sim::bo2mc::take_requests() {
            match request {
                sim::bo2mc::Request::Claw { block, seconds } => self.claw(f, (block[0], block[1], block[2]), seconds),
                sim::bo2mc::Request::PauseDay(paused) => f.day.paused = paused,
                sim::bo2mc::Request::SetDay(ticks) => {
                    let now = f.day.ticks;
                    let ahead = (ticks - now).rem_euclid(24_000.0);
                    if ahead > 0.5 {
                        self.day_move = Some((now, now + ahead, 0.0));
                    }
                }
                sim::bo2mc::Request::Give { item, count } => self.give(f, &item, count),
                sim::bo2mc::Request::Notice { text, seconds } => self.notice = Some((text, seconds)),
                // Its collision went with it (the sim's own boxes).
                sim::bo2mc::Request::BusGone => {}
                sim::bo2mc::Request::OpenDoor { door } => self.open_house(f, door),
                sim::bo2mc::Request::OpenVault => self.open_vault(f),
                sim::bo2mc::Request::SoulsWolves { count } => self.summon_souls_wolves(f, count),
                sim::bo2mc::Request::Rebuild { window } => self.rebuild_board(f, window),
                sim::bo2mc::Request::Sound { event, at, volume, pitch } => {
                    if let Some(sounds) = f.sounds.as_deref_mut() {
                        sounds.play(&f.world.packs, &event, Some(bevy::prelude::Vec3::from_array(at)), volume, pitch);
                    }
                }
                sim::bo2mc::Request::Shock { at, radius, damage } => {
                    if let Some(e) = f.entities.as_deref_mut() {
                        let n = e.shock(sim::voxel::to_block(f.origin, at), f64::from(radius) / 36.0, damage);
                        diag::info!(World, "bo2mc: Electric Cherry shocked {n} mobs");
                    }
                }
                sim::bo2mc::Request::Lightning { at, delay } => {
                    if let Some(e) = f.entities.as_deref_mut() {
                        e.strike(sim::voxel::to_block(f.origin, at), f64::from(delay));
                    }
                }
                sim::bo2mc::Request::Drop { item, count, at } => {
                    if let Some(e) = f.entities.as_deref_mut() {
                        let stack = e.inventory.recipes.stack(&item, count.min(64) as u8);
                        e.world_items.spawn_block_drop(stack, (at[0], at[1], at[2]));
                        diag::info!(World, "bo2mc: dropped {count} {item} at {at:?}");
                    }
                }
            }
        }
        self.chat(f, before);
        self.track_souls_wolves(f);
        self.storm(f);
        self.count_boards(f);
        // The clock: a SetDay plays out over a few seconds.
        if let Some((from, to, elapsed)) = self.day_move.as_mut() {
            *elapsed += f.dt;
            let t = (*elapsed / SET_DAY_SECONDS).clamp(0.0, 1.0);
            let eased = t * t * (3.0 - 2.0 * t);
            f.day.ticks = *from + (*to - *from) * eased;
            if t >= 1.0 {
                self.day_move = None;
            }
        }
        sim::bo2mc::set_day(f.day.ticks.rem_euclid(24_000.0), f.day.paused);
        // The day's measured length, once a day: dawn to nightfall.
        let tod = f.day.ticks.rem_euclid(24_000.0);
        if let Some(last) = self.last_tod {
            if last < 23_000.0 && tod >= 23_000.0 {
                self.dawn_at = Some(self.test_clock);
            }
            if last < 13_000.0 && tod >= 13_000.0 && let Some(dawn) = self.dawn_at.take() {
                diag::info!(World, "bo2mc: the day lasted {:.1} s (dawn to nightfall)", self.test_clock - dawn);
            }
        }
        self.last_tod = Some(tod);
        // The notice on screen.
        f.ui.notice = self.notice.as_ref().map(|(text, _)| text.clone());
        self.compass(f);
        if let Some((_, left)) = self.notice.as_mut() {
            *left -= f.dt as f32;
            if *left <= 0.0 {
                self.notice = None;
            }
        }
        self.survival(f);
        self.portal(f);
        // Doors on the use key.
        f.ui.door_in_reach = f.alive && self.door_target(f).is_some();
        if f.use_down && !self.use_was_down && f.alive {
            self.use_door(f);
        }
        self.use_was_down = f.use_down;
        // Underground, and where zombies rise.
        self.underground_clock -= f.dt;
        if self.underground_clock <= 0.0 {
            self.underground_clock = UNDERGROUND_SECONDS;
            self.underground = f.alive && self.is_underground(f);
            sim::bo2mc::set_player_underground(self.underground);
        }
        self.spawns_clock -= f.dt;
        if self.spawns_clock <= 0.0 && f.alive {
            self.spawns_clock = SPAWNS_SECONDS;
            let spots = self.spawn_spots(f);
            self.spawn_count = spots.len();
            sim::bo2mc::set_spawn_candidates(spots);
        }
        self.tick_containers(f);
    }

    /// The souls round's wolves: on the zombies' spawn spots around the
    /// player, angry at him from the start.
    fn summon_souls_wolves(&mut self, f: &mut Frame<'_>, count: u32) {
        let spots = self.spawn_spots(f);
        if spots.is_empty() {
            diag::warn!(World, "bo2mc: no spot for the souls wolves");
            return;
        }
        let step = (spots.len() / count.max(1) as usize).max(1);
        let positions: Vec<[f64; 3]> = spots.iter().step_by(step).take(count as usize).map(|&p| sim::voxel::to_block(f.origin, p)).collect();
        let n = positions.len();
        if let Some(e) = f.entities.as_deref_mut() {
            e.summon_souls_wolves(positions);
        }
        diag::info!(World, "bo2mc: {n} souls wolves summoned");
    }

    /// The souls round's thunderstorm: while the fog is in, Minecraft
    /// lightning strikes the zombies' spawn spots around him every few
    /// seconds (the first as the fog starts to roll in).
    fn storm(&mut self, f: &mut Frame<'_>) {
        if !sim::bo2mc::souls_fog() {
            self.storm = 0.3;
            return;
        }
        self.storm -= f.dt;
        if self.storm > 0.0 {
            return;
        }
        let spots = sim::bo2mc::spawn_candidates();
        let h = ((f.now * 1000.0) as u64 ^ 0x5eed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.storm = 3.0 + ((h >> 40) % 600) as f64 / 100.0;
        if spots.is_empty() {
            return;
        }
        let at = spots[(h >> 8) as usize % spots.len()];
        if let Some(e) = f.entities.as_deref_mut() {
            e.strike(sim::voxel::to_block(f.origin, at), 0.0);
        }
    }

    /// How many souls wolves live, for the rules side, and where the last
    /// one to die fell (its Max Ammo drops there).
    /// He put a pane back into window `window`: the first gap, the
    /// bottom row first.
    fn rebuild_board(&mut self, f: &mut Frame<'_>, window: usize) {
        let (Some(room), Some(w)) = (self.room_at, sim::bo2mc::WINDOWS.get(window)) else {
            return;
        };
        let Some(pos) = w.boards().iter().map(|c| (room.0 + c[0], room.1 + c[1], room.2 + c[2])).find(|&p| is_air(f.world, p)) else {
            return;
        };
        set_blocks(f.world, f.shapes, f.entities.as_deref_mut(), vec![(pos, Some(Block::new(BOARD)))]);
        if let Some(sounds) = f.sounds.as_deref_mut() {
            let at = Some(map_point(f.origin, centre(pos)));
            sounds.play(&f.world.packs, "minecraft:block.glass.place", at, 1.0, 0.8);
        }
        diag::info!(World, "bo2mc: board nailed back in window {window} at {pos:?}");
    }

    /// The boards up in each window, for the rules side (its rebuild
    /// prompt and points).
    fn count_boards(&mut self, f: &mut Frame<'_>) {
        let Some(room) = self.room_at else {
            return;
        };
        // IW4L_BO2MC_TEST_WINDOW=<n> (hidden tests): window n starts with
        // its boards torn off.
        static TORN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if let Some(w) = std::env::var("IW4L_BO2MC_TEST_WINDOW").ok().and_then(|v| v.parse::<usize>().ok()).and_then(|w| sim::bo2mc::WINDOWS.get(w))
            && !TORN.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let edits = w.boards().iter().map(|c| ((room.0 + c[0], room.1 + c[1], room.2 + c[2]), None)).collect();
            set_blocks(f.world, f.shapes, f.entities.as_deref_mut(), edits);
            diag::info!(World, "bo2mc: test window torn open");
        }
        let counts = sim::bo2mc::WINDOWS
            .iter()
            .map(|w| w.boards().iter().filter(|c| !is_air(f.world, (room.0 + c[0], room.1 + c[1], room.2 + c[2]))).count() as u8)
            .collect();
        sim::bo2mc::set_boards_up(counts);
    }

    fn track_souls_wolves(&mut self, f: &mut Frame<'_>) {
        let Some(e) = f.entities.as_deref() else { return };
        let alive: HashMap<u64, [f64; 3]> = e.souls_wolves().into_iter().collect();
        let died = self.souls_wolves.iter().filter(|(id, _)| !alive.contains_key(id)).map(|(_, &p)| p).last();
        let died_at = died.map(|p| sim::voxel::to_map(f.origin, p));
        if alive.len() != self.souls_wolves.len() {
            diag::info!(World, "bo2mc: souls wolves alive {}", alive.len());
        }
        sim::bo2mc::set_souls_wolves(alive.len(), died_at);
        self.wolf_progress.retain(|id, _| alive.contains_key(id));
        // The round is over (cleared, or given up on a wolf that never got
        // in): the wolves left die on Minecraft lightning.
        let fog = sim::bo2mc::souls_fog();
        if self.souls_fog_was && !fog && !alive.is_empty() {
            if let Some(e) = f.entities.as_deref_mut() {
                for p in alive.values() {
                    e.strike(*p, 0.0);
                }
                e.end_souls_wolves();
            }
            diag::info!(World, "bo2mc: the souls round is over; {} wolves left die on lightning", alive.len());
        }
        self.souls_fog_was = fog;
        let mut stuck = Vec::new();
        for (&id, p) in &alive {
            let d = DVec3::from_array(*p).distance(DVec3::from_array(f.feet));
            let (best, since, near) = self.wolf_progress.entry(id).or_insert((d, f.now, f.now));
            if d <= WOLF_NEAR {
                *near = f.now;
            }
            if d < *best - 1.0 {
                *best = d;
                *since = f.now;
            }
            if fog && f.alive && d > WOLF_NEAR && (f.now - *since > WOLF_STUCK_SECS || f.now - *near > WOLF_LATE_SECS) {
                stuck.push(id);
            }
        }
        self.souls_wolves = alive;
        // A wolf that cannot find a way in (boarded windows, Minecraft
        // paths) comes back as the hellhounds arrive: on a Minecraft
        // lightning bolt, a few blocks from him.
        for id in stuck {
            let h = (id ^ (f.now * 1000.0) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let Some(spot) = wolf_landing(f.world, f.feet, (h >> 32) as u32) else {
                diag::info!(World, "bo2mc: souls wolf {id} is stuck and there is no clear spot near him");
                self.wolf_progress.insert(id, (f64::MAX, f.now, f.now));
                continue;
            };
            if let Some(e) = f.entities.as_deref_mut() {
                e.strike(spot, 0.0);
                e.move_souls_wolf(id, spot);
            }
            let d = DVec3::from_array(spot).distance(DVec3::from_array(f.feet));
            self.wolf_progress.insert(id, (d, f.now, f.now));
            diag::info!(World, "bo2mc: souls wolf {id} was stuck; it came back on lightning at {spot:?}");
        }
    }

    /// Furnaces cook twenty ticks a second, open or not (a lit one shows
    /// its fire); a furnace or chest that is gone spills what it held.
    fn tick_containers(&mut self, f: &mut Frame<'_>) {
        let gone: Vec<BlockPos> = self
            .furnaces
            .keys()
            .chain(self.chests.keys())
            .copied()
            .filter(|&pos| minecraft_terrain::scene::Scene::block(&f.world.scene, pos).is_none_or(|b| container_kind(&b.id.path).is_none()))
            .collect();
        for pos in gone {
            let mut items = Vec::new();
            if let Some(mut furnace) = self.furnaces.remove(&pos) {
                items.extend(furnace.take_contents());
            }
            if let Some(mut chest) = self.chests.remove(&pos) {
                items.extend(chest.take_contents());
            }
            if self.open_container == Some(pos) {
                self.open_container = None;
                f.ui.inventory_open = false;
                f.ui.container_kind = 0;
            }
            if let Some(e) = f.entities.as_deref_mut() {
                for stack in items {
                    e.world_items.spawn_block_drop(stack, pos);
                }
            }
        }
        self.furnace_clock += f.dt;
        let ticks = (self.furnace_clock * 20.0) as u32;
        self.furnace_clock -= f64::from(ticks) / 20.0;
        if ticks == 0 || self.furnaces.is_empty() {
            return;
        }
        let Some(e) = f.entities.as_deref_mut() else {
            return;
        };
        let recipes = e.inventory.recipes.clone();
        let mut relit = Vec::new();
        for (pos, furnace) in &mut self.furnaces {
            let was = furnace.is_lit();
            for _ in 0..ticks.min(100) {
                furnace.tick(&recipes);
            }
            if furnace.is_lit() != was {
                relit.push((*pos, furnace.is_lit()));
            }
        }
        let mut edits = Vec::new();
        for (pos, lit) in relit {
            if let Some(block) = minecraft_terrain::scene::Scene::block(&f.world.scene, pos).cloned().filter(|b| b.id.path == "furnace") {
                edits.push((pos, Some(block.with("lit", if lit { "true" } else { "false" }))));
            }
        }
        set_blocks(f.world, f.shapes, Some(e), edits);
    }

    /// A zombie's claws on a block: its crack grows by the claw time over
    /// the block's dig time; at full, it breaks (a door, both halves).
    fn claw(&mut self, f: &mut Frame<'_>, pos: BlockPos, seconds: f32) {
        // The room's doors never break; its windows' boards do (protected
        // from him, not from the zombies).
        let board = sim::bo2mc::window_board_block([pos.0, pos.1, pos.2]).is_some();
        if sim::bo2mc::zombie_proof([pos.0, pos.1, pos.2]) || (!board && sim::bo2mc::is_protected([pos.0, pos.1, pos.2])) {
            return;
        }
        let Some(block) = minecraft_terrain::scene::Scene::block(&f.world.scene, pos).cloned() else {
            return;
        };
        let Some((_, info)) = state_info(f.world, pos) else {
            return;
        };
        if info.fluid.is_some() {
            return;
        }
        let dig = if board {
            (block.id.path == "glass").then_some(sim::bo2mc::BOARD_SECONDS)
        } else {
            state_info(f.world, pos).and_then(|(state, _)| claw_time(f.world, state, &block))
        };
        let Some(dig) = dig else {
            sim::bo2mc::mark_unbreakable([pos.0, pos.1, pos.2]);
            return;
        };
        let kind = f.world.scene.sound_type(&block);
        if self.claw_sounds.get(&pos).is_none_or(|at| f.now - at >= CLAW_SOUND_SECONDS) {
            self.claw_sounds.insert(pos, f.now);
            if let (Some(sounds), Some(kind)) = (f.sounds.as_deref_mut(), kind.as_ref()) {
                sounds.play(&f.world.packs, &kind.hit, Some(map_point(f.origin, centre(pos))), (kind.volume + 1.0) / 4.0, kind.pitch * 0.5);
            }
        }
        let done = f.mining.claw(&f.world.packs, &f.world.scene, &f.world.atlas, pos, &block, seconds / dig, f.now);
        if !done {
            return;
        }
        self.claw_sounds.remove(&pos);
        let mut edits = vec![(pos, None)];
        if is_door(&block) {
            let (lower, upper) = door_halves(&block, pos);
            edits = vec![(lower, None), (upper, None)];
        }
        if let (Some(sounds), Some(kind)) = (f.sounds.as_deref_mut(), kind.as_ref()) {
            sounds.play(&f.world.packs, &kind.break_sound, Some(map_point(f.origin, centre(pos))), (kind.volume + 1.0) / 2.0, kind.pitch * 0.8);
        }
        diag::info!(World, "bo2mc: zombies broke {} at {pos:?} ({dig:.1} s for one zombie)", block.id.path);
        set_blocks(f.world, f.shapes, f.entities.as_deref_mut(), edits);
    }

    /// Items into the player's inventory, the hotbar first.
    fn give(&mut self, f: &mut Frame<'_>, item: &str, count: u32) {
        let Some(entities) = f.entities.as_deref_mut() else {
            return;
        };
        let mut left = count;
        let max = u32::from(entities.inventory.recipes.max_stack(item).max(1));
        while left > 0 {
            let n = left.min(max);
            left -= n;
            let stack = entities.inventory.recipes.stack(item, n as u8);
            if entities.inventory.add_item(stack, entities.selected).is_some() {
                diag::info!(World, "bo2mc: inventory full, {item} not all given");
                break;
            }
        }
        diag::info!(World, "bo2mc: gave {count} {item}");
    }

    /// Minecraft survival at 20 ticks a second: falls, drowning, lava and
    /// fire, hunger and its regeneration (as player events for the rules
    /// side), eating, and the armor worn; the bars for the HUD.
    fn survival(&mut self, f: &mut Frame<'_>) {
        let v = &mut self.vitals;
        if !f.alive {
            v.fall_peak = None;
            v.last_feet = None;
            v.eat_ticks = 0;
            f.ui.vitals = None;
            return;
        }
        let block_path = |pos: BlockPos| -> String {
            minecraft_terrain::scene::Scene::block(&f.world.scene, pos).map_or(String::new(), |b| b.id.path.clone())
        };
        let cell = |p: [f64; 3]| (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
        // Minecraft points to BO2 health (2 points, a heart, = 10).
        let damage = |hp: f32, cause: &'static str| {
            // Creative and spectator take no harm.
            if sim::bo2mc::invulnerable() {
                return;
            }
            sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::Damage { amount: (hp * 5.0).round() as i32, cause });
            diag::info!(World, "bo2mc: {cause} damage {hp} points");
        };
        // The fall: from the highest point in the air to the landing (none
        // while flying, as in Minecraft).
        if sim::bo2mc::flying() {
            v.fall_peak = None;
        }
        let in_water_now = matches!(block_path(cell(f.feet)).as_str(), "water" | "bubble_column");
        if f.on_ground || in_water_now {
            if let Some(peak) = v.fall_peak.take() {
                let fall = peak - f.feet[1];
                if fall > 3.0 && !in_water_now {
                    damage((fall - 3.0).ceil() as f32, "fall");
                }
            }
        } else {
            v.fall_peak = Some(v.fall_peak.map_or(f.feet[1], |p| p.max(f.feet[1])));
        }
        {
            // Swimming: his body or his eyes in water (the rules side swims him).
            let wet = |p: [f64; 3]| matches!(block_path(cell(p)).as_str(), "water" | "bubble_column");
            let body = wet(f.feet) || wet([f.feet[0], f.feet[1] + 0.6, f.feet[2]]);
            let (body, eye) = (f.alive && body, f.alive && wet(f.eye));
            sim::bo2mc::set_player_water(body, eye);
            let state = u8::from(body) + u8::from(eye) * 2;
            if v.swim_state != state {
                v.swim_state = state;
                diag::info!(World, "bo2mc swim: body {body} eyes {eye} at {:.1} {:.2} {:.1}", f.feet[0], f.feet[1], f.feet[2]);
            }
        }
        v.clock += f.dt;
        // His BO2 health from the rules side (10 = one heart).
        let (health, max_health) = sim::bo2mc::player_health().unwrap_or((f.health, f.max_health));
        let mut hp = to_hp(health);
        let max_hp = to_hp(max_health.max(1));
        while v.clock >= 0.05 {
            v.clock -= 0.05;
            let feet = block_path(cell(f.feet));
            let body = block_path(cell([f.feet[0], f.feet[1] + 1.0, f.feet[2]]));
            let eye = block_path(cell(f.eye));
            let water = |p: &str| matches!(p, "water" | "bubble_column");
            // Air: 15 s of it under water, then 2 points a second.
            if water(&eye) {
                v.air -= 1;
                if v.air <= -20 {
                    v.air = 0;
                    damage(2.0, "drown");
                }
            } else {
                v.air = (v.air + 4).min(300);
            }
            // Lava and fire: hurt every half second, burning on after.
            v.hurt_ticks -= 1;
            let in_lava = feet == "lava" || body == "lava";
            let in_fire = matches!(feet.as_str(), "fire" | "soul_fire");
            if in_lava {
                v.fire_ticks = v.fire_ticks.max(300);
                if v.hurt_ticks <= 0 {
                    v.hurt_ticks = 10;
                    damage(4.0, "lava");
                }
            } else if in_fire {
                v.fire_ticks = v.fire_ticks.max(160);
                if v.hurt_ticks <= 0 {
                    v.hurt_ticks = 10;
                    damage(1.0, "in_fire");
                }
            }
            if v.fire_ticks > 0 {
                if water(&feet) || water(&body) {
                    v.fire_ticks = 0;
                } else {
                    v.fire_ticks -= 1;
                    if v.fire_ticks % 20 == 0 && !in_lava && !in_fire {
                        damage(1.0, "on_fire");
                    }
                }
            }
            // Hunger: sprinting and jumping tire (walking does not).
            // Stamin-Up: sprinting tires a quarter as much.
            // Creative and spectator never tire.
            if let Some(last) = v.last_feet.filter(|_| !sim::bo2mc::invulnerable()) {
                let moved = (f.feet[0] - last[0]).hypot(f.feet[2] - last[2]);
                if moved > 0.3 && moved < 4.0 {
                    let tire = if sim::bo2mc::player_has_perk("specialty_longersprint") { 0.025 } else { 0.1 };
                    v.food.add_exhaustion((tire * moved) as f32);
                }
            }
            if v.was_on_ground && !f.on_ground && !sim::bo2mc::invulnerable() && v.last_feet.is_some_and(|l| f.feet[1] > l[1]) {
                v.food.add_exhaustion(0.05);
            }
            v.was_on_ground = f.on_ground;
            v.last_feet = Some(f.feet);
            let before = hp;
            if v.food.tick(&mut hp, max_hp, true, minecraftoss_player::Difficulty::Normal) {
                damage(1.0, "starve");
            }
            if hp > before {
                // Quick Revive: hearts come back twice as fast.
                let quick = if sim::bo2mc::player_has_perk("specialty_quickrevive") { 2.0 } else { 1.0 };
                sim::bo2mc::ask(sim::bo2mc::Ask::Heal(((hp - before) * 5.0 * quick).round().max(1.0) as i32));
            }
            // Eating: hold the button with food, as long as it takes.
            let selected = f.entities.as_deref().map(|e| e.selected);
            let food_in = |e: &Entities, slot: usize| {
                e.inventory.slots[slot]
                    .as_ref()
                    .and_then(|s| minecraftoss_player::food::catalog().get(&s.id).map(|info| (s.id.clone(), info, slot)))
            };
            let food = f.entities.as_deref().and_then(|e| {
                food_in(e, e.selected).or_else(|| {
                    // The offhand's, when the main hand holds no gun.
                    let main_free = e.inventory.slots[e.selected].as_ref().is_none_or(|s| crate::minecraft_inventory::weapon_of(s).is_none());
                    if main_free { food_in(e, 40) } else { None }
                })
            });
            match food {
                Some((id, info, slot)) if f.right_down && (v.food.level < 20 || info.can_always_eat) => {
                    let hand = if slot == 40 { Some(40) } else { selected };
                    if v.eat_slot != hand {
                        v.eat_slot = hand;
                        v.eat_ticks = 0;
                    }
                    v.eat_ticks += 1;
                    if v.eat_ticks % 4 == 0
                        && let Some(sounds) = f.sounds.as_deref_mut()
                    {
                        sounds.play(&f.world.packs, "minecraft:entity.generic.eat", None, 0.5, 1.0);
                    }
                    if v.eat_ticks >= info.consume_ticks.max(1) {
                        v.eat_ticks = 0;
                        v.food.eat(info.nutrition, info.saturation);
                        if let Some(entities) = f.entities.as_deref_mut() {
                            let sel = slot;
                            if let Some(stack) = entities.inventory.slots[sel].as_mut() {
                                stack.count = stack.count.saturating_sub(1);
                                if stack.count == 0 {
                                    entities.inventory.slots[sel] = info.remainder.clone();
                                }
                            }
                        }
                        if let Some(sounds) = f.sounds.as_deref_mut() {
                            sounds.play(&f.world.packs, "minecraft:entity.player.burp", None, 0.5, 1.0);
                        }
                        diag::info!(World, "bo2mc: ate {id}: food {} saturation {:.1}", v.food.level, v.food.saturation);
                    }
                }
                _ => {
                    v.eat_ticks = 0;
                    v.eat_slot = None;
                }
            }
        }
        sim::bo2mc::set_food(f32::from(v.food.level), v.food.saturation);
        let (armor, toughness) = f
            .entities
            .as_deref()
            .map_or((0, 0.0), |e| (e.inventory.armor_value(), e.inventory.armor_toughness().0 as f32));
        if self.armor_told != Some((armor, toughness)) {
            self.armor_told = Some((armor, toughness));
            sim::bo2mc::ask(sim::bo2mc::Ask::Armor { points: f32::from(armor), toughness });
        }
        f.ui.vitals = Some(frame::minecraft_ui::McVitals {
            health: hp,
            max_health: max_hp,
            food: v.food.level,
            armor,
            air: (v.air < 300).then_some(v.air),
        });
    }

    /// The door under the crosshair within reach: its block and place
    /// (not the house's while it is locked: that one is bought).
    fn door_target(&self, f: &Frame<'_>) -> Option<(Block, BlockPos)> {
        let mut player = minecraftoss_player::Player::new(glam::DVec3::from_array(f.feet));
        player.yaw = f64::from(f.yaw);
        player.pitch = f64::from(f.pitch);
        let pos = player.target(&f.world.scene, USE_REACH)?.pos;
        if sim::bo2mc::is_protected([pos.0, pos.1, pos.2]) {
            return None;
        }
        let block = minecraft_terrain::scene::Scene::block(&f.world.scene, pos).cloned()?;
        is_door(&block).then_some((block, pos))
    }

    /// The house was bought at door `door`: both doors swing open.
    fn open_house(&mut self, f: &mut Frame<'_>, door: usize) {
        let Some(spawn) = self.room_at else { return };
        let mut edits = Vec::new();
        for c in sim::bo2mc::DOORS.iter().flat_map(|d| d.cells()) {
            for dy in 0..2 {
                let pos = (spawn.0 + c[0], spawn.1 + c[1] + dy, spawn.2 + c[2]);
                if let Some(b) = minecraft_terrain::scene::Scene::block(&f.world.scene, pos).cloned().filter(is_door) {
                    edits.push((pos, Some(b.with("open", "true"))));
                }
            }
        }
        set_blocks(f.world, f.shapes, f.entities.as_deref_mut(), edits);
        let d = sim::bo2mc::DOORS[door.min(sim::bo2mc::DOORS.len() - 1)];
        let lower = (spawn.0 + d.cell[0], spawn.1 + d.cell[1], spawn.2 + d.cell[2]);
        if let Some(sounds) = f.sounds.as_deref_mut() {
            sounds.play(&f.world.packs, "minecraft:block.wooden_door.open", Some(map_point(f.origin, centre(lower))), 1.0, 1.0);
        }
        diag::info!(World, "bo2mc: the house is open (bought at door {door})");
    }

    /// The vault was bought: its door swings open.
    fn open_vault(&mut self, f: &mut Frame<'_>) {
        let Some(spawn) = self.room_at else { return };
        let d = sim::bo2mc::VAULT_DOOR;
        let lower = (spawn.0 + d.cell[0], spawn.1 + d.cell[1], spawn.2 + d.cell[2]);
        let mut edits = Vec::new();
        for pos in [lower, (lower.0, lower.1 + 1, lower.2)] {
            if let Some(b) = minecraft_terrain::scene::Scene::block(&f.world.scene, pos).cloned().filter(is_door) {
                edits.push((pos, Some(b.with("open", "true"))));
            }
        }
        set_blocks(f.world, f.shapes, f.entities.as_deref_mut(), edits);
        if let Some(sounds) = f.sounds.as_deref_mut() {
            sounds.play(&f.world.packs, "minecraft:block.wooden_door.open", Some(map_point(f.origin, centre(lower))), 1.0, 1.0);
        }
        diag::info!(World, "bo2mc: the vault is open");
    }

    /// The use key on a door within reach: both halves open or close.
    fn use_door(&mut self, f: &mut Frame<'_>) {
        let Some((block, pos)) = self.door_target(f) else {
            return;
        };
        let (lower, upper) = door_halves(&block, pos);
        let open = block.properties.get("open").is_some_and(|o| o == "true");
        let set_open = |b: Option<Block>| b.filter(is_door).map(|b| b.with("open", if open { "false" } else { "true" }));
        let lower_block = set_open(minecraft_terrain::scene::Scene::block(&f.world.scene, lower).cloned());
        let upper_block = set_open(minecraft_terrain::scene::Scene::block(&f.world.scene, upper).cloned());
        let mut edits = Vec::new();
        if let Some(b) = lower_block {
            edits.push((lower, Some(b)));
        }
        if let Some(b) = upper_block {
            edits.push((upper, Some(b)));
        }
        set_blocks(f.world, f.shapes, f.entities.as_deref_mut(), edits);
        if let Some(sounds) = f.sounds.as_deref_mut() {
            let event = if open { "minecraft:block.wooden_door.close" } else { "minecraft:block.wooden_door.open" };
            sounds.play(&f.world.packs, event, Some(map_point(f.origin, centre(lower))), 1.0, 1.0);
        }
    }

    /// The door the player places: both halves, facing where he looks.
    /// None when there is no room for it.
    pub(super) fn place_door(world: &mut Loaded, shapes: &mut ShapeCache, entities: Option<&mut Entities>, item: &str, at: BlockPos, yaw: f32) -> bool {
        let upper = (at.0, at.1 + 1, at.2);
        if !open(world, at) || !open(world, upper) || !floor(world, (at.0, at.1 - 1, at.2)) {
            return false;
        }
        let facing = look_facing(yaw);
        let make = |upper: bool| {
            Block::new(item)
                .with("facing", facing)
                .with("half", if upper { "upper" } else { "lower" })
                .with("hinge", "left")
                .with("open", "false")
                .with("powered", "false")
        };
        set_blocks(world, shapes, entities, vec![(at, Some(make(false))), (upper, Some(make(true)))]);
        true
    }

    /// A portal trip: what goes with the player (hunger, air, the item he
    /// picked, his armor) moves from the world left behind into this one,
    /// with a word on where he is.
    pub(super) fn hand_over(&mut self, from: &mut Bo2mcWorld, to: u8) {
        self.vitals = std::mem::take(&mut from.vitals);
        self.vitals.fall_peak = None;
        self.vitals.last_feet = None;
        self.picked = from.picked.take();
        self.held_seen = from.held_seen.take();
        self.asked = std::mem::take(&mut from.asked);
        self.armor_told = None;
        self.arrived = true;
        self.travel = None;
        self.preload = None;
        self.portal_clock = 0.0;
        self.test_clock = from.test_clock;
        let text = match to {
            sim::bo2mc::NETHER => "The Nether: no day, no night. The waves never stop.",
            sim::bo2mc::END => "The End: no day, no night. The waves never stop.",
            _ => "Back in the overworld.",
        };
        self.notice = Some((text.into(), 6.0));
    }

    /// A portal at his feet: four seconds in a Nether portal (vanilla's 80
    /// ticks) or a step into an End portal takes him through. The
    /// dimension ahead starts loading at the first touch.
    fn portal(&mut self, f: &Frame<'_>) {
        if self.travel.is_some() {
            return;
        }
        let feet = (f.feet[0].floor() as i32, (f.feet[1] + 0.1).floor() as i32, f.feet[2].floor() as i32);
        let is = |dy: i32, name: &str| {
            minecraft_terrain::scene::Scene::block(&f.world.scene, (feet.0, feet.1 + dy, feet.2)).is_some_and(|b| b.id.path == name)
        };
        let here = sim::bo2mc::dimension();
        if f.alive && (is(0, "end_portal") || is(-1, "end_portal")) && here != sim::bo2mc::NETHER {
            let to = if here == sim::bo2mc::OVERWORLD { sim::bo2mc::END } else { sim::bo2mc::OVERWORLD };
            self.travel = Some(to);
            diag::info!(World, "bo2mc: into the End portal, to {}", sim::bo2mc::dimension_name(to));
            return;
        }
        if f.alive && (is(0, "nether_portal") || is(1, "nether_portal")) && here != sim::bo2mc::END {
            let to = if here == sim::bo2mc::OVERWORLD { sim::bo2mc::NETHER } else { sim::bo2mc::OVERWORLD };
            if self.portal_clock == 0.0 {
                self.preload = Some(to);
                diag::info!(World, "bo2mc: in a Nether portal, {} loading", sim::bo2mc::dimension_name(to));
            }
            self.portal_clock += f.dt;
            if self.portal_clock >= PORTAL_SECONDS {
                self.portal_clock = 0.0;
                self.travel = Some(to);
            }
        } else {
            self.portal_clock = 0.0;
        }
    }

    /// The player's feet below the generated ground of his column (the
    /// generated chunk's highest full ground block, trees left out); the
    /// spawn room and its yard count as ground level.
    fn is_underground(&self, f: &Frame<'_>) -> bool {
        // The Nether is all cave and the End has no ground to be under.
        if sim::bo2mc::endless() {
            return false;
        }
        let (x, z) = (f.feet[0].floor() as i32, f.feet[2].floor() as i32);
        let (ox, oy, oz) = (f.origin[0].floor() as i32, f.origin[1].floor() as i32, f.origin[2].floor() as i32);
        let (yard_x, yard_z) = (ROOM_HALF_X + 1 + YARD, ROOM_HALF_Z + 1 + YARD);
        let road = sim::bo2mc::bus_on() && sim::bo2mc::ROAD_X.contains(&(x - ox)) && sim::bo2mc::ROAD_Z.contains(&(z - oz));
        let top = if road || ((x - ox).abs() <= yard_x && (z - oz).abs() <= yard_z) {
            oy
        } else {
            let Some(chunk) = f.world.scene.generated_chunk((x >> 4, z >> 4)) else {
                return false;
            };
            let blocks = &f.world.registries.blocks;
            let (min_y, height) = (chunk.min_y(), chunk.height());
            let mut top = None;
            for y in (min_y..min_y + height).rev() {
                let state = chunk.block((x & 15) as usize, y, (z & 15) as usize);
                let info = blocks.state(state);
                if !info.collision_full_block || info.fluid.is_some() {
                    continue;
                }
                let tree = f.world.stream.states.block(state).is_some_and(|b| {
                    let path = b.id.path.as_str();
                    path.ends_with("_leaves") || path.ends_with("_log") || path.ends_with("_wood")
                });
                if !tree {
                    top = Some(y + 1);
                    break;
                }
            }
            let Some(top) = top else {
                return false;
            };
            top
        };
        f.feet[1] < f64::from(top) - 0.5
    }

    /// Where zombies may rise: natural ground 12-28 blocks around the
    /// player, spread around him, outside the room; underground, the cells
    /// of his tunnel 2-4 blocks from him.
    fn spawn_spots(&self, f: &Frame<'_>) -> Vec<[f32; 3]> {
        let world = &*f.world;
        let (px, py, pz) = (f.feet[0].floor() as i32, f.feet[1].floor() as i32, f.feet[2].floor() as i32);
        let (ox, oz) = (f.origin[0].floor() as i32, f.origin[2].floor() as i32);
        let in_room = |x: i32, z: i32| (x - ox).abs() <= ROOM_HALF_X + 1 && (z - oz).abs() <= ROOM_HALF_Z + 1;
        let point = |pos: BlockPos| sim::voxel::to_map(f.origin, [f64::from(pos.0) + 0.5, f64::from(pos.1), f64::from(pos.2) + 0.5]);
        let mut out = Vec::new();
        if self.underground {
            for r in 2i32..=4 {
                for dx in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dz.abs()) != r {
                            continue;
                        }
                        for dy in [0, 1, -1, 2, -2] {
                            let pos = (px + dx, py + dy, pz + dz);
                            if !in_room(pos.0, pos.2) && standable(world, pos) {
                                out.push(point(pos));
                                break;
                            }
                        }
                        if out.len() >= SPAWN_COUNT {
                            return out;
                        }
                    }
                }
            }
            return out;
        }
        // A golden-angle spiral of columns through the ring.
        let tries = SPAWN_COUNT * 3;
        for i in 0..tries {
            if out.len() >= SPAWN_COUNT {
                break;
            }
            let angle = i as f64 * 2.399_963;
            let r = SPAWN_NEAR + (SPAWN_FAR - SPAWN_NEAR) * ((i * 7) % tries) as f64 / tries as f64;
            let (x, z) = ((f.feet[0] + r * angle.cos()).floor() as i32, (f.feet[2] + r * angle.sin()).floor() as i32);
            if in_room(x, z) {
                continue;
            }
            // The highest cell a zombie can stand in near his height.
            for y in (py - 16..=py + 16).rev() {
                if standable(world, (x, y, z)) {
                    out.push(point((x, y, z)));
                    break;
                }
            }
        }
        out
    }
}

/// The crop a seed item plants on farmland (`ItemNameBlockItem`s).
fn seed_crop(item: &str) -> Option<&'static str> {
    Some(match item {
        "minecraft:wheat_seeds" => "minecraft:wheat",
        "minecraft:beetroot_seeds" => "minecraft:beetroots",
        "minecraft:carrot" => "minecraft:carrots",
        "minecraft:potato" => "minecraft:potatoes",
        "minecraft:melon_seeds" => "minecraft:melon_stem",
        "minecraft:pumpkin_seeds" => "minecraft:pumpkin_stem",
        "minecraft:torchflower_seeds" => "minecraft:torchflower_crop",
        _ => return None,
    })
}

/// bo2mc (survival): a held item places a block only when it is a block
/// item (tools, food and materials never go into the world). Seeds plant
/// through `use_item`; a wheat item is not the wheat crop.
pub(super) fn places_block(world: &Loaded, item: &str) -> bool {
    if item == "minecraft:redstone" {
        return true;
    }
    if seed_crop(item).is_some() || matches!(item, "minecraft:wheat" | "minecraft:sweet_berries" | "minecraft:glow_berries") {
        return false;
    }
    world.stream.states.state_of(&Block::new(item)).is_some()
}

/// bo2mc (survival): a plant goes only onto ground it grows on, as in
/// Minecraft: a sapling, flower or grass on dirt, grass or farmland; a
/// cactus on sand; sugar cane, a dead bush or bamboo on dirt or sand.
/// Any other block goes anywhere.
pub(super) fn has_soil(world: &Loaded, pos: BlockPos, block: &Block) -> bool {
    use minecraft_terrain::scene::Scene;
    let tagged = |state: minecraftoss_core::BlockStateId, tag: &str| {
        world.registries.block_tags.id(tag).is_some_and(|t| world.registries.block_in_tag(state, t))
    };
    let Some(state) = world.stream.states.state_of(block) else {
        return true;
    };
    let path = block.id.path.as_str();
    let plant = tagged(state, "minecraft:saplings")
        || tagged(state, "minecraft:flowers")
        || tagged(state, "minecraft:small_flowers")
        || matches!(path, "short_grass" | "fern" | "tall_grass" | "large_fern" | "sweet_berry_bush" | "dead_bush" | "cactus" | "sugar_cane" | "bamboo");
    if !plant {
        return true;
    }
    let Some(ground) = Scene::block(&world.scene, (pos.0, pos.1 - 1, pos.2)) else {
        return false;
    };
    // By name too: a block with properties the stream does not know
    // (grass just uncovered from snow) has no state to look up.
    let ground_state = world.stream.states.state_of(ground);
    let in_tag = |tag: &str| ground_state.is_some_and(|state| tagged(state, tag));
    let below = ground.id.path.as_str();
    let dirt = in_tag("minecraft:dirt")
        || matches!(below, "dirt" | "grass_block" | "coarse_dirt" | "podzol" | "rooted_dirt" | "moss_block" | "mud" | "muddy_mangrove_roots" | "mycelium" | "farmland");
    let sand = in_tag("minecraft:sand") || matches!(below, "sand" | "red_sand" | "suspicious_sand");
    match path {
        "cactus" => sand || below == "cactus",
        "sugar_cane" => dirt || sand || below == "sugar_cane",
        "bamboo" => dirt || sand || below == "gravel" || below == "bamboo",
        "dead_bush" => dirt || sand || below.ends_with("terracotta"),
        _ => dirt,
    }
}

fn face_name(face: minecraftoss_player::Face) -> &'static str {
    use minecraftoss_player::Face;
    match face {
        Face::Down => "down",
        Face::Up => "up",
        Face::North => "north",
        Face::South => "south",
        Face::West => "west",
        Face::East => "east",
    }
}

/// `PortalShape`: the open cells inside an obsidian frame around `start`,
/// 2-21 wide and 3-21 tall, standing along x or along z.
fn portal_shape(world: &Loaded, start: BlockPos) -> Option<(&'static str, Vec<BlockPos>)> {
    use minecraft_terrain::scene::Scene;
    let path = |at: BlockPos| Scene::block(&world.scene, at).map_or_else(|| "air".to_string(), |b| b.id.path.clone());
    let open = |at: BlockPos| matches!(path(at).as_str(), "air" | "cave_air" | "fire" | "nether_portal");
    let obsidian = |at: BlockPos| path(at) == "obsidian";
    for (axis, (sx, sz)) in [("x", (1, 0)), ("z", (0, 1))] {
        let along = |p: BlockPos, n: i32| (p.0 + sx * n, p.1, p.2 + sz * n);
        // Down to the frame's bottom, back to its near side.
        let mut bottom = start;
        let mut n = 0;
        while n < 21 && open((bottom.0, bottom.1 - 1, bottom.2)) {
            bottom.1 -= 1;
            n += 1;
        }
        if !obsidian((bottom.0, bottom.1 - 1, bottom.2)) {
            continue;
        }
        let mut back = 0;
        while back < 21 && open(along(bottom, -(back + 1))) {
            back += 1;
        }
        let corner = along(bottom, -back);
        if !obsidian(along(corner, -1)) {
            continue;
        }
        let mut width = 0;
        while width <= 21 && open(along(corner, width)) {
            width += 1;
        }
        if !(2..=21).contains(&width) || !obsidian(along(corner, width)) {
            continue;
        }
        let mut height = 0;
        while height <= 21 && open((corner.0, corner.1 + height, corner.2)) {
            height += 1;
        }
        if !(3..=21).contains(&height) {
            continue;
        }
        let mut whole = true;
        let mut cells = Vec::new();
        for w in 0..width {
            let column = along(corner, w);
            whole &= obsidian((column.0, column.1 - 1, column.2)) && obsidian((column.0, column.1 + height, column.2));
            for h in 0..height {
                let cell = (column.0, column.1 + h, column.2);
                whole &= open(cell);
                cells.push(cell);
            }
        }
        let (near, far) = (along(corner, -1), along(corner, width));
        for h in 0..height {
            whole &= obsidian((near.0, near.1 + h, near.2)) && obsidian((far.0, far.1 + h, far.2));
        }
        if whole {
            return Some((axis, cells));
        }
    }
    None
}

/// Compass words for a heading of `dx`, `dz` blocks (north is -Z).
fn compass(dx: f64, dz: f64) -> &'static str {
    let degrees = dx.atan2(-dz).to_degrees().rem_euclid(360.0);
    ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"][((degrees + 22.5) / 45.0) as usize % 8]
}

/// The Eye of Ender: set into an empty End portal frame (twelve in a ring
/// open the portal inside), or thrown, when it shows the way to the
/// nearest stronghold and breaks one time in five.
fn ender_eye(
    world: &mut Loaded,
    shapes: &mut ShapeCache,
    entities: &mut Entities,
    player: &minecraftoss_player::Player,
) -> Option<(&'static str, BlockPos)> {
    use minecraft_terrain::scene::Scene;
    if let Some(hit) = player.target(&world.scene, 4.5)
        && let Some(frame) = Scene::block(&world.scene, hit.pos).cloned()
        && frame.id.path == "end_portal_frame"
    {
        if frame.properties.get("eye").is_some_and(|e| e == "true") {
            return None;
        }
        let pos = hit.pos;
        set_blocks(world, shapes, Some(&mut *entities), vec![(pos, Some(frame.with("eye", "true")))]);
        consume_one(entities);
        diag::info!(World, "bo2mc: an Eye of Ender set in the frame at {pos:?}");
        let has_eye = |world: &Loaded, at: BlockPos| {
            Scene::block(&world.scene, at).is_some_and(|b| b.id.path == "end_portal_frame" && b.properties.get("eye").is_some_and(|e| e == "true"))
        };
        for cx in pos.0 - 2..=pos.0 + 2 {
            for cz in pos.2 - 2..=pos.2 + 2 {
                let ring: Vec<BlockPos> = (-1..=1)
                    .flat_map(|i| [(cx + i, pos.1, cz - 2), (cx + i, pos.1, cz + 2), (cx - 2, pos.1, cz + i), (cx + 2, pos.1, cz + i)])
                    .collect();
                if ring.iter().all(|&at| has_eye(world, at)) {
                    let inside = (-1..=1)
                        .flat_map(|i| (-1..=1).map(move |k| ((cx + i, pos.1, cz + k), Some(Block::new("minecraft:end_portal")))))
                        .collect();
                    set_blocks(world, shapes, Some(&mut *entities), inside);
                    diag::info!(World, "bo2mc: the End portal opened at {:?}", (cx, pos.1, cz));
                    sim::bo2mc::push(sim::bo2mc::Request::Notice { text: "The End portal is open.".into(), seconds: 5.0 });
                    return Some(("minecraft:block.end_portal.spawn", (cx, pos.1, cz)));
                }
            }
        }
        return Some(("minecraft:block.end_portal_frame.fill", pos));
    }
    let (px, pz) = (player.pos.x, player.pos.z);
    let nearest = world.strongholds.iter().copied().min_by_key(|&(sx, _, sz)| {
        let (dx, dz) = (f64::from(sx) - px, f64::from(sz) - pz);
        (dx * dx + dz * dz) as i64
    });
    let text = match nearest {
        Some((sx, _, sz)) => {
            let (dx, dz) = (f64::from(sx) - px, f64::from(sz) - pz);
            let distance = dx.hypot(dz);
            if distance < 16.0 {
                "The Eye of Ender sinks: the stronghold is right below. Dig down!".to_string()
            } else {
                format!("The Eye of Ender flies {}: a stronghold {} blocks away.", compass(dx, dz), distance.round() as i64)
            }
        }
        None => "The Eye of Ender drops: there is no stronghold here.".to_string(),
    };
    diag::info!(World, "bo2mc: {text}");
    sim::bo2mc::push(sim::bo2mc::Request::Notice { text, seconds: 6.0 });
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    if nanos % 5 == 0 {
        consume_one(entities);
    }
    let at = (px.floor() as i32, player.pos.y.floor() as i32 + 1, pz.floor() as i32);
    Some(("minecraft:entity.ender_eye.launch", at))
}

/// One less of the held stack (a seed planted, bone meal used).
fn consume_one(entities: &mut Entities) {
    let selected = entities.selected;
    let slot = &mut entities.inventory.slots[selected];
    if let Some(stack) = slot.as_mut() {
        stack.count = stack.count.saturating_sub(1);
        if stack.count == 0 {
            *slot = None;
        }
    }
}

/// A filled or emptied bucket in the held slot: the one bucket becomes
/// `into`, or one of a stack goes and `into` joins the inventory.
fn swap_bucket(entities: &mut Entities, into: &str) {
    let selected = entities.selected;
    let one = entities.inventory.slots[selected].as_ref().is_some_and(|s| s.count <= 1);
    if one {
        entities.inventory.slots[selected] = Some(minecraftoss_player::inventory::ItemStack::new(into, 1));
    } else {
        consume_one(entities);
        let _ = entities.inventory.add_item(minecraftoss_player::inventory::ItemStack::new(into, 1), selected);
    }
}

fn player_block(block: &Block) -> minecraftoss_player::Block {
    minecraftoss_player::Block { id: block.id.key(), properties: block.properties.clone() }
}

/// bo2mc (survival): `Item.useOn` for the held item on the block the
/// player looks at: a hoe tills, a shovel makes a path, an axe strips a
/// log, seeds plant on farmland, bone meal grows, buckets fill and pour,
/// flint and steel lights a fire. The sound to play and where, when the
/// item did something.
pub(super) fn use_item(
    world: &mut Loaded,
    shapes: &mut ShapeCache,
    entities: &mut Entities,
    player: &minecraftoss_player::Player,
    item: &str,
) -> Option<(&'static str, BlockPos)> {
    use minecraft_terrain::scene::Scene;
    use minecraftoss_player::Face;
    let selected = entities.selected;
    if item == "minecraft:ender_eye" {
        return ender_eye(world, shapes, entities, player);
    }
    // Armor goes on (trading places with what was worn), as vanilla's
    // right click does; a carved pumpkin, a block, is placed instead.
    if let Some(stack) = entities.inventory.slots[selected].clone()
        && !places_block(world, item)
        && let Some(index) = match entities.inventory.recipes.equipment_slot(&stack) {
            Some("head") => Some(39),
            Some("chest") => Some(38),
            Some("legs") => Some(37),
            Some("feet") => Some(36),
            _ => None,
        }
    {
        let worn = entities.inventory.slots[index].replace(stack);
        entities.inventory.slots[selected] = worn;
        let sound = match item.strip_prefix("minecraft:").unwrap_or(item).split('_').next() {
            Some("leather") => "minecraft:item.armor.equip_leather",
            Some("chainmail") => "minecraft:item.armor.equip_chain",
            Some("iron") => "minecraft:item.armor.equip_iron",
            Some("golden") => "minecraft:item.armor.equip_gold",
            Some("diamond") => "minecraft:item.armor.equip_diamond",
            Some("netherite") => "minecraft:item.armor.equip_netherite",
            Some("turtle") => "minecraft:item.armor.equip_turtle",
            _ => "minecraft:item.armor.equip_generic",
        };
        diag::info!(World, "bo2mc: put on {item}");
        let at = (player.pos.x.floor() as i32, player.pos.y.floor() as i32 + 1, player.pos.z.floor() as i32);
        return Some((sound, at));
    }
    // An empty bucket aims at a still source; everything else at outlines.
    if item == "minecraft:bucket" {
        let hit = player.target_source_fluid(&world.scene, 4.5)?;
        let block = Scene::block(&world.scene, hit.pos)?.clone();
        let (filled, sound) = match block.id.path.as_str() {
            "water" => ("minecraft:water_bucket", "minecraft:item.bucket.fill"),
            "lava" => ("minecraft:lava_bucket", "minecraft:item.bucket.fill_lava"),
            _ => return None,
        };
        if block.properties.get("level").is_some_and(|l| l != "0") || sim::bo2mc::is_protected([hit.pos.0, hit.pos.1, hit.pos.2]) {
            return None;
        }
        set_blocks(world, shapes, Some(&mut *entities), vec![(hit.pos, None)]);
        swap_bucket(entities, filled);
        return Some((sound, hit.pos));
    }
    let hit = player.target(&world.scene, 4.5)?;
    let pos = hit.pos;
    if sim::bo2mc::is_protected([pos.0, pos.1, pos.2]) {
        return None;
    }
    let clicked = Scene::block(&world.scene, pos)?.clone();
    let path = clicked.id.path.as_str();
    let (dx, dy, dz) = hit.face.offset();
    let front = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
    let above = (pos.0, pos.1 + 1, pos.2);
    let air_at = |world: &Loaded, at: BlockPos| {
        Scene::block(&world.scene, at).is_none_or(|b| matches!(b.id.path.as_str(), "air" | "cave_air" | "void_air"))
    };
    let open_above = air_at(world, above) && hit.face != Face::Down;
    if item.ends_with("_hoe") {
        // `HoeItem.TILLABLES`.
        let into = match path {
            "grass_block" | "dirt" | "dirt_path" if open_above => Block::new("minecraft:farmland").with("moisture", "0"),
            "coarse_dirt" | "rooted_dirt" if open_above => Block::new("minecraft:dirt"),
            _ => return None,
        };
        set_blocks(world, shapes, Some(&mut *entities), vec![(pos, Some(into))]);
        entities.inventory.wear_tool(selected, 1);
        return Some(("minecraft:item.hoe.till", pos));
    }
    if item.ends_with("_shovel") {
        // `ShovelItem.FLATTENABLES`.
        if !(open_above && matches!(path, "grass_block" | "dirt" | "podzol" | "mycelium" | "coarse_dirt" | "rooted_dirt")) {
            return None;
        }
        set_blocks(world, shapes, Some(&mut *entities), vec![(pos, Some(Block::new("minecraft:dirt_path")))]);
        entities.inventory.wear_tool(selected, 1);
        return Some(("minecraft:item.shovel.flatten", pos));
    }
    if item.ends_with("_axe") {
        // `AxeItem.STRIPPABLES`: the same block with its bark off.
        let strippable = !path.starts_with("stripped_")
            && (path.ends_with("_log") || path.ends_with("_wood") || path.ends_with("_stem") || path.ends_with("_hyphae") || path == "bamboo_block")
            && !matches!(path, "melon_stem" | "pumpkin_stem" | "attached_melon_stem" | "attached_pumpkin_stem" | "mushroom_stem");
        if !strippable {
            return None;
        }
        let mut into = Block::new(&format!("minecraft:stripped_{path}"));
        into.properties = clicked.properties.clone();
        if world.stream.states.state_of(&into).is_none() {
            return None;
        }
        set_blocks(world, shapes, Some(&mut *entities), vec![(pos, Some(into))]);
        entities.inventory.wear_tool(selected, 1);
        return Some(("minecraft:item.axe.strip", pos));
    }
    if let Some(crop) = seed_crop(item) {
        if hit.face != Face::Up || path != "farmland" || !air_at(world, above) {
            return None;
        }
        set_blocks(world, shapes, Some(&mut *entities), vec![(above, Some(Block::new(crop).with("age", "0")))]);
        consume_one(entities);
        return Some(("minecraft:item.crop.plant", above));
    }
    if item == "minecraft:sweet_berries" {
        let soil = matches!(path, "grass_block" | "dirt" | "coarse_dirt" | "podzol" | "farmland" | "rooted_dirt" | "moss_block");
        if hit.face != Face::Up || !soil || !air_at(world, above) {
            return None;
        }
        set_blocks(world, shapes, Some(&mut *entities), vec![(above, Some(Block::new("minecraft:sweet_berry_bush").with("age", "0")))]);
        consume_one(entities);
        return Some(("minecraft:block.sweet_berry_bush.place", above));
    }
    if item == "minecraft:bone_meal" {
        // `BonemealableBlock.isValidBonemealTarget`, roughly: a crop or
        // bush not yet grown, a sapling, grass with room above.
        let age = clicked.properties.get("age").and_then(|a| a.parse::<u8>().ok()).unwrap_or(0);
        let growable = match path {
            "wheat" | "carrots" | "potatoes" | "melon_stem" | "pumpkin_stem" => age < 7,
            "beetroots" | "sweet_berry_bush" => age < 3,
            "torchflower_crop" | "cocoa" => age < 2,
            "grass_block" | "moss_block" => air_at(world, above),
            "short_grass" | "fern" | "bamboo_sapling" | "bamboo" | "sea_pickle" | "kelp" | "seagrass" | "glow_lichen" | "red_mushroom"
            | "brown_mushroom" | "big_dripleaf" | "small_dripleaf" | "azalea" | "flowering_azalea" | "mangrove_propagule" => true,
            p => p.ends_with("_sapling"),
        };
        if !growable {
            return None;
        }
        entities.bone_meal(pos, face_name(hit.face));
        consume_one(entities);
        return Some(("minecraft:item.bone_meal.use", pos));
    }
    if matches!(item, "minecraft:water_bucket" | "minecraft:lava_bucket") {
        let at = if minecraftoss_player::replaceable(&player_block(&clicked)) { pos } else { front };
        let empty = Scene::block(&world.scene, at).is_none_or(|b| minecraftoss_player::replaceable(&player_block(b)));
        if !empty || sim::bo2mc::is_protected([at.0, at.1, at.2]) {
            return None;
        }
        let (fluid, sound) = if item == "minecraft:water_bucket" {
            ("minecraft:water", "minecraft:item.bucket.empty")
        } else {
            ("minecraft:lava", "minecraft:item.bucket.empty_lava")
        };
        set_blocks(world, shapes, Some(&mut *entities), vec![(at, Some(Block::new(fluid).with("level", "0")))]);
        swap_bucket(entities, "minecraft:bucket");
        return Some((sound, at));
    }
    if item == "minecraft:flint_and_steel" {
        if !air_at(world, front) || sim::bo2mc::is_protected([front.0, front.1, front.2]) {
            return None;
        }
        // Inside an obsidian frame: a Nether portal (`PortalShape`).
        if let Some((axis, cells)) = portal_shape(world, front) {
            let block = Block::new("minecraft:nether_portal").with("axis", axis);
            let count = cells.len();
            set_blocks(world, shapes, Some(&mut *entities), cells.into_iter().map(|c| (c, Some(block.clone()))).collect());
            entities.inventory.wear_tool(selected, 1);
            diag::info!(World, "bo2mc: a Nether portal lit at {front:?} ({count} blocks, axis {axis})");
            return Some(("minecraft:item.flintandsteel.use", front));
        }
        set_blocks(world, shapes, Some(&mut *entities), vec![(front, Some(Block::new("minecraft:fire").with("age", "0")))]);
        entities.inventory.wear_tool(selected, 1);
        return Some(("minecraft:item.flintandsteel.use", front));
    }
    None
}
