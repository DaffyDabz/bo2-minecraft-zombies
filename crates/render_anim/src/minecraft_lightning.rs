//! bo2mc (Minecraft Zombies): Minecraft's own lightning bolts.
//!
//! His ask (10-07): the souls round's hellhounds arrive on Minecraft
//! lightning, "really to sell that you're in Minecraft". A bolt is vanilla's
//! `LightningBolt` (two ticks of life, one to three flashes, each flash a
//! new shape) drawn the way `LightningBoltRenderer` draws it: eight jagged
//! sixteen-block segments up from the strike and two side branches, with
//! the thunder and the impact sounds on its first tick and the sky flash
//! (`setSkyFlashTime`) while it lives.
use glam::{DVec3, Vec3};
use minecraft_terrain::mesh::{Atlas, ChunkMesh, Vertex};
use minecraft_terrain::pack::ResourceId;
use minecraftoss_player::rng::LegacyRandom;
use std::sync::atomic::{AtomicBool, Ordering};

/// A plain white block face for the bolt's colour (the vanilla bolt is
/// untextured).
const SPRITES: [&str; 3] = ["minecraft:block/white_concrete", "minecraft:block/snow", "minecraft:block/quartz_block_top"];
/// Vanilla's bolt is (0.45, 0.45, 0.5) drawn four times over additively,
/// a white-lavender glow; one solid pass in that glow's colour.
const COLOR: [f32; 4] = [0.92, 0.92, 1.0, 1.0];
/// The widest of vanilla's four passes is 0.7 blocks; the solid pass sits
/// between its core and its glow.
const WIDTH: f32 = 0.3;

/// The sky lights up while a bolt is in its flash (read by the sky).
static SKY_FLASH: AtomicBool = AtomicBool::new(false);

pub(crate) fn sky_flash() -> bool {
    SKY_FLASH.load(Ordering::Relaxed)
}

struct Bolt {
    /// The strike, in blocks.
    pos: DVec3,
    /// Seconds before it strikes.
    delay: f64,
    seed: u64,
    life: i32,
    flashes: i32,
    thundered: bool,
}

#[derive(Default)]
pub(crate) struct Bolts {
    bolts: Vec<Bolt>,
}

impl Bolts {
    /// A bolt at `pos` (blocks) in `delay` seconds.
    pub(crate) fn strike(&mut self, pos: [f64; 3], delay: f64, random: &mut LegacyRandom) {
        let flashes = random.next_int(3) as i32 + 1;
        self.bolts.push(Bolt { pos: DVec3::from_array(pos), delay, seed: random.next_long(), life: 2, flashes, thundered: false });
    }

    /// Counts the delays down, every frame.
    pub(crate) fn frame(&mut self, dt: f64) {
        for bolt in &mut self.bolts {
            bolt.delay -= dt;
        }
    }

    /// `LightningBolt.tick`, twenty times a second. The sounds go out as
    /// (event, block point, volume, pitch): vanilla's thunder at volume
    /// 10000 is heard everywhere.
    pub(crate) fn tick(&mut self, random: &mut LegacyRandom, sounds: &mut Vec<(String, DVec3, f32, f32)>) {
        let mut flash = false;
        self.bolts.retain_mut(|bolt| {
            if bolt.delay > 0.0 {
                return true;
            }
            if !bolt.thundered {
                bolt.thundered = true;
                let thunder = 0.8 + random.next_float() * 0.2;
                let impact = 0.5 + random.next_float() * 0.2;
                sounds.push(("minecraft:entity.lightning_bolt.thunder".to_owned(), bolt.pos, 10_000.0, thunder));
                sounds.push(("minecraft:entity.lightning_bolt.impact".to_owned(), bolt.pos, 2.0, impact));
                diag::info!(World, "bo2mc: lightning strikes at ({:.0}, {:.0}, {:.0})", bolt.pos.x, bolt.pos.y, bolt.pos.z);
            }
            bolt.life -= 1;
            if bolt.life < 0 {
                if bolt.flashes == 0 {
                    return false;
                }
                if bolt.life < -(random.next_int(10) as i32) {
                    bolt.flashes -= 1;
                    bolt.life = 1;
                    bolt.seed = random.next_long();
                }
            }
            if bolt.life >= 0 {
                flash = true;
            }
            true
        });
        SKY_FLASH.store(flash, Ordering::Relaxed);
    }

