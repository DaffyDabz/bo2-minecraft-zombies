//! Minecraft Zombies, the world side of `sim::bo2mc`: the spawn room built
//! in blocks around the world spawn (its walls, floor and roof kept from
//! being broken), and the requests the Black Ops II rules make of the world.
use std::collections::{HashMap, HashSet};

use minecraft_terrain::scene::Block;
use sim::bo2mc::{DOORS, ROOM_HALF_X, ROOM_HALF_Z, ROOM_HEIGHT};

use super::{Loaded, ShapeCache};
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

/// The world side's own state for one world load.
#[derive(Default)]
pub(super) struct Bo2mcWorld {
    /// The spawn room stands.
    pub built: bool,
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
}

/// The facing of a door whose outward direction is `outward` (cells).
fn facing(outward: [i32; 2]) -> &'static str {
    match outward {
        [0, -1] => "north",
        [0, 1] => "south",
        [1, 0] => "east",
        _ => "west",
    }
}

/// A closed oak door's half (`upper`) facing `facing`.
pub(super) fn door(facing: &str, upper: bool, open: bool) -> Block {
    Block::new(DOOR)
        .with("facing", facing)
        .with("half", if upper { "upper" } else { "lower" })
        .with("hinge", "left")
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
        if d.cell[0] == dx && d.cell[2] == dz && (dy == d.cell[1] || dy == d.cell[1] + 1) {
            return Some(door(facing(d.outward), dy != d.cell[1], false));
        }
    }
    if dx.abs() == wall_x || dz.abs() == wall_z {
        return Some(Block::new(WALL));
    }
    None
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
        let shape = block
            .as_ref()
            .and_then(|b| world.stream.states.state_of(b))
            .map_or(0, |state| shapes.shape_id(&world.registries, state));
        let placed = block.is_some();
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
        entities: Option<&mut Entities>,
        spawn: BlockPos,
    ) {
        let (wall_x, wall_z) = (ROOM_HALF_X + 1, ROOM_HALF_Z + 1);
        let (yard_x, yard_z) = (wall_x + YARD, wall_z + YARD);
        let mut edits = Vec::new();
        let mut protected = HashSet::new();
        let range = world.stream.states.vertical_range();
        for dx in -yard_x..=yard_x {
            for dz in -yard_z..=yard_z {
                let in_room = dx.abs() <= wall_x && dz.abs() <= wall_z;
                // Above the floor: the room, or the yard's open air.
                for dy in 0..=CLEAR_HEIGHT {
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
                    if in_room && dy == -1 {
                        protected.insert([pos.0, pos.1, pos.2]);
                        edits.push((pos, Some(Block::new(FLOOR))));
                        continue;
                    }
                    if is_ground(world, pos) {
                        break;
                    }
                    let fill = match (in_room, dy) {
                        (false, -1) => "minecraft:grass_block",
                        (false, _) => "minecraft:dirt",
                        (true, _) => "minecraft:stone",
                    };
                    edits.push((pos, Some(Block::new(fill))));
                }
            }
        }
        let count = edits.len();
        set_blocks(world, shapes, entities, edits);
        diag::info!(
            World,
            "bo2mc: spawn room built at {spawn:?}: {count} blocks set, {} protected",
            protected.len()
        );
        sim::bo2mc::set_room_built(protected);
        self.built = true;
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
            }
        }
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
        if let Some((_, left)) = self.notice.as_mut() {
            *left -= f.dt as f32;
            if *left <= 0.0 {
                self.notice = None;
            }
        }
        self.survival(f);
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
        if sim::bo2mc::is_protected([pos.0, pos.1, pos.2]) {
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
        let dig = if is_door(&block) { Some(sim::bo2mc::DOOR_SECONDS) } else { sim::bo2mc::claw_seconds(info.destroy_speed) };
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
        diag::info!(World, "bo2mc: zombies broke {} at {pos:?}", block.id.path);
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
            sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::Damage { amount: (hp * 5.0).round() as i32, cause });
            diag::info!(World, "bo2mc: {cause} damage {hp} points");
        };
        // The fall: from the highest point in the air to the landing.
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
            if let Some(last) = v.last_feet {
                let moved = (f.feet[0] - last[0]).hypot(f.feet[2] - last[2]);
                if moved > 0.3 && moved < 4.0 {
                    v.food.add_exhaustion((0.1 * moved) as f32);
                }
            }
            if v.was_on_ground && !f.on_ground && v.last_feet.is_some_and(|l| f.feet[1] > l[1]) {
                v.food.add_exhaustion(0.05);
            }
            v.was_on_ground = f.on_ground;
            v.last_feet = Some(f.feet);
            let before = hp;
            if v.food.tick(&mut hp, max_hp, true, minecraftoss_player::Difficulty::Normal) {
                damage(1.0, "starve");
            }
            if hp > before {
                sim::bo2mc::ask(sim::bo2mc::Ask::Heal(((hp - before) * 5.0).round().max(1.0) as i32));
            }
            // Eating: hold the button with food, as long as it takes.
            let selected = f.entities.as_deref().map(|e| e.selected);
            let food = f.entities.as_deref().and_then(|e| e.inventory.slots[e.selected].as_ref()).and_then(|s| {
                minecraftoss_player::food::catalog().get(&s.id).map(|info| (s.id.clone(), info))
            });
            match food {
                Some((id, info)) if f.right_down && (v.food.level < 20 || info.can_always_eat) => {
                    if v.eat_slot != selected {
                        v.eat_slot = selected;
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
                            let sel = entities.selected;
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

    /// The door under the crosshair within reach: its block and place.
    fn door_target(&self, f: &Frame<'_>) -> Option<(Block, BlockPos)> {
        let mut player = minecraftoss_player::Player::new(glam::DVec3::from_array(f.feet));
        player.yaw = f64::from(f.yaw);
        player.pitch = f64::from(f.pitch);
        let pos = player.target(&f.world.scene, USE_REACH)?.pos;
        let block = minecraft_terrain::scene::Scene::block(&f.world.scene, pos).cloned()?;
        is_door(&block).then_some((block, pos))
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

    /// The player's feet below the generated ground of his column (the
    /// generated chunk's highest full ground block, trees left out); the
    /// spawn room and its yard count as ground level.
    fn is_underground(&self, f: &Frame<'_>) -> bool {
        let (x, z) = (f.feet[0].floor() as i32, f.feet[2].floor() as i32);
        let (ox, oy, oz) = (f.origin[0].floor() as i32, f.origin[1].floor() as i32, f.origin[2].floor() as i32);
        let (yard_x, yard_z) = (ROOM_HALF_X + 1 + YARD, ROOM_HALF_Z + 1 + YARD);
        let top = if (x - ox).abs() <= yard_x && (z - oz).abs() <= yard_z {
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
