//! The Minecraft world's mobs, as MinecraftOSS runs them: its integrated
//! server (`engine/viewer/src/server.rs`) ticks the level at 20 Hz on its own
//! thread with natural spawning and the entity world, every passive and
//! hostile mob's AI and pathfinding. Here it is fed the streamed chunks and
//! the player, and hands back the tracked mobs, which are drawn with the
//! viewer's mob renderers. MW2 bullets hurt the mobs with their real damage
//! (a hundred MW2 health to Minecraft's twenty), and mobs hurt the player
//! the other way round.
use glam::{DVec3, Vec3};
use minecraft_terrain::lighting::SkyLight;
use minecraft_terrain::mesh::{Atlas, ChunkMesh};
use minecraft_terrain::scene::{Block, HandcraftedScene, Scene};
use minecraft_terrain::server::{PlayerEdit, ServerHandle, ServerSim, TickInput};
use minecraft_terrain::terrain::TerrainStream;
use minecraft_terrain::client_mobs::{ClientMobs, server_mobs};
use minecraft_terrain::mesh::ItemVisuals;
use minecraft_terrain::server::{EntitySnapshot, ServerItem};
use minecraftoss_player::inventory::{Inventory, ItemStack};
use minecraftoss_player::items::{ItemEntity, WorldItems};
use minecraftoss_player::loot::LootBook;
use minecraftoss_player::rng::XoroshiroRandom;
use std::collections::{HashMap, HashSet};
use minecraft_terrain::pack::PackStack;
use minecraft_terrain::poof_particles::PoofParticles;
use minecraft_terrain::portal_particles::PortalParticles;
use minecraftoss_entities::tempt::PlayerCandidate;
use minecraftoss_entities::world::{EntityWorld, MobHit, PlayerHitKind};

const TICK_SECONDS: f64 = 1.0 / 20.0;
/// Minecraft health per MW2 health.
const HEALTH_SCALE: f32 = 20.0 / 100.0;
/// The player's id in the entity world.
const PLAYER: u64 = 0;
/// `Player.getEyeHeight` standing.
const EYE_HEIGHT: f32 = 1.62;

pub(crate) struct Entities {
    server: ServerHandle,
    /// The mobs the player tracks, as of the last server tick.
    world: EntityWorld,
    /// The client's copies of them: tracked, interpolated, animated.
    client: ClientMobs,
    poof: PoofParticles,
    portal: PortalParticles,
    items: ItemVisuals,
    clock: f64,
    ticks: u64,
    /// The player's inventory: vanilla slots, stacking and recipes.
    pub(crate) inventory: Inventory,
    /// The selected hotbar slot.
    pub(crate) selected: usize,
    /// Item entities as the client shows them: the server's, and drops the
    /// client spawned that the server has not taken yet.
    pub(crate) world_items: WorldItems,
    server_item_ids: HashSet<u32>,
    /// Client drops handed to the server: the command count that sent each.
    server_handed: HashMap<u32, (u64, ItemEntity)>,
    /// Stacks the server let the player pick up, not yet in the inventory.
    server_picked: Vec<(i32, [f64; 3], String, i32, Option<String>)>,
    server_snapshot: Option<EntitySnapshot>,
    server_handled: u64,
    loot: Option<LootBook>,
    loot_sequences: HashMap<String, XoroshiroRandom>,
    seed: i64,
    /// Sounds the mob world made since the last take: event, block point,
    /// volume, pitch.
    pub(crate) sounds: Vec<(String, DVec3, f32, f32)>,
    random: minecraftoss_player::rng::LegacyRandom,
    /// bo2mc: Minecraft lightning (the souls round's hellhounds and storm).
    bolts: crate::minecraft_lightning::Bolts,
    /// Vanilla's difficulty: 0 (peaceful) spawns no monsters and removes
    /// the ones there are; 2 is normal.
    pub(crate) difficulty: i32,
    /// The `doMobSpawning` rule: off in Minecraft Zombies' Nether and End,
    /// where only the waves come.
    pub(crate) spawn_mobs: bool,
    /// Hunting monsters standing still: where each stood and for how many
    /// ticks (bo2mc: a stuck one claws its way to the player).
    stuck: HashMap<u64, (DVec3, u32)>,
    /// The stuck hunters as of the last tick: each one's feet.
    pub(crate) diggers: Vec<DVec3>,
}

/// What bullets and blasts count as for block loot: vanilla drops nothing
/// from stone or ores broken bare-handed.
const LOOT_TOOL: &str = "minecraft:diamond_pickaxe";
/// TNT's power: a blast drops each block's loot one time in this many.
const BLAST_POWER: f32 = 4.0;

/// The mobs drawn this frame: entity models (cut out, back-face culled,
/// translucent) and their shadows, in `mesh::Vertex`s; and what goes with
/// the particles (held items, puffs, flames, potions).
#[derive(Default)]
pub(crate) struct MobMeshes {
    pub models: ChunkMesh,
    pub culled: ChunkMesh,
    pub translucent: ChunkMesh,
    pub shadows: ChunkMesh,
    pub items: ChunkMesh,
}

