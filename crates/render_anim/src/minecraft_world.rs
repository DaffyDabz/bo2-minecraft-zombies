//! The Minecraft map at run time: MinecraftOSS generates and streams a seeded
//! world around the local player, whose chunks become block collision and
//! whose section meshes go to the renderer. The world sits with its player
//! spawn at map origin, a block to `sim::voxel::BLOCK` map units.
use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use bevy::input::gamepad::GamepadButton;
use bevy::prelude::*;
use minecraft_terrain::clouds::CloudMask;
use minecraft_terrain::day_cycle::{DayCycle, Skybox};
use minecraft_terrain::environment::{DimensionEnvironment, View};
use minecraft_terrain::lighting::SkyLight;
use minecraft_terrain::mesh::{Atlas, SectionMesh, Vertex};
use minecraft_terrain::pack::PackStack;
use minecraft_terrain::scene::HandcraftedScene;
use minecraft_terrain::sections::{CullCamera, SectionPos};
use minecraft_terrain::terrain::{Dimension, TerrainStream};
use minecraftoss_core::BlockStateId;
use minecraftoss_core::registries::{DataPaths, Registries};

mod bo2mc_world;

const VIEW_DISTANCE: i32 = 8;
const TICK_SECONDS: f64 = 1.0 / 20.0;
/// Blocks on a side of the light volume MW2 models are lit from.
pub const LIGHT_VOLUME: i32 = 64;
/// Chunk sections fade in over this long, as the viewer's default option.
const FADE_MILLIS: u64 = 750;
/// Minecraft Zombies: blocks around the world spawn that must be in before
/// the player is let in (the spawn room, its yard and some ground beyond).
const SPAWN_READY_BLOCKS: i32 = 24;

/// What the renderer takes from the world each frame.
#[derive(Resource, Default)]
pub struct MinecraftWorldView {
    /// A Minecraft map is loaded: the stand-in map's world is not drawn.
    pub active: bool,
    /// Block point at map origin.
    pub origin: [f64; 3],
    pub atlas: Option<Arc<Atlas>>,
    pub uploads: Vec<(SectionPos, SectionMesh)>,
    pub removed: Vec<SectionPos>,
    pub visible: Vec<(SectionPos, f32)>,
    /// Bumped when the world is replaced, so stale sections are dropped.
    pub generation: u64,
    /// The environment uniform of MinecraftOSS for this frame, in block space.
    pub environment: [[f32; 4]; 16],
    /// Sun and the eight moon phases, 32 pixels each, side by side.
    pub celestial: Option<Arc<image::RgbaImage>>,
    pub clouds: Option<Arc<(Vec<Vertex>, Vec<u32>)>>,
    /// Sky and block light around the player: origin block, then
    /// `LIGHT_VOLUME` cubed pairs, x fastest then z then y.
    pub light_volume: Option<Arc<([i32; 3], Vec<u8>)>>,
    /// Sky and block light at the eye, for the view model.
    pub eye_light: [f32; 2],
    /// Break particles as section vertices and indices, rebuilt each frame.
    pub particles: (Vec<u8>, Vec<u32>),
    /// Destroy stage cubes over blocks being mined: position, strip uv.
    pub cracks: (Vec<[f32; 5]>, Vec<u32>),
    /// The ten destroy stages side by side.
    pub crack_texture: Option<Arc<image::RgbaImage>>,
    /// Mob models (cut out, back-face culled, translucent) and entity
    /// shadows, as `mesh::Vertex` bytes and indices.
    pub entity_meshes: [(Vec<u8>, Vec<u32>); 4],
    /// The black card behind the inventory's character, in blocks.
    pub backdrop: Option<[[f32; 3]; 4]>,
    /// The first-person hand or held item: section vertex bytes in view
    /// space (x right, y up, z back) and indices, with the projection that
    /// draws them (vanilla's fixed 70 degree hand field of view).
    pub hand: (Vec<u8>, Vec<u32>),
    pub hand_clip: [f32; 16],
}

struct Loaded {
    stream: TerrainStream,
    scene: HandcraftedScene,
    packs: PackStack,
    atlas: Arc<Atlas>,
    registries: Arc<Registries>,
    seed: i64,
    environment: DimensionEnvironment,
    celestial: Arc<image::RgbaImage>,
    cloud_mask: Option<CloudMask>,
    crack_texture: Arc<image::RgbaImage>,
}

/// The hand's swing and the timers of mining and placing by hand.
#[derive(Default)]
struct HandState {
    /// Ticks into a swing, while one runs.
    swing: Option<f32>,
    /// Seconds towards the next hand-mining and placing tick.
    clock: f64,
    /// Ticks until another placement while the button is held
    /// (`rightClickDelay`).
    place_delay: u32,
}

/// The player's walk, for vanilla's step and fall sounds.
#[derive(Default)]
struct StepState {
    last: Option<[f64; 3]>,
    /// `Entity.moveDist` and `nextStep`.
    move_dist: f32,
    next_step: f32,
    /// The highest point since leaving the ground.
    air_peak: Option<f64>,
}

#[derive(Default)]
struct Runtime {
    loading: Option<mpsc::Receiver<Result<Loaded, String>>>,
    world: Option<Loaded>,
    day: DayCycle,
    environment_accumulator: f64,
    environment_primed: bool,
    light: Option<SkyLight>,
    light_volume_at: Option<[i32; 3]>,
    light_volume_age: u32,
    cloud_center: Option<(i32, i32)>,
    mining: crate::minecraft_mining::Mining,
    minimap: crate::minecraft_minimap::Minimap,
    hand: HandState,
    steps: StepState,
    sounds: Option<crate::minecraft_sounds::Sounds>,
    inventory_ui: crate::minecraft_inventory::InventoryUi,
    entities: Option<crate::minecraft_entities::Entities>,
    shapes: ShapeCache,
    was_alive: bool,
    /// Minecraft Zombies: Black Ops II's Nuketown Zombies on this world.
    bo2mc: bool,
    /// The chunks around the world spawn are in (generated, with collision),
    /// so the player can be let in.
    spawn_ready: bool,
    /// Minecraft Zombies' spawn room and the rules' requests.
    bo2mc_world: bo2mc_world::Bo2mcWorld,
}

/// Collision shape ids of block states, as handed to `sim::voxel`.
#[derive(Default)]
struct ShapeCache {
    /// Shape id of each block state already seen.
    shapes: HashMap<BlockStateId, u16>,
    /// Boxes of each shape id, to reuse an id for a repeated shape.
    shape_ids: HashMap<Vec<[u32; 6]>, u16>,
}

impl ShapeCache {
    /// The collision shape id of a block state (0 = no collision), adding
    /// its boxes to the block world the first time a shape is seen.
    fn shape_id(&mut self, registries: &Registries, state: BlockStateId) -> u16 {
        if let Some(&id) = self.shapes.get(&state) {
            return id;
        }
        let boxes = registries.blocks.collision_boxes(state);
        let id = if boxes.is_empty() {
            0
        } else {
            let key: Vec<[u32; 6]> = boxes.iter().map(|b| b.map(|v| (v as f32).to_bits())).collect();
            match self.shape_ids.get(&key) {
                Some(&id) => id,
                None => {
                    let boxes32 = boxes.iter().map(|b| b.map(|v| v as f32)).collect();
                    let id = sim::voxel::add_shapes(vec![boxes32]).unwrap_or(0);
                    self.shape_ids.insert(key, id);
                    id
                }
            }
        };
        self.shapes.insert(state, id);
        id
    }
}