    /// `LightningBoltRenderer`: full-bright boxes along each segment.
    pub(crate) fn append_mesh(&self, mesh: &mut ChunkMesh, atlas: &Atlas) {
        if self.bolts.iter().all(|b| b.delay > 0.0) {
            return;
        }
        let Some(sprite) = SPRITES.iter().filter_map(|s| ResourceId::parse(s).ok()).find(|s| atlas.contains(s)) else {
            return;
        };
        let [u0, v0, u1, v1] = atlas.region(&sprite);
        let uv = [(u0 + u1) * 0.5, (v0 + v1) * 0.5];
        for bolt in self.bolts.iter().filter(|b| b.delay <= 0.0) {
            let base = bolt.pos.as_vec3();
            for (bottom, top, y, w_bottom, w_top) in segments(bolt.seed) {
                let b = base + Vec3::new(bottom[0], y, bottom[1]);
                let t = base + Vec3::new(top[0], y + 16.0, top[1]);
                append_box(mesh, b, t, w_bottom, w_top, uv);
            }
        }
    }
}

/// Vanilla's segments for one seed: (bottom xz, top xz, bottom y, bottom
/// width, top width), the main bolt (seg 7 down to 0, its bottom at the
/// strike) and two branches off it.
fn segments(seed: u64) -> Vec<([f32; 2], [f32; 2], f32, f32, f32)> {
    let mut out = Vec::new();
    let mut xs = [0.0f32; 8];
    let mut zs = [0.0f32; 8];
    let (mut x, mut z) = (0.0f32, 0.0f32);
    let mut random = LegacyRandom::new(seed);
    for i in (0..8).rev() {
        xs[i] = x;
        zs[i] = z;
        x += random.next_int(11) as f32 - 5.0;
        z += random.next_int(11) as f32 - 5.0;
    }
    let mut random = LegacyRandom::new(seed);
    for branch in 0..3i32 {
        let top = if branch > 0 { 7 - branch } else { 7 };
        let bottom = if branch > 0 { top - 2 } else { 0 };
        let mut bx = xs[top as usize] - x;
        let mut bz = zs[top as usize] - z;
        for seg in (bottom..=top).rev() {
            let (px, pz) = (bx, bz);
            if branch == 0 {
                bx += random.next_int(11) as f32 - 5.0;
                bz += random.next_int(11) as f32 - 5.0;
            } else {
                bx += random.next_int(31) as f32 - 15.0;
                bz += random.next_int(31) as f32 - 15.0;
            }
            let (mut w_bottom, mut w_top) = (WIDTH, WIDTH);
            if branch == 0 {
                w_bottom *= seg as f32 * 0.1 + 1.0;
                w_top *= (seg - 1) as f32 * 0.1 + 1.0;
            }
            out.push(([bx, bz], [px, pz], seg as f32 * 16.0, w_bottom, w_top.max(WIDTH * 0.5)));
        }
    }
    out
}

/// Four sides from a bottom square to a top square, both windings (the
/// pass may cull back faces).
fn append_box(mesh: &mut ChunkMesh, bottom: Vec3, top: Vec3, w_bottom: f32, w_top: f32, uv: [f32; 2]) {
    const CORNERS: [(f32, f32); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    for i in 0..4 {
        let (a, b) = (CORNERS[i], CORNERS[(i + 1) % 4]);
        let points = [
            bottom + Vec3::new(a.0 * w_bottom, 0.0, a.1 * w_bottom),
            bottom + Vec3::new(b.0 * w_bottom, 0.0, b.1 * w_bottom),
            top + Vec3::new(b.0 * w_top, 0.0, b.1 * w_top),
            top + Vec3::new(a.0 * w_top, 0.0, a.1 * w_top),
        ];
        let start = mesh.vertices.len() as u32;
        for p in points {
            mesh.vertices.push(Vertex { position: p.to_array(), uv, color: COLOR, sky_light: 15.0, block_light: 15.0 });
        }
        mesh.indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
        mesh.indices.extend_from_slice(&[start, start + 2, start + 1, start, start + 3, start + 2]);
        mesh.faces += 2;
    }
}