/// The player as the mobs see it this frame.
pub(crate) struct PlayerView {
    pub feet: [f64; 3],
    pub alive: bool,
    /// MW2 health, 0..100.
    pub health: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Entities {
    pub(crate) fn new(stream: &TerrainStream, seed: i64, dimension: &str) -> Self {
        let sim = ServerSim::new(stream.world_gen(), stream.states.clone(), dimension);
        let mut server = ServerHandle::spawn(sim);
        // Mob and block loot and the recipes (stack sizes), from the game's
        // data JAR when MinecraftOSS has one.
        let jar = data_jar();
        let mut inventory = Inventory::default();
        let mut loot = None;
        if let Some(jar) = jar {
            server.load_loot(jar.clone(), seed as u64);
            match minecraftoss_player::crafting::RecipeBook::from_jar(&jar) {
                Ok(recipes) => {
                    // Real stack sizes, durability and armor from the item
                    // catalog the setup unpacked (64 for everything without).
                    let catalog = assets::minecraft_map::root()
                        .map(|root| minecraftoss_core::registries::DataPaths::under(&root).item_catalog)
                        .and_then(|path| minecraftoss_player::item_catalog::ItemCatalog::from_path(&path).ok());
                    let recipes = match catalog {
                        Some(catalog) => recipes.with_item_catalog(std::sync::Arc::new(catalog)),
                        None => {
                            diag::warn!(World, "Minecraft item catalog unavailable: stacks of 64");
                            recipes
                        }
                    };
                    let recipes = std::sync::Arc::new(recipes);
                    server.set_recipe_book(recipes.clone());
                    inventory.recipes = recipes;
                }
                Err(error) => diag::warn!(World, "Minecraft recipes unavailable: {error:#}"),
            }
            match LootBook::from_jar(&jar) {
                Ok(book) => loot = Some(book),
                Err(error) => diag::warn!(World, "Minecraft block loot unavailable: {error:#}"),
            }
        } else {
            diag::warn!(World, "Minecraft data JAR not found: no loot or recipes");
        }
        Self {
            server,
            world: EntityWorld::default(),
            client: ClientMobs::default(),
            poof: PoofParticles::default(),
            portal: PortalParticles::default(),
            items: ItemVisuals::default(),
            clock: 0.0,
            ticks: 0,
            inventory,
            selected: 0,
            world_items: WorldItems::default(),
            server_item_ids: HashSet::new(),
            server_handed: HashMap::new(),
            server_picked: Vec::new(),
            server_snapshot: None,
            server_handled: 0,
            loot,
            loot_sequences: HashMap::new(),
            seed,
            sounds: Vec::new(),
            random: minecraftoss_player::rng::LegacyRandom::new((seed ^ 0x1735) as u64),
            bolts: Default::default(),
            difficulty: 2,
            spawn_mobs: true,
            stuck: HashMap::new(),
            diggers: Vec::new(),
        }
    }

    pub(crate) fn load_chunk(&mut self, chunk: &std::sync::Arc<minecraftoss_core::Chunk>) {
        self.server.load_chunk(chunk);
    }

    pub(crate) fn unload_chunk(&mut self, pos: minecraftoss_core::ChunkPos) {
        self.server.unload_chunk(pos);
    }

    /// Block loot for what the weapons broke, dropped as vanilla drops it
    /// (`Block.popResource`); a blast keeps each drop one time in four.
    pub(crate) fn drop_blocks(&mut self, broken: &[((i32, i32, i32), Block, bool)]) {
        let Some(loot) = self.loot.as_ref() else {
            return;
        };
        let tool = ItemStack::new(LOOT_TOOL, 1);
        for (pos, block, blast) in broken {
            let block = minecraftoss_player::Block { id: block.id.key(), properties: block.properties.clone() };
            let Some(drops) = loot.roll_drops_named(&block, Some(&tool), self.seed as u64, &mut self.loot_sequences) else {
                continue;
            };
            for mut drop in drops {
                if *blast && self.random.next_float() >= 1.0 / BLAST_POWER {
                    continue;
                }
                if drop.components.is_none() {
                    drop.max = drop.max.min(self.inventory.recipes.max_stack(&drop.id));
                }
                self.world_items.spawn_block_drop(drop, *pos);
            }
        }
    }

    /// bo2mc (survival): block loot as broken with `tools[i]` (None = no
    /// drop: the wrong tool, or a bullet through a block that needs one;
    /// Some(None) = the bare hand).
    pub(crate) fn drop_blocks_with(&mut self, broken: &[((i32, i32, i32), Block, bool)], tools: &[Option<Option<ItemStack>>]) {
        let Some(loot) = self.loot.as_ref() else {
            return;
        };
        for ((pos, block, blast), tool) in broken.iter().zip(tools) {
            let Some(tool) = tool else {
                continue;
            };
            let block = minecraftoss_player::Block { id: block.id.key(), properties: block.properties.clone() };
            let Some(drops) = loot.roll_drops_named(&block, tool.as_ref(), self.seed as u64, &mut self.loot_sequences) else {
                continue;
            };
            for mut drop in drops {
                if *blast && self.random.next_float() >= 1.0 / BLAST_POWER {
                    continue;
                }
                if drop.components.is_none() {
                    drop.max = drop.max.min(self.inventory.recipes.max_stack(&drop.id));
                }
                self.world_items.spawn_block_drop(drop, *pos);
            }
        }
    }

    /// `ItemEntity`s to draw and pick up, as the viewer's
    /// `server_items_tick` does for a server-simulated world: client drops
    /// go to the server, stacks the server offered go into the inventory,
    /// and the server's items are mirrored for drawing.
    fn server_items_tick(&mut self, feet: DVec3) {
        let entities = std::mem::take(&mut self.world_items.entities);
        let to_hand: Vec<ItemEntity> = entities
            .iter()
            .filter(|e| !self.server_item_ids.contains(&e.entity_id) && !self.server_handed.contains_key(&e.entity_id))
            .cloned()
            .collect();
        for entity in &to_hand {
            let components = entity.stack.components.as_ref().map(|c| c.to_string());
            self.server.spawn_item(
                &entity.stack.id,
                i32::from(entity.stack.count),
                components.as_deref(),
                entity.position.to_array(),
                entity.velocity.to_array(),
                i32::from(entity.pickup_delay),
                entity.age as i32,
            );
            self.server_handed.insert(entity.entity_id, (self.server.sent(), entity.clone()));
        }
        let previous: HashMap<u32, ItemEntity> = entities.into_iter().map(|e| (e.entity_id, e)).collect();
        let target = feet + DVec3::Y * 0.81;
        self.world_items.tick_pickup_effects(target);
        for (id, position, item, count, components) in std::mem::take(&mut self.server_picked) {
            let recipes = self.inventory.recipes.clone();
            let make_stack = |item: &str, count: i32| {
                let mut stack = ItemStack::new(item, count.clamp(0, 255) as u8);
                stack.components = components.as_deref().and_then(|c| serde_json::from_str(c).ok());
                if stack.components.is_none() {
                    stack.max = stack.max.min(recipes.max_stack(item));
                }
                stack
            };
            let taken = match self.inventory.add_item(make_stack(&item, count), self.selected) {
                None => count,
                Some(rest) => {
                    let rest_count = i32::from(rest.count);
                    self.server.spawn_item(&item, rest_count, components.as_deref(), feet.to_array(), [0.0; 3], 0, 0);
                    count - rest_count
                }
            };
            if taken <= 0 {
                continue;
            }
            let transfer = make_stack(&item, taken);
            let snapshot = previous.get(&(id as u32)).cloned().unwrap_or_else(|| ItemEntity {
                entity_id: id as u32,
                stack: transfer.clone(),
                position: DVec3::from_array(position),
                previous_position: DVec3::from_array(position),
                velocity: DVec3::ZERO,
                age: 0,
                bob_offset: bob_offset(id),
                pickup_delay: 0,
                on_ground: true,
            });
            self.world_items.note_pickup(snapshot, target, transfer);
        }
        let Some(snapshot) = self.server_snapshot.take() else {
            self.world_items.entities = previous.into_values().collect();
            self.world_items.entities.sort_by_key(|e| e.entity_id);
            return;
        };
        let recipes = self.inventory.recipes.clone();
        self.world_items.entities = snapshot
            .items
            .into_iter()
            .map(|item: ServerItem| {
                let mut stack = ItemStack::new(&item.item, item.count.clamp(0, 255) as u8);
                stack.components = item.components.as_deref().and_then(|c| serde_json::from_str(c).ok());
                if stack.components.is_none() {
                    stack.max = stack.max.min(recipes.max_stack(&item.item));
                }
                ItemEntity {
                    entity_id: item.id as u32,
                    stack,
                    position: DVec3::from_array(item.position),
                    previous_position: DVec3::from_array(item.previous_position),
                    velocity: DVec3::from_array(item.velocity),
                    age: item.age.max(0) as u32,
                    bob_offset: bob_offset(item.id),
                    pickup_delay: item.pickup_delay.clamp(0, i32::from(u16::MAX)) as u16,
                    on_ground: item.on_ground,
                }
            })
            .collect();
        self.server_item_ids = self.world_items.entities.iter().map(|e| e.entity_id).collect();
        let handled = self.server_handled;
        self.server_handed.retain(|_, (sent, _)| *sent > handled);
        for (_, entity) in self.server_handed.values() {
            self.world_items.entities.push(entity.clone());
        }
    }

    /// bo2mc: an item hung still at block point `at` for show (the bread
    /// on the house wall): never picked up, never despawns.
    pub(crate) fn show_item(&mut self, item: &str, at: [f64; 3]) {
        self.server.spawn_item(item, 1, None, at, [0.0; 3], 32767, -32768);
    }

    /// A held item's model under a view-space pose, lit at `light_at`.
    pub(crate) fn held_item_mesh(
        &mut self,
        id: &str,
        pose: glam::Mat4,
        light_at: glam::Vec3,
        packs: &PackStack,
        atlas: &Atlas,
        light: &SkyLight,
    ) -> ChunkMesh {
        let mut mesh = ChunkMesh::default();
        let _ = self.items.append_posed_blocks(&mut mesh, &[(pose, light_at, id.to_owned())], packs, atlas, light);
        mesh
    }

    /// bo2mc (survival): the held item used on the mob the player looks
    /// at within reach (food to breed or grow, shears, a bucket to milk).
    /// False when no mob is there.
    pub(crate) fn use_on_mob(&mut self, eye: DVec3, look: DVec3) -> bool {
        let Some((hit, _)) = self.world.mob_on_ray(eye, look, 3.0) else {
            return false;
        };
        self.server.mob_action(hit, None, &self.inventory.clone(), self.selected, false);
        true
    }

    /// The held item's attack on the mob the player looks at within reach
    /// (an empty hand deals one damage), at this attack strength.
    pub(crate) fn punch(&mut self, eye: DVec3, look: DVec3, yaw: f32, attack_damage: f32, strength: f32, critical: bool) -> bool {
        let Some((hit, _)) = self.world.mob_on_ray(eye, look, 3.0) else {
            return false;
        };
        let attack = minecraftoss_entities::world::PlayerAttack {
            player_id: PLAYER,
            position: eye - DVec3::Y * f64::from(EYE_HEIGHT),
            yaw,
            attack_damage: f64::from(attack_damage),
            strength,
            sprinting: false,
            can_critical: critical,
            can_sweep: false,
        };
        self.server.mob_action(hit, Some(attack), &self.inventory.clone(), self.selected, false);
        true
    }

    /// A block the player placed, for the level the mobs walk in.
    pub(crate) fn placed(&mut self, scene: &HandcraftedScene, pos: (i32, i32, i32)) {
        self.server.player_edit(scene, pos, PlayerEdit::Place);
    }

    /// bo2mc (survival): bone meal on a clicked face, grown by the level.
    pub(crate) fn bone_meal(&mut self, pos: (i32, i32, i32), face: &'static str) {
        self.server.bone_meal(pos, face);
    }

    /// Client ticks run so far.
    pub(crate) fn client_ticks(&self) -> u64 {
        self.ticks
    }

    /// Blocks the player's weapons broke, for the level the mobs walk in.
    pub(crate) fn broke(&mut self, scene: &HandcraftedScene, positions: &[(i32, i32, i32)]) {
        for &pos in positions {
            self.server.player_edit(scene, pos, PlayerEdit::Break);
        }
    }

    /// A bullet on a mob: an attack with the bullet's damage from where it
    /// was fired.
    pub(crate) fn shoot(&mut self, key: u64, damage: f32, from: [f64; 3], yaw: f32) {
        let Some(hit) = decode(key) else {
            return;
        };
        let attack = minecraftoss_entities::world::PlayerAttack {
            player_id: PLAYER,
            position: DVec3::from_array(from),
            yaw,
            attack_damage: f64::from(damage * HEALTH_SCALE),
            strength: 1.0,
            sprinting: false,
            can_critical: false,
            can_sweep: false,
        };
        self.server
            .mob_action(hit, Some(attack), &minecraftoss_player::inventory::Inventory::default(), 0, false);
    }

    /// Sends a server tick when one is due and takes what came back: block
    /// changes to show, and mob hits on the player in MW2 damage.
    /// bo2mc's souls round: angry wolves at these block points.
    pub(crate) fn summon_souls_wolves(&mut self, positions: Vec<[f64; 3]>) {
        self.server.summon_souls_wolves(positions);
    }

    /// A souls wolf that cannot reach him snaps to a block point.
    pub(crate) fn move_souls_wolf(&mut self, id: u64, to: [f64; 3]) {
        self.server.move_souls_wolf(id, to);
    }

    pub(crate) fn end_souls_wolves(&mut self) {
        self.server.end_souls_wolves();
    }

    /// bo2mc: Electric Cherry's shock: every living mob within `radius`
    /// blocks of `center` (a block point) takes `damage` Minecraft points.
    /// How many it reached.
    pub(crate) fn shock(&mut self, center: [f64; 3], radius: f64, damage: f32) -> usize {
        let c = DVec3::from_array(center);
        let mut n = 0;
        for (key, b) in self.boxes() {
            let mid = DVec3::new((b[0] + b[3]) * 0.5, (b[1] + b[4]) * 0.5, (b[2] + b[5]) * 0.5);
            if mid.distance(c) > radius + 0.5 {
                continue;
            }
            let Some(hit) = decode(key) else { continue };
            let attack = minecraftoss_entities::world::PlayerAttack {
                player_id: PLAYER,
                position: c,
                yaw: 0.0,
                attack_damage: f64::from(damage),
                strength: 1.0,
                sprinting: false,
                can_critical: false,
                can_sweep: false,
            };
            self.server.mob_action(hit, Some(attack), &minecraftoss_player::inventory::Inventory::default(), 0, false);
            n += 1;
        }
        n
    }

    /// The chat's `/summon`: a mob of `kind` (`minecraft:cow`) at a block
    /// point.
    pub(crate) fn summon(&mut self, kind: &str, at: [f64; 3]) {
        self.server.summon(kind.to_owned(), at, None);
    }

    /// A Minecraft lightning bolt at a block point in `delay` seconds.
    pub(crate) fn strike(&mut self, pos: [f64; 3], delay: f64) {
        self.bolts.strike(pos, delay, &mut self.random);
    }

    /// The souls wolves alive as the player tracks them, and where.
    pub(crate) fn souls_wolves(&self) -> Vec<(u64, [f64; 3])> {
        self.world
            .wolves()
            .iter()
            .filter(|w| w.ai.state.wolf.souls && w.wolf.health > 0.0)
            .map(|w| (w.id, w.wolf.body.position.to_array()))
            .collect()
    }

    pub(crate) fn tick(
        &mut self,
        dt: f64,
        day_ticks: i64,
        bright_outside: bool,
        player: &PlayerView,
    ) -> (Vec<((i32, i32, i32), Option<Block>)>, Vec<(i32, Option<[f64; 3]>)>) {
        self.clock += dt;
        self.bolts.frame(dt);
        let ticked = self.clock >= TICK_SECONDS;
        if ticked {
            self.clock = (self.clock - TICK_SECONDS).min(TICK_SECONDS);
            self.ticks += 1;
            // Packets first, then the client level's entity ticks.
            self.client.tick();
            for enderman in self.world.endermen() {
                if let Some(mob) = self.client.get(enderman.id) {
                    self.portal.emit_enderman(mob.position, 0.6, 2.9);
                }
            }
            self.portal.tick();
            self.bolts.tick(&mut self.random, &mut self.sounds);
            if self.ticks % 200 == 0 {
                // Monsters and how close the nearest is (do they hunt him?).
                let w = &self.world;
                let [fx, fy, fz] = player.feet;
                let mut monsters = 0;
                let mut hunting = 0;
                let mut near = f64::MAX;
                let mut near_goals = String::new();
                let mut note = |p: &minecraftoss_entities::movement::Body, ai: Option<&minecraftoss_entities::monster_ai::MonsterAi>| {
                    monsters += 1;
                    if ai.is_some_and(|ai| matches!(ai.state.target, Some(minecraftoss_entities::monster_ai::Target::Player(_)))) {
                        hunting += 1;
                    }
                    let (dx, dy, dz) = (p.position.x - fx, p.position.y - fy, p.position.z - fz);
                    let d = (dx * dx + dy * dy + dz * dz).sqrt();
                    if d < near {
                        near = d;
                        near_goals = ai.map_or("no ai".to_owned(), |ai| format!("{:?}, {dy:.0} up", ai.running_goals()));
                    }
                };
                w.zombies().iter().filter(|e| e.zombie.health > 0.0).for_each(|e| note(&e.zombie.body, e.ai.as_deref()));
                w.skeletons().iter().filter(|e| e.skeleton.health > 0.0).for_each(|e| note(&e.skeleton.body, e.ai.as_deref()));
                w.creepers().iter().filter(|e| e.creeper.health > 0.0 && !e.creeper.exploded).for_each(|e| note(&e.creeper.body, Some(&e.ai)));
                w.spiders().iter().filter(|e| e.spider.health > 0.0).for_each(|e| note(&e.spider.body, Some(&e.ai)));
                w.witches().iter().filter(|e| e.witch.health > 0.0).for_each(|e| note(&e.witch.body, Some(&e.ai)));
                let nearest = if monsters > 0 { format!("nearest {near:.0} blocks away ({near_goals})") } else { "none".to_owned() };
                let animals = w.cows().iter().filter(|e| e.cow.health > 0.0).count()
                    + w.sheep().iter().filter(|e| e.health > 0.0).count()
                    + w.pigs().iter().filter(|e| e.pig.health > 0.0).count()
                    + w.chickens().iter().filter(|e| e.chicken.health > 0.0).count();
                diag::info!(
                    World,
                    "Minecraft mobs: {} tracked, {animals} animals, {monsters} monsters, {hunting} hunting him, {} digging, {nearest}",
                    self.boxes().len(),
                    self.diggers.len()
                );
            }
            let feet = DVec3::from_array(player.feet);
            let center = ((feet.x.floor() as i32) >> 4, (feet.z.floor() as i32) >> 4);
            let candidate = PlayerCandidate {
                id: PLAYER,
                position: feet,
                eye_height: EYE_HEIGHT,
                main_hand_cow_food: false,
                offhand_cow_food: false,
                main_hand_pig_food: false,
                offhand_pig_food: false,
                main_hand_chicken_food: false,
                offhand_chicken_food: false,
                main_hand_carrot_on_a_stick: false,
                offhand_carrot_on_a_stick: false,
                main_hand_wolf_interest: false,
                offhand_wolf_interest: false,
                main_hand_horse_tempt: false,
                offhand_horse_tempt: false,
                alive: player.alive,
                spectator: !player.alive,
                attackable: player.alive,
            };
            let souls = sim::bo2mc::enabled() && sim::bo2mc::souls_fog();
            self.server.tick(TickInput {
                day_ticks,
                players: if player.alive { vec![player.feet] } else { Vec::new() },
                difficulty: self.difficulty,
                simulation_center: center,
                simulation_distance: 8,
                pickup: player
                    .alive
                    .then(|| (player.feet, Box::new(self.inventory.clone()), self.selected)),
                mob_players: vec![candidate],
                mob_views: vec![(
                    PLAYER,
                    minecraftoss_entities::enderman::PlayerView {
                        head_yaw: player.yaw,
                        pitch: player.pitch,
                        disguised: false,
                    },
                )],
                mob_vitals: vec![(
                    PLAYER,
                    minecraftoss_entities::monster_ai::PlayerVitals {
                        health: player.health * HEALTH_SCALE,
                        ..Default::default()
                    },
                )],
                bright_outside,
                tracking: (player.feet, 160.0),
                player_hurts: Vec::new(),
                spawn_mobs: self.spawn_mobs && !souls,
                // A souls round is dogs only: no night monsters join it.
                clear_monsters: souls,
                // Vulture Aid: Looting III on what he kills.
                looting: if sim::bo2mc::enabled() && sim::bo2mc::player_has_perk("specialty_nomotionsensor") { 3 } else { 0 },
            });
        }
        let mut changes = Vec::new();
        let mut hits = Vec::new();
        for output in self.server.poll() {
            self.server_handled = output.handled;
            if let Some(snapshot) = output.entities {
                self.server_snapshot = Some(snapshot);
            }
            self.server_picked.extend(output.picked);
            changes.extend(output.changes);
            for sound in &output.mob_sounds {
                self.sounds.push((sound.event.clone(), sound.position, sound.volume, sound.pitch));
            }
            for sound in output.mob_results.iter().flat_map(|result| result.sounds.iter()) {
                self.sounds.push((sound.event.clone(), sound.position, sound.volume, sound.pitch));
            }
            // What the mob took or gave (food eaten, milk, worn shears).
            for (slot, now) in output.mob_results.iter().flat_map(|result| result.slots.iter()) {
                if let Some(held) = self.inventory.slots.get_mut(*slot) {
                    *held = now.clone();
                }
            }
            // `ServerExplosion`'s sound: loud, pitched down.
            for blast in &output.explosions {
                let pitch = (1.0 + (self.random.next_float() - self.random.next_float()) * 0.2) * 0.7;
                self.sounds.push(("minecraft:entity.generic.explode".to_owned(), blast.position, 4.0, pitch));
            }
            if let Some(mobs) = output.mobs {
                self.world = *mobs;
                for (feet, width, height) in self.client.receive(server_mobs(&self.world)) {
                    self.poof.spawn(feet, width, height);
                }
            }
            for hit in output.player_hits.into_iter().filter(|h| h.player_id == PLAYER) {
                // PhD Flopper: a creeper's blast does not hurt him.
                if matches!(hit.kind, PlayerHitKind::Explosion { .. })
                    && sim::bo2mc::enabled()
                    && sim::bo2mc::player_has_perk("specialty_flakjacket")
                {
                    diag::info!(World, "bo2mc: PhD Flopper took a creeper blast");
                    continue;
                }
                let from = match hit.kind {
                    PlayerHitKind::Melee { attacker, .. } => Some(attacker.to_array()),
                    _ => None,
                };
                hits.push(((hit.damage / HEALTH_SCALE).round() as i32, from));
            }
        }
        if ticked {
            self.find_diggers(DVec3::from_array(player.feet));
            self.server_items_tick(DVec3::from_array(player.feet));
            // `ItemEntity.playerTouch`'s pickup pop.
            for _ in 0..self.world_items.take_pickup_sounds() {
                let pitch = ((self.random.next_float() - self.random.next_float()) * 0.7 + 1.0) * 2.0;
                self.sounds.push(("minecraft:entity.item.pickup".to_owned(), DVec3::from_array(player.feet), 0.2, pitch));
            }
        }
        (changes, hits)
    }

    /// Hunting monsters (not spiders, which climb) that have stood still
    /// for a second and a half, or half a second against a wall, while the
    /// player is out of reach: they are walled off and claw through.
    fn find_diggers(&mut self, feet: DVec3) {
        use minecraftoss_entities::monster_ai::{MonsterAi, Target};
        use minecraftoss_entities::movement::Body;
        let w = &self.world;
        let mut hunters: Vec<(u64, DVec3, bool)> = Vec::new();
        let mut add = |id: u64, body: &Body, ai: Option<&MonsterAi>| {
            if ai.is_some_and(|ai| matches!(ai.state.target, Some(Target::Player(_)))) {
                hunters.push((id, body.position, body.horizontal_collision));
            }
        };
        w.zombies().iter().filter(|e| e.zombie.health > 0.0).for_each(|e| add(e.id, &e.zombie.body, e.ai.as_deref()));
        w.skeletons().iter().filter(|e| e.skeleton.health > 0.0).for_each(|e| add(e.id, &e.skeleton.body, e.ai.as_deref()));
        w.creepers().iter().filter(|e| e.creeper.health > 0.0 && !e.creeper.exploded).for_each(|e| add(e.id, &e.creeper.body, Some(&e.ai)));
        w.witches().iter().filter(|e| e.witch.health > 0.0).for_each(|e| add(e.id, &e.witch.body, Some(&e.ai)));
        let mut stuck = HashMap::with_capacity(hunters.len());
        self.diggers.clear();
        for (id, p, against_wall) in hunters {
            let (dx, dy, dz) = (feet.x - p.x, feet.y - p.y, feet.z - p.z);
            let flat = (dx * dx + dz * dz).sqrt();
            let out_of_reach = flat > 2.0 || dy.abs() >= 2.0;
            let (anchor, still) = match self.stuck.get(&id) {
                Some(&(anchor, still)) if (p - anchor).with_y(0.0).length() < 0.4 => (anchor, still + 1),
                _ => (p, 0),
            };
            stuck.insert(id, (anchor, still));
            if out_of_reach && flat < 32.0 && (still >= 30 || (against_wall && still >= 10)) {
                self.diggers.push(p);
            }
        }
        self.stuck = stuck;
    }

    /// Every living mob's key and box in blocks, for bullets.
    pub(crate) fn boxes(&self) -> Vec<(u64, [f64; 6])> {
        let w = &self.world;
        let mut out = Vec::new();
        let mut add = |hit: MobHit, body: &minecraftoss_entities::movement::Body| {
            let p = body.position;
            let half = f64::from(body.width) * 0.5;
            out.push((
                encode(hit),
                [p.x - half, p.y, p.z - half, p.x + half, p.y + f64::from(body.height), p.z + half],
            ));
        };
        for e in w.bats().iter().filter(|e| e.bat.health > 0.0) {
            add(MobHit::Bat(e.id), &e.bat.body);
        }
        for e in w.zombies().iter().filter(|e| e.zombie.health > 0.0) {
            add(MobHit::Zombie(e.id), &e.zombie.body);
        }
        for e in w.skeletons().iter().filter(|e| e.skeleton.health > 0.0) {
            add(MobHit::Skeleton(e.id), &e.skeleton.body);
        }
        for e in w.creepers().iter().filter(|e| e.creeper.health > 0.0 && !e.creeper.exploded) {
            add(MobHit::Creeper(e.id), &e.creeper.body);
        }
        for e in w.spiders().iter().filter(|e| e.spider.health > 0.0) {
            add(MobHit::Spider(e.id), &e.spider.body);
        }
        for e in w.slimes().iter().filter(|e| e.slime.health > 0.0) {
            add(MobHit::Slime(e.id), &e.slime.body);
        }
        for e in w.endermen().iter().filter(|e| e.enderman.health > 0.0) {
            add(MobHit::Enderman(e.id), &e.enderman.body);
        }
        for e in w.witches().iter().filter(|e| e.witch.health > 0.0) {
            add(MobHit::Witch(e.id), &e.witch.body);
        }
        for e in w.iron_golems().iter().filter(|e| e.golem.health > 0.0) {
            add(MobHit::IronGolem(e.id), &e.golem.body);
        }
        for e in w.wolves().iter().filter(|e| e.wolf.health > 0.0) {
            add(MobHit::Wolf(e.id), &e.wolf.body);
        }
        for e in w.villagers().iter().filter(|e| e.villager.health > 0.0) {
            add(MobHit::Villager(e.id), &e.villager.body);
        }
        for e in w.cows().iter().filter(|e| e.cow.health > 0.0) {
            let hit = if e.mooshroom.is_some() { MobHit::Mooshroom(e.id) } else { MobHit::Cow(e.id) };
            add(hit, &e.cow.body);
        }
        for e in w.sheep().iter().filter(|e| e.health > 0.0) {
            add(MobHit::Sheep(e.id), &e.body);
        }
        for e in w.pigs().iter().filter(|e| e.pig.health > 0.0) {
            add(MobHit::Pig(e.id), &e.pig.body);
        }
        for e in w.chickens().iter().filter(|e| e.chicken.health > 0.0) {
            add(MobHit::Chicken(e.id), &e.chicken.body);
        }
        out
    }

    /// Steps the client-side puffs, which settle on the scene's blocks.
    pub(crate) fn tick_scene(&mut self, scene: &HandcraftedScene, ticks: u32) {
        for _ in 0..ticks {
            self.poof.tick(scene);
        }
    }

    /// The mobs this frame, drawn with the viewer's renderers.
    pub(crate) fn meshes(
        &mut self,
        scene: &HandcraftedScene,
        packs: &PackStack,
        atlas: &Atlas,
        light: &SkyLight,
        forward: Vec3,
        camera: DVec3,
        sky_darken: u8,
    ) -> MobMeshes {
        use minecraft_terrain::*;
        // Mobs appear on the client once their trackers start.
        self.client.spawn_missing(&server_mobs(&self.world));
        let w = &self.world;
        let poses = &self.client;
        let partial = (self.clock / TICK_SECONDS).clamp(0.0, 1.0) as f32;
        let mut out = MobMeshes::default();
        cow_render::append_cows(&mut out.models, w.cows().iter(), poses, atlas, light, partial);
        sheep_render::append_sheep(&mut out.models, w.sheep().iter(), poses, atlas, light, partial);
        pig_render::append_pigs(&mut out.models, w.pigs().iter(), poses, atlas, light, partial);
        chicken_render::append_chickens(&mut out.models, w.chickens().iter(), poses, atlas, light, partial);
        bat_render::append_bats(&mut out.culled, w.bats().iter(), poses, atlas, light, partial);
        let zombie_items = zombie_render::append_zombies(&mut out.models, w.zombies().iter(), poses, atlas, light, partial);
        creeper_render::append_creepers(&mut out.models, w.creepers().iter(), poses, atlas, light, partial);
        spider_render::append_spiders(&mut out.models, w.spiders().iter(), poses, atlas, light, partial);
        let skeleton_items =
            skeleton_render::append_skeletons(&mut out.models, w.skeletons().iter(), poses, atlas, light, partial);
        let villager_items = villager_render::append_villagers(
            &mut out.models,
            w.villagers().iter(),
            poses,
            &|pos| Scene::block(scene, pos).and_then(|b| b.properties.get("facing").cloned()),
            atlas,
            light,
            partial,
        );
        horse_render::append_horses(&mut out.models, &mut out.translucent, w.cows().iter(), poses, atlas, light, partial);
        slime_render::append_slimes(&mut out.models, &mut out.translucent, w.slimes().iter(), poses, atlas, light, partial);
        let carried = enderman_render::append_endermen(
            &mut out.models,
            w.endermen().iter(),
            poses,
            atlas,
            light,
            partial,
            self.ticks.rotate_left(32) ^ (partial.to_bits() as u64),
        );
        let witch_items = witch_render::append_witches(&mut out.models, w.witches().iter(), poses, atlas, light, partial);
        let poppies = golem_render::append_iron_golems(&mut out.models, w.iron_golems().iter(), poses, atlas, light, partial);
        wolf_render::append_wolves(&mut out.models, w.wolves().iter(), poses, atlas, light, partial, w.game_time());
        flame_render::append_flames(
            &mut out.items,
            w.burning()
                .into_iter()
                .map(|(previous, now, width, height)| (previous.lerp(now, f64::from(partial)), width, height)),
            atlas,
            forward,
            light,
        );
        witch_render::append_potions(
            &mut out.items,
            w.potions()
                .iter()
                .filter(|p| p.potion.alive)
                .map(|p| (p, p.previous_position.lerp(p.potion.position, f64::from(partial)))),
            atlas,
            forward,
            light,
        );
        if let Ok(items) = self.items.mesh(&self.world_items, packs, atlas, light, partial) {
            let base = out.items.vertices.len() as u32;
            out.items.vertices.extend(items.vertices);
            out.items.indices.extend(items.indices.iter().map(|i| i + base));
        }
        self.poof.append_mesh(&mut out.items, atlas, forward, partial, light);
        self.portal.append_mesh(&mut out.items, atlas, forward, partial, light);
        self.bolts.append_mesh(&mut out.items, atlas);
        let held: Vec<_> =
            skeleton_items.into_iter().chain(zombie_items).chain(villager_items).chain(witch_items).collect();
        let _ = self.items.append_held_items(&mut out.items, &held, packs, atlas, light);
        let carried: Vec<_> = carried.into_iter().chain(poppies).collect();
        let _ = self.items.append_posed_blocks(&mut out.items, &carried, packs, atlas, light);
        // Their shadows, at `getMaxLocalRawBrightness`.
        let casters = client_mobs::shadow_casters(w, poses, camera, partial);
        let raw = |pos: (i32, i32, i32)| light.get(pos).saturating_sub(sky_darken).max(light.get_block(pos));
        if let Ok(shadows) = mesh::entity_shadows(&casters, scene, atlas, &raw, 0.0) {
            out.shadows = shadows;
        }
        out
    }
}

/// The viewer's `server_bob_offset`: a server item's bob phase from its id.
fn bob_offset(id: i32) -> f32 {
    let hash = (id as u32).wrapping_mul(0x9e37_79b9).rotate_left(13).wrapping_mul(0x85eb_ca6b);
    hash as f32 / u32::MAX as f32 * std::f32::consts::TAU
}

/// The game's JAR with its loot tables and recipes.
pub(crate) fn data_jar() -> Option<std::path::PathBuf> {
    let root = assets::minecraft_map::root()?;
    // The client JAR fetched from Mojang, or the one a checkout's harness got.
    let fetched = root.join(assets::minecraft_setup::CLIENT_JAR);
    if fetched.is_file() {
        return Some(fetched);
    }
    let root = root.join("harness/.gradle/loom-cache/minecraftMaven/net/minecraft");
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("minecraft-common-") {
            continue;
        }
        for jar in std::fs::read_dir(entry.path().join("26.3")).ok()?.flatten() {
            if jar.path().extension().is_some_and(|ext| ext == "jar") {
                return Some(jar.path());
            }
        }
    }
    None
}