/// The player's MW2 body stands in the inventory's character window, on a
/// black card: placed from this frame's camera and projection where the
/// window shows, facing the camera, turned and aiming towards the mouse,
/// and drawn with the card in the view model's depth band so no wall comes
/// between.
pub(crate) fn place_inventory_puppet(
    ui: Res<frame::MinecraftUi>,
    mut puppet: ResMut<frame::InventoryPuppet>,
    mut view: ResMut<MinecraftWorldView>,
    local: Res<net::LocalPresentClient>,
    presented: Res<net::PresentedSnapshot>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<&Transform, With<render_scene::FlyCamera>>,
    lenses: Query<&Projection, With<render_scene::FpvLens>>,
) {
    use std::sync::atomic::Ordering::Relaxed;
    puppet.active = false;
    puppet.bo2_entnum = None;
    view.backdrop = None;
    render_frame::DEPTH_HACK_SCENE_ENTNUM.store(u32::MAX, Relaxed);
    let alive = presented.player(local.0).is_some_and(|ps| ps.pm_type == 0);
    if !(ui.active && ui.inventory_open && alive) || ui.character_box.is_none() {
        if sim::bo2mc::enabled() {
            sim::bo2mc::set_puppet(None);
        }
        return;
    }
    let (Some([cx, cy, box_w, box_h]), Ok(window), Ok(camera), Ok(Projection::Perspective(lens))) =
        (ui.character_box, windows.single(), cameras.single(), lenses.single())
    else {
        return;
    };
    let (w, h) = (window.width().max(1.0), window.height().max(1.0));
    let tan_v = (lens.fov * 0.5).tan();
    let tan_h = tan_v * if lens.aspect_ratio > 1e-3 { lens.aspect_ratio } else { w / h };
    let (eye, fwd, right, up) = (camera.translation, *camera.forward(), *camera.right(), *camera.up());
    // A point `distance` along the view at a window pixel.
    let at = |px: f32, py: f32, distance: f32| {
        let (nx, ny) = (px / w * 2.0 - 1.0, 1.0 - py / h * 2.0);
        eye + (fwd + right * (nx * tan_h) + up * (ny * tan_v)) * distance
    };
    let distance = 10.0;
    let world_h = box_h / h * 2.0 * tan_v * distance;
    // A standing MW2 player is about 72 units: most of the window.
    let scale = world_h * 0.82 / 72.0;
    let feet = at(cx, cy + box_h * 0.5, distance) + up * (world_h * 0.07);
    // Turned by the mouse as vanilla's
    // `InventoryScreen.renderEntityInInventoryFollowsMouse` turns its body.
    let turn = (ui.gaze[0] * 1.2).atan() * 0.7;
    let x_axis = (-fwd * turn.cos() + right * turn.sin()).normalize_or(-fwd);
    let y_axis = up.cross(x_axis);
    puppet.root = Mat4::from_cols(
        (x_axis * scale).extend(0.0),
        (y_axis * scale).extend(0.0),
        (up * scale).extend(0.0),
        feet.extend(1.0),
    );
    puppet.pitch = (ui.gaze[1] * 1.2).atan().to_degrees() * 0.6;
    puppet.client = local.0.0;
    if sim::bo2mc::enabled() {
        // Minecraft Zombies (playtest 1): his BO2 body (the rules side's
        // model of him, playing a standing idle) stands at the root above
        // instead of the MW2 one; the rules side keeps it near him (the
        // spot only has to be on his screen).
        sim::bo2mc::set_puppet(Some(sim::bo2mc::Puppet {
            origin: feet.to_array(),
            yaw: x_axis.y.atan2(x_axis.x).to_degrees(),
        }));
        puppet.bo2_entnum = sim::bo2mc::puppet_number();
        render_frame::BACKDROP_IN_SCENE.store(true, Relaxed);
    } else {
        puppet.active = true;
        render_frame::DEPTH_HACK_SCENE_ENTNUM.store(local.0.0, Relaxed);
        render_frame::BACKDROP_IN_SCENE.store(false, Relaxed);
    }
    // The card: the window and a margin the panel covers, behind it.
    let corner = |dx: f32, dy: f32| {
        let p = at(cx + dx * box_w * 0.6, cy + dy * box_h * 0.6, distance * 1.4);
        let b = sim::voxel::to_block(view.origin, p.to_array());
        [b[0] as f32, b[1] as f32, b[2] as f32]
    };
    view.backdrop = Some([corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)]);
}

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        Update,
        place_inventory_puppet
            .after(crate::sync_camera_from_presented)
            .after(frame::PresentedPublished)
            .in_set(frame::ClientSet::Present),
    );
    app.init_resource::<MinecraftWorldView>()
        .init_resource::<frame::MinecraftUi>()
        .init_resource::<frame::InventoryPuppet>()
        .insert_non_send(Runtime::default())
        .add_systems(
            Update,
            update
                .after(frame::PresentedPublished)
                .in_set(frame::ClientSet::Present),
        );
}

fn load(seed: i64) -> Result<Loaded, String> {
    let root = assets::minecraft_map::root().ok_or_else(assets::minecraft_setup::status)?;
    let paths = DataPaths::under(&root);
    let registries = Arc::new(Registries::load(&paths)?);
    let packs = PackStack::open(vec![root.join("resourcepacks/local/minecraft-26.3")])
        .map_err(|e| e.to_string())?;
    let stream = TerrainStream::for_dimension(
        registries.clone(),
        seed,
        VIEW_DISTANCE,
        Dimension::Overworld,
        None,
    )
    .map_err(|e| e.to_string())?;
    let build = minecraft_terrain::mesh::build(&HandcraftedScene::default(), &packs)
        .map_err(|e| e.to_string())?;
    let scene = HandcraftedScene::streamed(stream.states.clone());
    let environment =
        DimensionEnvironment::load(&registries, Dimension::Overworld.dimension_type())?;
    let celestial = Arc::new(celestial_image(&packs).map_err(|e| e.to_string())?);
    let cloud_mask = CloudMask::from_pack(&packs).ok();
    let crack_texture = Arc::new(crate::minecraft_mining::crack_strip(&packs).map_err(|e| e.to_string())?);
    Ok(Loaded {
        stream,
        scene,
        packs,
        atlas: build.atlas,
        registries,
        seed,
        environment,
        celestial,
        cloud_mask,
        crack_texture,
    })
}

/// The sky's sun and moon phases, laid out as MinecraftOSS lays them out.
fn celestial_image(packs: &PackStack) -> anyhow::Result<image::RgbaImage> {
    let mut celestial = image::RgbaImage::new(32 * 9, 32);
    let names = [
        "environment/celestial/sun",
        "environment/celestial/moon/full_moon",
        "environment/celestial/moon/waning_gibbous",
        "environment/celestial/moon/third_quarter",
        "environment/celestial/moon/waning_crescent",
        "environment/celestial/moon/new_moon",
        "environment/celestial/moon/waxing_crescent",
        "environment/celestial/moon/first_quarter",
        "environment/celestial/moon/waxing_gibbous",
    ];
    for (index, path) in names.iter().enumerate() {
        let id = minecraft_terrain::pack::ResourceId::parse(&format!("minecraft:{path}"))?;
        if let Some(bytes) = packs.texture(&id)? {
            let img =
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8();
            let tile =
                image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Nearest);
            image::imageops::replace(&mut celestial, &tile, (index as i64) * 32, 0);
        }
    }
    Ok(celestial)
}

/// A controller trigger, held or just pressed: with a block or an empty hand
/// the right trigger mines and the left places, as the mouse buttons do.
fn pad_trigger(pad: Option<&bevy::input::gamepad::Gamepad>, button: GamepadButton, just: bool) -> bool {
    pad.is_some_and(|pad| if just { pad.just_pressed(button) } else { pad.pressed(button) })
}

/// The clock a world starts at: `IW4L_MINECRAFT_TIME`, else nightfall
/// (13000) for Minecraft Zombies.
fn start_ticks(bo2mc: bool) -> Option<f64> {
    std::env::var("IW4L_MINECRAFT_TIME")
        .ok()
        .and_then(|t| t.trim().parse::<f64>().ok())
        .or(bo2mc.then_some(13_000.0))
}

/// bo2mc: `IW4L_BO2MC_DAY_SPEED` times the day clock (tests).
fn day_speed() -> f64 {
    static SPEED: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SPEED.get_or_init(|| {
        std::env::var("IW4L_BO2MC_DAY_SPEED").ok().and_then(|s| s.trim().parse().ok()).filter(|s: &f64| *s > 0.0).unwrap_or(1.0)
    })
}