/// A mob hit as one number: the kind in the top byte, the id below.
fn encode(hit: MobHit) -> u64 {
    let kind: u64 = match hit {
        MobHit::Bat(_) => 0,
        MobHit::Zombie(_) => 1,
        MobHit::Skeleton(_) => 2,
        MobHit::Creeper(_) => 3,
        MobHit::Spider(_) => 4,
        MobHit::Slime(_) => 5,
        MobHit::Enderman(_) => 6,
        MobHit::Witch(_) => 7,
        MobHit::IronGolem(_) => 8,
        MobHit::Wolf(_) => 9,
        MobHit::Villager(_) => 10,
        MobHit::Cow(_) => 11,
        MobHit::Mooshroom(_) => 12,
        MobHit::Sheep(_) => 13,
        MobHit::Pig(_) => 14,
        MobHit::Chicken(_) => 15,
    };
    (kind << 56) | (hit.id() & ((1 << 56) - 1))
}

fn decode(key: u64) -> Option<MobHit> {
    let id = key & ((1 << 56) - 1);
    Some(match key >> 56 {
        0 => MobHit::Bat(id),
        1 => MobHit::Zombie(id),
        2 => MobHit::Skeleton(id),
        3 => MobHit::Creeper(id),
        4 => MobHit::Spider(id),
        5 => MobHit::Slime(id),
        6 => MobHit::Enderman(id),
        7 => MobHit::Witch(id),
        8 => MobHit::IronGolem(id),
        9 => MobHit::Wolf(id),
        10 => MobHit::Villager(id),
        11 => MobHit::Cow(id),
        12 => MobHit::Mooshroom(id),
        13 => MobHit::Sheep(id),
        14 => MobHit::Pig(id),
        15 => MobHit::Chicken(id),
        _ => return None,
    })
}