fn seed() -> i64 {
    if let Some(seed) = std::env::var("IW4L_MINECRAFT_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return seed;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    (nanos as i64) ^ 0x5DEE_CE66_D1CE_4E5B
}

#[allow(clippy::too_many_arguments)]
fn update(
    time: Res<Time>,
    mut installed: MessageReader<frame::MatchInstalled>,
    mut torn_down: MessageReader<frame::MatchTornDown>,
    local: Res<net::LocalPresentClient>,
    presented: Res<net::PresentedSnapshot>,
    authority: Option<ResMut<net::AuthorityWorld>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    (mut ui, mut puppet, mut images, mut sound_queue, buttons): (
        ResMut<frame::MinecraftUi>,
        ResMut<frame::InventoryPuppet>,
        ResMut<Assets<Image>>,
        ResMut<audio::McSoundQueue>,
        Res<ButtonInput<MouseButton>>,
    ),
    mut view: ResMut<MinecraftWorldView>,
    mut runtime: NonSendMut<Runtime>,
    (skate, cameras, gamepads, active_pad, cmd_template): (
        Res<frame::SkateMode>,
        Query<&Transform, With<render_scene::FlyCamera>>,
        Query<&bevy::input::gamepad::Gamepad>,
        Option<Res<frame::ActivePad>>,
        Option<Res<net::ClientCmdTemplate>>,
    ),
) {
    let pad = active_pad.and_then(|active| active.0).and_then(|entity| gamepads.get(entity).ok());
    for _ in torn_down.read() {
        stop(&mut runtime, &mut view);
    }
    ui.loading_world = runtime.loading.is_some() || (runtime.bo2mc && !runtime.spawn_ready);
    for match_ in installed.read() {
        stop(&mut runtime, &mut view);
        if assets::minecraft_map::has_minecraft_world(&match_.zone) {
            let seed = seed();
            runtime.bo2mc = assets::minecraft_map::is_bo2mc_load(&match_.zone);
            diag::info!(World, "Minecraft world: seed {seed} bo2mc {}", runtime.bo2mc);
            // A new game (a restart too) is the same world fresh: nothing
            // of the last one (room, requests, a paused clock) carries over
            // while this one loads.
            if runtime.bo2mc {
                sim::bo2mc::reset();
                sim::bo2mc::set_day(start_ticks(true).unwrap_or(13_000.0), false);
            }
            ui.loading_world = true;
            let (send, receive) = mpsc::channel();
            let _ = std::thread::Builder::new()
                .name("minecraft-world-load".into())
                .spawn(move || {
                    let _ = send.send(load(seed));
                });
            runtime.loading = Some(receive);
            view.active = true;
        }
    }
    let Some(mut authority) = authority else {
        return;
    };

    if let Some(receive) = &runtime.loading
        && let Ok(result) = receive.try_recv()
    {
        runtime.loading = None;
        match result {
            Ok(world) => {
                let (x, y, z) = world.stream.player_spawn;
                view.origin = [x, y, z];
                view.atlas = Some(world.atlas.clone());
                view.celestial = Some(world.celestial.clone());
                view.crack_texture = Some(world.crack_texture.clone());
                runtime.mining = Default::default();
                runtime.sounds = Some(crate::minecraft_sounds::Sounds::load(&world.packs));
                let mut entities = crate::minecraft_entities::Entities::new(&world.stream, world.seed);
                // Minecraft Zombies has no Minecraft monsters; animals stay.
                if runtime.bo2mc {
                    entities.difficulty = 0;
                }
                runtime.entities = Some(entities);
                runtime.spawn_ready = false;
                runtime.day = DayCycle::default();
                // Game ticks since sunrise to start at: 6000 noon, 13000
                // dusk, 18000 midnight. Minecraft Zombies starts at
                // nightfall.
                if let Some(ticks) = start_ticks(runtime.bo2mc) {
                    runtime.day.set(ticks);
                }
                runtime.environment_accumulator = 0.0;
                runtime.environment_primed = false;
                runtime.light = Some(SkyLight::streamed());
                runtime.light_volume_at = None;
                runtime.cloud_center = None;
                view.generation += 1;
                sim::voxel::activate(
                    authority.0.content().clip_brushes(),
                    view.origin,
                    vec![Vec::new()],
                );
                diag::info!(
                    World,
                    "Minecraft world ready: seed {} spawn {:?}",
                    world.seed,
                    world.stream.player_spawn
                );
                runtime.world = Some(world);
                runtime.shapes = ShapeCache::default();
                runtime.bo2mc_world = Default::default();
                runtime.was_alive = false;
            }
            Err(error) => {
                diag::warn!(World, "Minecraft world failed to load: {error}");
                view.active = false;
            }
        }
    }

    let origin = view.origin;
    let Runtime {
        world,
        shapes,
        bo2mc_world,
        was_alive,
        day,
        environment_accumulator,
        environment_primed,
        light,
        light_volume_at,
        light_volume_age,
        cloud_center,
        mining,
        entities,
        inventory_ui,
        sounds,
        hand,
        minimap,
        steps,
        bo2mc,
        spawn_ready,
        ..
    } = &mut *runtime;
    let Some(world) = world.as_mut() else {
        ui.active = false;
        ui.inventory_open = false;
        puppet.active = false;
        return;
    };
    // Before the player is in, the world streams around its spawn.
    let presented_player = presented.player(local.0);
    let feet = presented_player.map_or(origin, |ps| sim::voxel::to_block(origin, ps.origin));
    let block = (
        feet[0].floor() as i32,
        feet[1].floor() as i32,
        feet[2].floor() as i32,
    );
    let (loaded, forgotten) = world.stream.server_tick(block, &mut world.scene);
    for chunk in loaded {
        let (min_y, height) = (chunk.min_y(), chunk.height());
        let mut ids = vec![0u16; (height * 256) as usize];
        // Runs of one state (air, stone) skip the map.
        let mut last = None;
        for y in 0..height {
            for z in 0..16usize {
                for x in 0..16usize {
                    let state = chunk.block(x, min_y + y, z);
                    if let Some((last_state, id)) = last
                        && last_state == state
                    {
                        ids[((y as usize * 16) + z) * 16 + x] = id;
                        continue;
                    }
                    let id = shapes.shape_id(&world.registries, state);
                    last = Some((state, id));
                    ids[((y as usize * 16) + z) * 16 + x] = id;
                }
            }
        }
        if let Some(entities) = entities.as_mut() {
            entities.load_chunk(&chunk);
        }
        sim::voxel::set_chunk(
            chunk.pos.x,
            chunk.pos.z,
            sim::voxel::VoxelChunk {
                min_y,
                height,
                shapes: ids,
            },
        );
    }
    for pos in forgotten {
        sim::voxel::remove_chunk(pos.x, pos.z);
        if let Some(entities) = entities.as_mut() {
            entities.unload_chunk(pos);
        }
    }

    // Minecraft Zombies: the loading screen holds until the ground around
    // the spawn (the spawn room and its yard) is in.
    if *bo2mc && !*spawn_ready {
        let (x, z) = (origin[0].floor() as i32, origin[2].floor() as i32);
        let reach = SPAWN_READY_BLOCKS;
        *spawn_ready = ((x - reach) >> 4..=(x + reach) >> 4)
            .all(|cx| ((z - reach) >> 4..=(z + reach) >> 4).all(|cz| world.scene.generated_chunk((cx, cz)).is_some()));
        if *spawn_ready {
            diag::info!(World, "bo2mc: spawn ground in, origin {origin:?}");
        }
    }
    ui.loading_world = *bo2mc && !*spawn_ready;
    if *bo2mc && *spawn_ready && !bo2mc_world.built {
        let spawn = (origin[0].floor() as i32, origin[1].floor() as i32, origin[2].floor() as i32);
        bo2mc_world.build_room(world, shapes, entities.as_mut(), spawn);
        if let Some(entities) = entities.as_mut() {
            bo2mc_world::Bo2mcWorld::starting_items(entities);
            // IW4L_BO2MC_TEST_KIT=1 (tests): a crafting table in the room,
            // iron armor worn, five steaks; a furnace, a chest, water, lava.
            if std::env::var("IW4L_BO2MC_TEST_KIT").is_ok_and(|v| v == "1") {
                // A full set of iron armor worn (15 armor points).
                entities.inventory.slots[36] = Some(entities.inventory.recipes.stack("minecraft:iron_boots", 1));
                entities.inventory.slots[37] = Some(entities.inventory.recipes.stack("minecraft:iron_leggings", 1));
                entities.inventory.slots[38] = Some(entities.inventory.recipes.stack("minecraft:iron_chestplate", 1));
                entities.inventory.slots[39] = Some(entities.inventory.recipes.stack("minecraft:iron_helmet", 1));
                entities.inventory.slots[9] = Some(entities.inventory.recipes.stack("minecraft:cooked_beef", 5));
                let table = (spawn.0 + 3, spawn.1, spawn.2 - 2);
                bo2mc_world::set_blocks(world, shapes, Some(entities), vec![(table, Some(minecraft_terrain::scene::Block::new("minecraft:crafting_table")))]);
                // A furnace and a chest beside it, raw iron and coal to smelt.
                entities.inventory.slots[12] = Some(entities.inventory.recipes.stack("minecraft:raw_iron", 8));
                entities.inventory.slots[13] = Some(entities.inventory.recipes.stack("minecraft:coal", 8));
                let furnace = (spawn.0 + 3, spawn.1, spawn.2 + 2);
                let chest = (spawn.0 + 3, spawn.1, spawn.2);
                bo2mc_world::set_blocks(
                    world,
                    shapes,
                    Some(entities),
                    vec![
                        (furnace, Some(minecraft_terrain::scene::Block::new("minecraft:furnace").with("facing", "west").with("lit", "false"))),
                        (chest, Some(minecraft_terrain::scene::Block::new("minecraft:chest").with("facing", "west").with("type", "single").with("waterlogged", "false"))),
                        // Two blocks of water to drown in, a lava block.
                        ((spawn.0 - 6, spawn.1, spawn.2 + 4), Some(minecraft_terrain::scene::Block::new("minecraft:water").with("level", "0"))),
                        ((spawn.0 - 6, spawn.1 + 1, spawn.2 + 4), Some(minecraft_terrain::scene::Block::new("minecraft:water").with("level", "0"))),
                        ((spawn.0 - 6, spawn.1, spawn.2 - 4), Some(minecraft_terrain::scene::Block::new("minecraft:lava").with("level", "0"))),
                    ],
                );
            }
        }
    }

    let Some(ps) = presented_player else {
        ui.active = false;
        puppet.active = false;
        return;
    };

    // Every spawn lands on the Minecraft spawn once its ground exists.
    let alive = ps.pm_type == 0;
    let spawn_chunk = ((origin[0].floor() as i32) >> 4, (origin[2].floor() as i32) >> 4);
    if alive && !*was_alive && world.scene.generated_chunk(spawn_chunk).is_some() {
        // Retried each frame until the authority has the player to move.
        if authority.0.teleport(local.0, [0.0, 0.0, 0.0]) {
            diag::info!(World, "Minecraft spawn: moved to the world spawn");
            *was_alive = true;
        }
    } else if !alive {
        *was_alive = false;
    }

    // Minecraft's yaw: 0 facing +Z (map -Y), turning towards -X.
    let yaw_rad = ps.viewangles[1].to_radians();
    let mc_yaw = (-yaw_rad.cos()).atan2(-yaw_rad.sin()).to_degrees();
    let mut all_events = sim::voxel::take_events();

    // The hand, when no gun is selected: vanilla's left click mines by hand
    // (with the hand's break speed) or punches, its right click places the
    // held block (`Player.place_selected`, vanilla's placement states).
    // Footsteps (`Entity.applyMovementEmissionAndPlaySound`): the walked
    // distance grows by 0.6 of each horizontal move on the ground, and past
    // the next step the block under the feet sounds its step at 0.15 of its
    // volume. A fall of more than three blocks lands with the block's fall
    // sound and the player's (`LivingEntity.causeFallDamage`).
    let on_ground = alive && ps.ground_entity_num != playerstate_iw4::ENTITYNUM_NONE;
    if let Some(last) = steps.last.filter(|_| alive) {
        let horizontal = ((feet[0] - last[0]).hypot(feet[2] - last[2]) * 0.6) as f32;
        let block_at = |dy: f64| {
            let pos = (feet[0].floor() as i32, (feet[1] - dy).floor() as i32, feet[2].floor() as i32);
            minecraft_terrain::scene::Scene::block(&world.scene, pos).cloned().map(|b| (pos, b))
        };
        let under = block_at(0.2);
        let centre = |pos: (i32, i32, i32)| {
            Vec3::from_array(sim::voxel::to_map(origin, [pos.0 as f64 + 0.5, pos.1 as f64 + 1.0, pos.2 as f64 + 0.5]))
        };
        if on_ground && horizontal < 2.0 {
            steps.move_dist += horizontal;
            if steps.move_dist > steps.next_step
                && let Some((pos, block)) = under.as_ref()
            {
                steps.next_step = steps.move_dist as i32 as f32 + 1.0;
                // Snow layers and carpets sound instead of what they lie on.
                let inside = block_at(-0.01).filter(|(_, b)| {
                    let p = b.id.path.as_str();
                    p == "snow" || p.ends_with("_carpet") || p == "moss_carpet"
                });
                let (pos, block) = inside.as_ref().map_or((*pos, block), |(p, b)| (*p, b));
                if let (Some(sounds), Some(kind)) = (sounds.as_mut(), world.scene.sound_type(block)) {
                    sounds.play(&world.packs, &kind.step, Some(centre(pos)), kind.volume * 0.15, kind.pitch);
                }
            }
        }
        if on_ground {
            if let Some(peak) = steps.air_peak.take() {
                let fall = peak - feet[1];
                if fall > 3.0
                    && let Some(sounds) = sounds.as_mut()
                {
                    let event = if fall > 7.0 { "minecraft:entity.player.big_fall" } else { "minecraft:entity.player.small_fall" };
                    sounds.play(&world.packs, event, None, 1.0, 1.0);
                    if let Some((pos, block)) = under.as_ref()
                        && let Some(kind) = world.scene.sound_type(block)
                    {
                        sounds.play(&world.packs, &kind.fall, Some(centre(*pos)), kind.volume * 0.5, kind.pitch * 0.75);
                    }
                }
            }
        } else {
            steps.air_peak = Some(steps.air_peak.map_or(feet[1], |p| p.max(feet[1])));
        }
    } else {
        steps.air_peak = None;
    }
    steps.last = alive.then_some(feet);

    let dt_hand = time.delta_secs_f64();
    if let Some(ticks) = hand.swing.as_mut() {
        *ticks += (dt_hand * 20.0) as f32;
        if *ticks >= crate::minecraft_hand::SWING_TICKS {
            hand.swing = None;
        }
    }
    // Blocks the hand (or a held tool) worked on this frame, for their loot.
    let mut hand_hits = std::collections::HashSet::new();
    let holding = entities.as_ref().is_some_and(|e| {
        e.inventory.slots[e.selected].as_ref().is_none_or(|s| crate::minecraft_inventory::weapon_of(s).is_none())
    });
    ui.holding_item = alive && holding;
    ui.empty_hand = ui.holding_item
        && entities.as_ref().is_some_and(|e| e.inventory.slots[e.selected].is_none());
    hand.clock += dt_hand;
    let hand_ticks = (hand.clock / TICK_SECONDS) as u32;
    hand.clock -= f64::from(hand_ticks) * TICK_SECONDS;
    // The knife item mines too (bo2mc), as it swings.
    if let Some(entities) = entities.as_mut()
        && (ui.holding_item || (*bo2mc && ui.held_action == 2 && alive))
        && !ui.inventory_open
    {
        let mut player = minecraftoss_player::Player::new(glam::DVec3::from_array(feet));
        player.yaw = f64::from(mc_yaw);
        player.pitch = f64::from(ps.viewangles[0]);
        player.selected = entities.selected;
        let eye_block = glam::DVec3::from_array(feet) + glam::DVec3::Y * 1.62;
        let look = {
            let (yaw, pitch) = (f64::from(mc_yaw).to_radians(), f64::from(ps.viewangles[0]).to_radians());
            glam::DVec3::new(-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos())
        };
        if buttons.just_pressed(MouseButton::Left) || pad_trigger(pad, GamepadButton::RightTrigger2, true) {
            hand.swing = Some(0.0);
            entities.punch(eye_block, look, mc_yaw);
        }
        if buttons.pressed(MouseButton::Left) || pad_trigger(pad, GamepadButton::RightTrigger2, false) {
            for _ in 0..hand_ticks {
                if let Some(hit) = player.target(&world.scene, 4.5) {
                    // A hand mines as vanilla's `getDestroyProgress`: a
                    // block's hardness times thirty ticks. bo2mc
                    // (survival): times the held tool's speed when it is
                    // the block's tool, and a block that needs a tool takes
                    // a hundred ticks a hardness without one.
                    let factor = if *bo2mc {
                        let held = entities.inventory.slots[entities.selected].as_ref().map_or("", |s| s.id.as_str());
                        world.scene.state_at(hit.pos).map_or(1.0 / 30.0, |state| {
                            let (speed, correct) = bo2mc_world::tool_speed(&world.registries, state, held);
                            speed / if correct { 30.0 } else { 100.0 }
                        })
                    } else {
                        1.0 / 30.0
                    };
                    hand_hits.insert(hit.pos);
                    all_events.push(sim::voxel::VoxelEvent::Shot {
                        block: [hit.pos.0, hit.pos.1, hit.pos.2],
                        damage: 160.0 * factor,
                    });
                    if hand.swing.is_none_or(|t| t >= crate::minecraft_hand::SWING_TICKS * 0.5) {
                        hand.swing = Some(0.0);
                    }
                }
            }
        }
        hand.place_delay = hand.place_delay.saturating_sub(hand_ticks);
        let place = (buttons.just_pressed(MouseButton::Right) || pad_trigger(pad, GamepadButton::LeftTrigger2, true))
            || ((buttons.pressed(MouseButton::Right) || pad_trigger(pad, GamepadButton::LeftTrigger2, false)) && hand.place_delay == 0);
        // A crafting table opens its 3x3 grid on the place button.
        let bench = place
            && player
                .target(&world.scene, 4.5)
                .and_then(|hit| minecraft_terrain::scene::Scene::block(&world.scene, hit.pos))
                .is_some_and(|b| b.id.path == "crafting_table");
        if bench {
            hand.place_delay = 4;
            ui.inventory_open = true;
            ui.workbench = true;
        }
        // bo2mc (survival): a furnace or a chest opens its screen too.
        let container_hit = if *bo2mc && place && !bench {
            player
                .target(&world.scene, 4.5)
                .and_then(|hit| minecraft_terrain::scene::Scene::block(&world.scene, hit.pos).map(|b| (hit.pos, b.id.path.clone())))
                .and_then(|(pos, path)| bo2mc_world::container_kind(&path).map(|kind| (pos, kind)))
        } else {
            None
        };
        if let Some((pos, kind)) = container_hit {
            hand.place_delay = 4;
            ui.inventory_open = true;
            ui.container_kind = kind;
            bo2mc_world.open_container = Some(pos);
        }
        let place = place && !bench && container_hit.is_none();
        // bo2mc: a door item places both halves, facing where he looks.
        let door_item = entities.inventory.slots[entities.selected]
            .as_ref()
            .map(|s| s.id.clone())
            .filter(|id| *bo2mc && id.ends_with("_door"));
        if place && let Some(item) = door_item {
            hand.place_delay = 4;
            if let Some(hit) = player.target(&world.scene, 4.5) {
                let (dx, dy, dz) = hit.face.offset();
                let at = (hit.pos.0 + dx, hit.pos.1 + dy, hit.pos.2 + dz);
                let [fx, fy, fz] = feet;
                let inside = (fx - 0.3) < f64::from(at.0 + 1)
                    && (fx + 0.3) > f64::from(at.0)
                    && fy < f64::from(at.1 + 2)
                    && (fy + 1.8) > f64::from(at.1)
                    && (fz - 0.3) < f64::from(at.2 + 1)
                    && (fz + 0.3) > f64::from(at.2);
                if !inside && bo2mc_world::Bo2mcWorld::place_door(world, shapes, Some(&mut *entities), &item, at, mc_yaw) {
                    let selected = entities.selected;
                    let slot = &mut entities.inventory.slots[selected];
                    if let Some(stack) = slot.as_mut() {
                        stack.count = stack.count.saturating_sub(1);
                        if stack.count == 0 {
                            *slot = None;
                        }
                    }
                    hand.swing = Some(0.0);
                    if let Some(sounds) = sounds.as_mut() {
                        let centre = [at.0 as f64 + 0.5, at.1 as f64 + 0.5, at.2 as f64 + 0.5];
                        sounds.play(&world.packs, "minecraft:block.wood.place", Some(Vec3::from_array(sim::voxel::to_map(origin, centre))), 1.0, 0.8);
                    }
                }
            }
        } else if place
            && !(*bo2mc
                && entities.inventory.slots[entities.selected]
                    .as_ref()
                    .is_some_and(|s| minecraftoss_player::food::catalog().contains_key(&s.id)))
        {
            // (bo2mc: food is eaten, never placed.)
            hand.place_delay = 4;
            if let Some(pos) = player.place_selected(
                &mut world.scene,
                &mut entities.inventory,
                minecraftoss_player::GameMode::Survival,
            ) {
                // Not into the player's own box.
                let [fx, fy, fz] = feet;
                let inside = (fx - 0.3) < f64::from(pos.0 + 1)
                    && (fx + 0.3) > f64::from(pos.0)
                    && fy < f64::from(pos.1 + 1)
                    && (fy + 1.8) > f64::from(pos.1)
                    && (fz - 0.3) < f64::from(pos.2 + 1)
                    && (fz + 0.3) > f64::from(pos.2);
                let block = minecraft_terrain::scene::Scene::block(&world.scene, pos).cloned();
                if inside {
                    world.scene.set(pos, None);
                    if let Some(block) = block {
                        let _ = entities.inventory.add_item(
                            minecraftoss_player::inventory::ItemStack::new(block.id.key(), 1),
                            entities.selected,
                        );
                    }
                } else if let Some(block) = block {
                    let state = world.stream.states.state_of(&block);
                    let shape = state.map_or(0, |state| shapes.shape_id(&world.registries, state));
                    sim::voxel::set_block_shape(pos.0, pos.1, pos.2, shape);
                    world.stream.record_edits(&world.scene, &[pos]);
                    world.stream.mark_edited(&world.scene, &[pos]);
                    entities.placed(&world.scene, pos);
                    hand.swing = Some(0.0);
                    if let (Some(sounds), Some(kind)) = (sounds.as_mut(), world.scene.sound_type(&block)) {
                        let centre = [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5];
                        let at = Vec3::from_array(sim::voxel::to_map(origin, centre));
                        sounds.play(&world.packs, &kind.place, Some(at), (kind.volume + 1.0) / 2.0, kind.pitch * 0.8);
                    }
                }
            }
        }
    }

    // Shots and explosions from the authoritative game: bullets that met a
    // mob hurt it, the rest mine.
    let (mob_shots, events): (Vec<_>, Vec<_>) = all_events
        .into_iter()
        .partition(|event| matches!(event, sim::voxel::VoxelEvent::MobShot { .. }));
    if let Some(entities) = entities.as_mut() {
        for shot in mob_shots {
            if let sim::voxel::VoxelEvent::MobShot { key, damage, from } = shot {
                entities.shoot(key, damage, from, mc_yaw);
            }
        }
    }
    // Vanilla's block sounds: a hit for each bullet into a block, then the
    // break of each block broken (`SoundType` volume and pitch as
    // `MultiPlayerGameMode` and `LevelRenderer` scale them), and blasts.
    let at = |b: [f64; 3]| Vec3::from_array(sim::voxel::to_map(origin, b));
    if let Some(sounds) = sounds.as_mut() {
        for event in &events {
            match *event {
                sim::voxel::VoxelEvent::Shot { block, .. } => {
                    let pos = (block[0], block[1], block[2]);
                    if let Some(kind) = minecraft_terrain::scene::Scene::block(&world.scene, pos)
                        .and_then(|b| world.scene.sound_type(b))
                    {
                        let centre = [block[0] as f64 + 0.5, block[1] as f64 + 0.5, block[2] as f64 + 0.5];
                        sounds.play(&world.packs, &kind.hit, Some(at(centre)), (kind.volume + 1.0) / 4.0, kind.pitch * 0.5);
                    }
                }
                sim::voxel::VoxelEvent::Explosion { center } => {
                    let pitch = (1.0 + (sounds.random() - sounds.random()) * 0.2) * 0.7;
                    sounds.play(&world.packs, "minecraft:entity.generic.explode", Some(at(center)), 4.0, pitch);
                }
                sim::voxel::VoxelEvent::MobShot { .. } | sim::voxel::VoxelEvent::Ray { .. } => {}
            }
        }
    }
    let broken = mining.apply(
        events,
        &mut crate::minecraft_mining::WorldRefs {
            stream: &mut world.stream,
            scene: &mut world.scene,
            packs: &world.packs,
            atlas: &world.atlas,
            registries: &world.registries,
        },
        time.elapsed_secs_f64(),
    );
    if let Some(sounds) = sounds.as_mut() {
        for (pos, block, blast) in &broken {
            if *blast {
                continue;
            }
            if let Some(kind) = world.scene.sound_type(block) {
                let centre = [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5];
                sounds.play(&world.packs, &kind.break_sound, Some(at(centre)), (kind.volume + 1.0) / 2.0, kind.pitch * 0.8);
            }
        }
    }
    if let Some(entities) = entities.as_mut() {
        let positions: Vec<_> = broken.iter().map(|(pos, ..)| *pos).collect();
        entities.broke(&world.scene, &positions);
        if *bo2mc {
            // Survival: loot only with the right tool (the held one for the
            // hand's blocks; a bullet counts as a bare hand).
            let held = entities.inventory.slots[entities.selected].clone().filter(|s| crate::minecraft_inventory::weapon_of(s).is_none());
            let tools: Vec<Option<Option<minecraftoss_player::inventory::ItemStack>>> = broken
                .iter()
                .map(|(pos, block, _)| {
                    let tool = if hand_hits.contains(pos) { held.clone() } else { None };
                    let id = tool.as_ref().map_or("", |s| s.id.as_str());
                    let correct = world
                        .stream
                        .states
                        .state_of(block)
                        .is_none_or(|state| bo2mc_world::tool_speed(&world.registries, state, id).1);
                    if hand_hits.contains(pos) {
                        diag::info!(
                            World,
                            "bo2mc mined {} with {}: {}",
                            block.id.path,
                            if id.is_empty() { "the hand" } else { id },
                            if correct { "drops" } else { "no drop (wrong tool)" }
                        );
                    }
                    correct.then_some(tool)
                })
                .collect();
            entities.drop_blocks_with(&broken, &tools);
        } else {
            entities.drop_blocks(&broken);
        }
    }

    let eye = sim::voxel::to_block(origin, [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current]);
    let (pitch, yaw) = (ps.viewangles[0].to_radians(), ps.viewangles[1].to_radians());
    let map_forward = [pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), -pitch.sin()];
    let forward = glam::Vec3::new(map_forward[0], map_forward[2], -map_forward[1]);
    let aspect = windows
        .single()
        .map(|w| w.width() / w.height().max(1.0))
        .unwrap_or(16.0 / 9.0);
    // Culled from the camera that draws: the player's eye, or while
    // skating the Skate camera (a frame behind, so with room to spare).
    let skate_camera = cameras.iter().next().filter(|_| skate.active).map(|t| {
        let at = sim::voxel::to_block(origin, t.translation.to_array());
        let ahead = t.rotation * Vec3::NEG_Z;
        (
            glam::DVec3::new(at[0], at[1], at[2]),
            glam::Vec3::new(ahead.x, ahead.z, -ahead.y).normalize_or(forward),
        )
    });
    let (cull_at, cull_forward) = skate_camera.unwrap_or((glam::DVec3::new(eye[0], eye[1], eye[2]), forward));
    let camera = CullCamera {
        position: cull_at,
        forward: cull_forward,
        fov_degrees: if skate_camera.is_some() { 120.0 } else { 90.0 },
        aspect,
        yaw_degrees: (-cull_forward.x).atan2(cull_forward.z).to_degrees(),
        pitch_degrees: (-cull_forward.y).asin().to_degrees(),
    };
    let update = world
        .stream
        .frame(&world.scene, &camera, FADE_MILLIS, &world.atlas, &world.packs);
    view.uploads.extend(update.uploads);
    view.removed.extend(update.removed);
    view.visible = update.visible;
    let Some(light) = light.as_mut() else {
        return;
    };
    for (chunk, column) in update.lights {
        light.set_chunk_column(chunk, column);
    }

    if let Some(sounds) = sounds.as_mut() {
        sound_queue.0.append(&mut sounds.queued);
    }
    // The minimap's picture, and its corners on the map.
    ui.minimap = minimap
        .update(time.delta_secs_f64(), feet, &world.scene, &world.packs, &world.atlas, &mut images)
        .map(|(image, [bx, bz])| {
            let corner = |x: i32, z: i32| {
                let p = sim::voxel::to_map(origin, [f64::from(x), feet[1], f64::from(z)]);
                [p[0], p[1]]
            };
            (image, corner(bx, bz), corner(bx + 256, bz + 256))
        });
    // The Overworld clock and the environment attributes of MinecraftOSS.
    let dt = time.delta_secs_f64();
    let partial = mining.tick(&world.scene, dt);
    view.particles = mining.particle_mesh(&world.atlas, forward, partial, light);

    // The mobs: a server tick when due, the blocks it changed, its hits on
    // the player, the mobs' boxes for bullets and their meshes.
    if let Some(entities) = entities.as_mut() {
        let player = crate::minecraft_entities::PlayerView {
            feet,
            alive,
            health: ps.health as f32,
            yaw: mc_yaw,
            pitch: ps.viewangles[0],
        };
        let bright_outside = world.environment.sky_light_level() > 11.0;
        let ticks_before = entities.client_ticks();
        let (changes, hits) = entities.tick(dt, day.ticks as i64, bright_outside, &player);
        let mob_ticks = (entities.client_ticks() - ticks_before) as u32;
        if !changes.is_empty() {
            let mut positions = Vec::with_capacity(changes.len());
            for (pos, block) in changes {
                // Mobs never change the spawn room (bo2mc).
                if sim::bo2mc::is_protected([pos.0, pos.1, pos.2]) {
                    continue;
                }
                let state = block.as_ref().and_then(|b| world.stream.states.state_of(b));
                let shape = state.map_or(0, |state| shapes.shape_id(&world.registries, state));
                sim::voxel::set_block_shape(pos.0, pos.1, pos.2, shape);
                world.scene.set(pos, block);
                positions.push(pos);
            }
            world.stream.mark_edited(&world.scene, &positions);
        }
        for (amount, from) in hits {
            sim::voxel::push_player_damage(local.0.0, amount, from.map(|b| sim::voxel::to_map(origin, b)));
        }
        // The inventory: MW2 guns as items, the HUD's clicks, the hotbar's
        // gun, and what the HUD shows.
        // bo2mc: the rules side's BO2 weapons (guns, grenades with how
        // many, the knife) are the items.
        let bo2_items = if *bo2mc { sim::bo2mc::player_weapons() } else { Vec::new() };
        let bo2_index: Vec<Option<u32>> =
            bo2_items
                .iter()
                .map(|w| {
                    // The rules side sends each one's index (playtest 1b: a
                    // bought gun's name may not resolve here); else by name,
                    // which it gives without BO2's `_mp`.
                    (w.index != 0).then_some(w.index).or_else(|| {
                        authority.0.weapon_index_by_script_name(&w.name).or_else(|| authority.0.weapon_index_by_script_name(&format!("{}_mp", w.name)))
                    })
                })
                .collect();
        let bo2_item = |w: u32| bo2_items.iter().zip(&bo2_index).find(|(_, i)| **i == Some(w)).map(|(item, _)| item);
        let owned: Vec<(u32, u8)> = if *bo2mc {
            bo2_items
                .iter()
                .zip(&bo2_index)
                .filter_map(|(w, index)| {
                    let n = match w.kind {
                        "lethal" | "tactical" | "mine" => (w.clip + w.stock).clamp(0, 64),
                        _ => 1,
                    };
                    (n > 0).then_some(((*index)?, n as u8))
                })
                .collect()
        } else {
            ps.weapons
                .iter()
                .filter(|&&w| w > 0)
                .map(|&w| w as u32)
                .filter(|&w| authority.0.weapon_combat_row(w).is_some_and(|facts| facts.inventory_type == 0))
                .map(|w| (w, 1))
                .collect()
        };
        ui.active = alive;
        if !alive {
            ui.inventory_open = false;
        }
        // The table's grid empties into the inventory when the screen shuts.
        if !ui.inventory_open && ui.container_kind != 0 {
            ui.container_kind = 0;
            bo2mc_world.open_container = None;
        }
        if !ui.inventory_open && ui.workbench {
            ui.workbench = false;
            for stack in entities.inventory.settle_workbench() {
                if let Some(rest) = entities.inventory.add_item(stack, entities.selected) {
                    let _ = rest;
                }
            }
        }
        inventory_ui.sync_weapons(&mut entities.inventory, &owned);
        let mut selected = entities.selected;
        let container = match (ui.container_kind, bo2mc_world.open_container) {
            (1, Some(pos)) => crate::minecraft_inventory::OpenContainer::Furnace(bo2mc_world.furnaces.entry(pos).or_default()),
            (2, Some(pos)) => crate::minecraft_inventory::OpenContainer::Chest(bo2mc_world.chests.entry(pos).or_default()),
            _ => crate::minecraft_inventory::OpenContainer::None,
        };
        let thrown = inventory_ui.apply_input(&mut ui, &mut entities.inventory, &mut selected, container);
        ui.bo2mc = *bo2mc;
        // bo2mc (playtest 1b): a gun BO2 raised itself (a wall buy, the
        // box, Pack-a-Punch) takes the hotbar selection when it sits on the
        // hotbar; one in the main inventory goes back down for the picked
        // item. A gun he asked for in the last 3 s is his own pick.
        if *bo2mc && alive {
            let now_s = time.elapsed_secs_f64();
            bo2mc_world.asked.retain(|(_, at)| now_s - at < 3.0);
            let held = bo2_items.iter().find(|w| w.held && w.kind == "gun").map(|w| w.name.clone());
            if held.is_some() && held != bo2mc_world.held_seen {
                let name = held.clone().unwrap_or_default();
                let mine = bo2mc_world.asked.iter().any(|(n, _)| *n == name);
                let picked_now = entities.inventory.slots[selected]
                    .as_ref()
                    .and_then(crate::minecraft_inventory::weapon_of)
                    .and_then(bo2_item)
                    .map(|w| w.name.clone());
                // His own pick still on its way up (the knife put away, a
                // switch under way) is not BO2's doing either.
                let switching = picked_now.as_ref().is_some_and(|p| bo2mc_world.asked.iter().any(|(n, _)| n == p));
                if !mine && !switching && bo2mc_world.held_seen.is_some() && picked_now.as_deref() != Some(name.as_str()) {
                    let index = bo2_items.iter().zip(&bo2_index).find(|(w, _)| w.name == name).and_then(|(_, i)| *i);
                    let slot = index.and_then(|w| {
                        (0..frame::minecraft_ui::MC_HOTBAR).find(|&i| {
                            entities.inventory.slots[i].as_ref().and_then(crate::minecraft_inventory::weapon_of) == Some(w)
                        })
                    });
                    match slot {
                        Some(i) => {
                            selected = i;
                            bo2mc_world.picked = Some(Some(name.clone()));
                        }
                        None => bo2mc_world.picked = None,
                    }
                    diag::info!(World, "bo2mc hotbar: BO2 raised {name}: {}", slot.map_or("not on the hotbar, back to the pick".to_owned(), |i| format!("slot {}", i + 1)));
                }
            }
            bo2mc_world.held_seen = held;
        }
        let selected_item = entities.inventory.slots[selected]
            .as_ref()
            .and_then(crate::minecraft_inventory::weapon_of)
            .and_then(bo2_item);
        ui.held_action = match selected_item.map(|w| w.kind) {
            Some("lethal") => 1,
            Some("melee") => 2,
            Some("tactical") => 3,
            _ => 0,
        };
        // Tell the rules side what he picked: a BO2 weapon by name (a gun
        // goes in his hands, the knife makes his attack a swing), or a
        // Minecraft item.
        if *bo2mc {
            let picked = selected_item.map(|w| w.name.clone());
            if alive && bo2mc_world.picked.as_ref() != Some(&picked) {
                if let Some(name) = &picked {
                    bo2mc_world.asked.push((name.clone(), time.elapsed_secs_f64()));
                }
                sim::bo2mc::ask(match &picked {
                    Some(name) => sim::bo2mc::Ask::Select(name.clone()),
                    None => sim::bo2mc::Ask::SelectBlock,
                });
                bo2mc_world.picked = Some(picked);
            }
        }
        let thrower = crate::minecraft_inventory::Thrower {
            eye: glam::DVec3::from_array(eye),
            yaw: mc_yaw,
            pitch: ps.viewangles[0],
        };
        crate::minecraft_inventory::throw(&mut entities.world_items, thrown, &thrower);
        ui.weapon_request = if *bo2mc {
            None
        } else {
            inventory_ui.weapon_request(&entities.inventory, &mut selected, ps.weapon as u32, &|_| true)
        };
        entities.selected = selected;
        let open_slots = match (ui.container_kind, bo2mc_world.open_container) {
            (1, Some(pos)) => bo2mc_world.furnaces.get(&pos).map(|f| {
                ui.furnace_burn = if f.lit_total > 0 { f.lit_remaining as f32 / f.lit_total as f32 } else { 0.0 };
                ui.furnace_cook = if f.cook_total > 0 { f.cook_progress as f32 / f.cook_total as f32 } else { 0.0 };
                f.slots.to_vec()
            }),
            (2, Some(pos)) => bo2mc_world.chests.get(&pos).map(|c| c.slots.clone()),
            _ => None,
        }
        .unwrap_or_default();
        inventory_ui.publish(&mut ui, &entities.inventory, selected, &open_slots, &world.packs, &mut images);

        if let Some(sounds) = sounds.as_mut() {
            for (event, position, volume, pitch) in std::mem::take(&mut entities.sounds) {
                sounds.play(&world.packs, &event, Some(at(position.to_array())), volume, pitch);
            }
        }
        // The held item in view; an empty hand is MW2's own hands.
        view.hand = Default::default();
        let swing = hand.swing.map_or(0.0, |t| (t / crate::minecraft_hand::SWING_TICKS).clamp(0.0, 1.0));
        ui.hand_swing = swing;
        if ui.holding_item
            && !puppet.active
            && let Some(stack) = entities.inventory.slots[entities.selected].clone()
        {
            let eye_light_at = glam::Vec3::new(eye[0] as f32, eye[1] as f32, eye[2] as f32);
            let display = minecraft_terrain::pack::ResourceId::parse(&stack.id)
                .ok()
                .and_then(|id| minecraft_terrain::model::item_first_person_transform(&world.packs, &id).ok())
                .unwrap_or(glam::Mat4::IDENTITY);
            let pose = crate::minecraft_hand::item_pose(display, swing, 0.0);
            let mesh = entities.held_item_mesh(&stack.id, pose, eye_light_at, &world.packs, &world.atlas, light);
            let vertices: Vec<minecraft_terrain::mesh::SectionVertex> =
                mesh.vertices.iter().map(minecraft_terrain::mesh::SectionVertex::from_vertex).collect();
            view.hand = (bytemuck::cast_slice(&vertices).to_vec(), mesh.indices);
            // Reverse-Z with no far plane, as the scene's; 70 degrees up.
            let f = 1.0 / (35.0f32.to_radians()).tan();
            let near = 0.05;
            view.hand_clip = Mat4::from_cols(
                Vec4::new(f / aspect, 0.0, 0.0, 0.0),
                Vec4::new(0.0, f, 0.0, 0.0),
                Vec4::new(0.0, 0.0, 0.0, -1.0),
                Vec4::new(0.0, 0.0, near, 0.0),
            )
            .to_cols_array();
        }

        sim::voxel::set_mob_boxes(entities.boxes());
        entities.tick_scene(&world.scene, mob_ticks);
        let sky_darken = (15.0 - world.environment.sky_light_level()).clamp(0.0, 15.0) as u8;
        let meshes = entities.meshes(
            &world.scene,
            &world.packs,
            &world.atlas,
            light,
            forward,
            glam::DVec3::from_array(eye),
            sky_darken,
        );
        let raw = |mesh: &minecraft_terrain::mesh::ChunkMesh| {
            (bytemuck::cast_slice::<_, u8>(&mesh.vertices).to_vec(), mesh.indices.clone())
        };
        view.entity_meshes = [
            raw(&meshes.models),
            raw(&meshes.culled),
            raw(&meshes.translucent),
            raw(&meshes.shadows),
        ];
        let mesh = meshes.items;
        let (bytes, indices) = &mut view.particles;
        let base = (bytes.len() / std::mem::size_of::<minecraft_terrain::mesh::SectionVertex>()) as u32;
        let vertices: Vec<minecraft_terrain::mesh::SectionVertex> =
            mesh.vertices.iter().map(minecraft_terrain::mesh::SectionVertex::from_vertex).collect();
        bytes.extend_from_slice(bytemuck::cast_slice(&vertices));
        indices.extend(mesh.indices.iter().map(|i| i + base));
    }
    // Minecraft Zombies: the rules' requests, the clock, doors on the use
    // key, underground and the zombies' spawn spots.
    if *bo2mc && bo2mc_world.built {
        use playerstate_iw4::buttons::{USE, USE_RELOAD};
        let use_down = cmd_template.as_ref().is_some_and(|t| t.ready && t.cmd.buttons & (USE | USE_RELOAD) != 0);
        let right_down = (buttons.pressed(MouseButton::Right) || pad_trigger(pad, GamepadButton::LeftTrigger2, false)) && !ui.inventory_open;
        bo2mc_world.frame(&mut bo2mc_world::Frame {
            world: &mut *world,
            shapes: &mut *shapes,
            entities: entities.as_mut(),
            mining: &mut *mining,
            sounds: sounds.as_mut(),
            day: &mut *day,
            ui: &mut ui,
            origin,
            feet,
            eye,
            yaw: mc_yaw,
            pitch: ps.viewangles[0],
            alive,
            use_down,
            right_down,
            on_ground,
            health: ps.health,
            max_health: ps.max_health,
            dt,
            now: time.elapsed_secs_f64(),
        });
        // Playtest 1b: his use key is E, as is Minecraft's inventory. With
        // a BO2 prompt up (a wall buy, the box, a machine, the ammo
        // station) E buys and the inventory stays shut.
        let prompt = presented
            .snapshot()
            .is_some_and(|s| s.meta.script_dvars(local.0).string("bo2zm_hint").is_some_and(|h| !h.trim().is_empty()));
        ui.door_in_reach |= alive && prompt;
    } else {
        ui.notice = None;
        ui.door_in_reach = false;
        ui.vitals = None;
    }
    view.cracks = mining.crack_mesh();
    day.advance(dt * if *bo2mc { bo2mc_world::clock_rate(day.ticks.rem_euclid(24_000.0)) } else { 1.0 });
    let eye_block = (eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32);
    view.eye_light = [
        f32::from(light.get(eye_block)),
        f32::from(light.get_block(eye_block)),
    ];
    world
        .environment
        .update_rain_fog(0.0, light.get(eye_block), false, (dt * 20.0) as f32);
    *environment_accumulator += dt;
    if !*environment_primed || *environment_accumulator >= TICK_SECONDS {
        *environment_accumulator = (*environment_accumulator % TICK_SECONDS).min(TICK_SECONDS);
        let scene = &world.scene;
        world.environment.tick(
            day.ticks.floor() as i64,
            0.0,
            0.0,
            eye,
            |x, y, z| scene.noise_biome((x, y, z)).map_or(0, |id| id.0),
            !*environment_primed,
        );
        *environment_primed = true;
    }
    let partial_tick = (*environment_accumulator / TICK_SECONDS).clamp(0.0, 1.0) as f32;
    let sky = world.environment.sky_state(&View {
        partial_tick,
        forward,
        camera_y: eye[1] as f32,
        render_distance: VIEW_DISTANCE as u32,
        rain_level: 0.0,
        thunder_level: 0.0,
    });
    let render_distance = VIEW_DISTANCE as f32 * 16.0;
    let right = forward.cross(glam::Vec3::Y).normalize_or(glam::Vec3::X);
    let up = right.cross(forward).normalize_or(glam::Vec3::Y);
    let put = |v: glam::Vec3| [v.x, v.y, v.z, 0.0];
    let game_time = day.ticks;
    view.environment = [
        put(forward),
        put(right),
        put(up),
        [eye[0] as f32, eye[1] as f32, eye[2] as f32, 0.0],
        put(sky.sky),
        [sky.fog.x, sky.fog.y, sky.fog.z, render_distance.min(sky.sky_fog_end)],
        [
            sky.sky_light_color.x,
            sky.sky_light_color.y,
            sky.sky_light_color.z,
            sky.sky_light_factor,
        ],
        sky.sunset,
        [sky.sun_direction.x, sky.sun_direction.y, sky.sun_direction.z, sky.rain_brightness],
        [sky.moon_direction.x, sky.moon_direction.y, sky.moon_direction.z, sky.rain_brightness],
        // The brightness option at its default.
        [sky.cloud.x, sky.cloud.y, sky.cloud.z, 0.5],
        [aspect, 0.0, sky.star_brightness, sky.star_angle],
        [sky.moon_phase as f32, (game_time as f32) * 0.03, 96.0, 160.0],
        [
            sky.fog_start,
            sky.fog_end,
            render_distance - (render_distance / 10.0).clamp(4.0, 64.0),
            render_distance,
        ],
        [
            sky.ambient.x,
            sky.ambient.y,
            sky.ambient.z,
            match sky.skybox {
                Skybox::Overworld => 0.0,
                Skybox::End => 1.0,
                _ => 2.0,
            },
        ],
        [
            sky.block_light_tint.x,
            sky.block_light_tint.y,
            sky.block_light_tint.z,
            sky.block_factor,
        ],
    ];

    // Clouds, rebuilt when the camera crosses a cloud cell.
    if let Some(mask) = &world.cloud_mask {
        let center = mask.center(eye[0] as f32, eye[2] as f32, game_time);
        if *cloud_center != Some(center) {
            *cloud_center = Some(center);
            let mesh = mask.build(center, eye[1] as f32);
            view.clouds = Some(Arc::new((mesh.vertices, mesh.indices)));
        }
    }

    // The light MW2 models stand in, around the player.
    *light_volume_age += 1;
    let half = LIGHT_VOLUME / 2;
    let corner = [block.0 - half, block.1 - half, block.2 - half];
    let moved =
        light_volume_at.is_none_or(|at| (0..3).any(|k| (at[k] - corner[k]).abs() >= 4));
    if moved || *light_volume_age >= 20 {
        *light_volume_age = 0;
        *light_volume_at = Some(corner);
        let n = LIGHT_VOLUME;
        let index = |x: i32, y: i32, z: i32| (((y * n + z) * n + x) * 2) as usize;
        let mut raw = vec![0u8; (n * n * n * 2) as usize];
        for z in 0..n {
            for x in 0..n {
                let (bx, bz) = (corner[0] + x, corner[2] + z);
                // One column lookup for the whole stack, not two per cell.
                let column = light.chunk_column((bx >> 4, bz >> 4));
                for y in 0..n {
                    let pos = (bx, corner[1] + y, bz);
                    let at = index(x, y, z);
                    let (sky, block) = match column {
                        Some(column) => (column.get(pos), column.get_block(pos)),
                        None => (light.get(pos), light.get_block(pos)),
                    };
                    raw[at] = sky;
                    raw[at + 1] = block;
                }
            }
        }
        // Unlit cells (inside blocks) take their brightest neighbour, so a
        // model beside a block is not darkened by filtering into it. Levels
        // become unorm bytes.
        let mut data = vec![0u8; raw.len()];
        for y in 0..n {
            for z in 0..n {
                for x in 0..n {
                    let at = index(x, y, z);
                    let (mut sky, mut block) = (raw[at], raw[at + 1]);
                    if sky == 0 && block == 0 {
                        for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                            if (0..n).contains(&nx) && (0..n).contains(&ny) && (0..n).contains(&nz) {
                                let near = index(nx, ny, nz);
                                sky = sky.max(raw[near]);
                                block = block.max(raw[near + 1]);
                            }
                        }
                    }
                    data[at] = sky.min(15) * 17;
                    data[at + 1] = block.min(15) * 17;
                }
            }
        }
        view.light_volume = Some(Arc::new((corner, data)));
    }
}

fn stop(runtime: &mut Runtime, view: &mut MinecraftWorldView) {
    if runtime.bo2mc {
        sim::bo2mc::reset();
    }
    runtime.bo2mc = false;
    runtime.spawn_ready = false;
    runtime.bo2mc_world = Default::default();
    if runtime.world.take().is_some() || runtime.loading.take().is_some() || view.active {
        sim::voxel::deactivate();
        view.active = false;
        view.atlas = None;
        view.uploads.clear();
        view.removed.clear();
        view.visible.clear();
        view.celestial = None;
        view.crack_texture = None;
        view.particles = Default::default();
        view.entity_meshes = Default::default();
        view.hand = Default::default();
        view.cracks = Default::default();
        view.clouds = None;
        view.light_volume = None;
        view.generation += 1;
    }
}

/// The Minecraft light a Black Ops II model at map point `p` stands in, for
/// the T6 model shader: the terrain's lightmap colour (0..1, display space)
/// of the sky and block light around the model's feet and middle, from the
/// light volume around the player (the eye's light outside it). None when
/// no Minecraft world is drawn.
pub fn model_light(view: &MinecraftWorldView, p: [f32; 3]) -> Option<[f32; 3]> {
    if !view.active {
        return None;
    }
    let b = sim::voxel::to_block(view.origin, p);
    let n = LIGHT_VOLUME;
    let cell = |dy: f64| -> Option<(f32, f32)> {
        let volume = view.light_volume.as_ref()?;
        let (corner, data) = (volume.0, &volume.1);
        let x = b[0].floor() as i32 - corner[0];
        let y = (b[1] + dy).floor() as i32 - corner[1];
        let z = b[2].floor() as i32 - corner[2];
        if !(0..n).contains(&x) || !(0..n).contains(&y) || !(0..n).contains(&z) {
            return None;
        }
        let at = (((y * n + z) * n + x) * 2) as usize;
        let level = |v: u8| f32::from(v) / 17.0;
        Some((level(*data.get(at)?), level(*data.get(at + 1)?)))
    };
    // The brighter of the cell at the feet and the one above; outside the
    // volume, the light at the eye.
    let (sky, block) = match (cell(0.5), cell(1.5)) {
        (Some(a), Some(b)) => (a.0.max(b.0), a.1.max(b.1)),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => (view.eye_light[0], view.eye_light[1]),
    };
    Some(lightmap(&view.environment, sky, block))
}

/// The terrain shader's `lightmap` (MinecraftOSS's port of 26.3
/// lightmap.fsh) at the brightness option's default.
fn lightmap(environment: &[[f32; 4]; 16], sky_level: f32, block_level: f32) -> [f32; 3] {
    let brightness = |level: f32| level / (4.0 - 3.0 * level);
    let (sky_light, gamma_mix, ambient, tint) = (environment[6], environment[10][3], environment[14], environment[15]);
    let (sky, block) = (sky_level / 15.0, block_level / 15.0);
    let parabolic = (2.0 * block - 1.0) * (2.0 * block - 1.0);
    let colour: [f32; 3] = std::array::from_fn(|k| {
        let block_colour = tint[k] + (1.0 - tint[k]) * 0.9 * parabolic;
        (ambient[k] + sky_light[k] * brightness(sky) * sky_light[3] + block_colour * brightness(block) * tint[3])
            .clamp(0.0, 1.0)
    });
    let greatest = colour[0].max(colour[1]).max(colour[2]);
    let inverted = 1.0 - greatest;
    let scale = (1.0 - inverted * inverted * inverted * inverted) / greatest.max(0.00001);
    std::array::from_fn(|k| colour[k] + (colour[k] * scale - colour[k]) * gamma_mix)
}
