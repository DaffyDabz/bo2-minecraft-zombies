//! bo2mc: a Minecraft compass at the top right of the screen (his 10-08),
//! its needle always on the house's mystery box. The world side picks which
//! of Minecraft's 32 compass pictures shows; this draws it, item-sized.

use asset_core::AssetRef;
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use frame::MinecraftUi;
use frame::minecraft_ui::COMPASS_FRAMES;

use crate::draw2d::{Draw2dCmd, Draw2dList, Draw2dOp, Draw2dProvenance, tessellate_fonts};
use crate::gpu_list::{HudTessPass, TessJob};
use crate::images::{HUD_CHROME_NAMESPACE, HudImages};

#[derive(Component)]
pub(crate) struct MinecraftCompassRaster;

const MATERIAL: &str = "mc_compass";
/// The screen's right and top edges.
const RIGHT: i32 = 3;
const TOP: i32 = 1;
/// The compass's size and its gap from the corner, in virtual pixels.
const SIZE: f32 = 40.0;
const MARGIN: f32 = 8.0;

pub(crate) fn update(
    surface: Res<crate::surface::Hud2dSurface>,
    ui: Option<Res<MinecraftUi>>,
    mut pass: ResMut<HudTessPass>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut picture: Local<Option<Handle<Image>>>,
) {
    pass.minecraft_compass = TessJob::Hide;
    let Some(ui) = ui else {
        return;
    };
    let Some((frames, shown)) = ui.compass.as_ref().filter(|_| ui.active && ui.bo2mc && surface.is_ready()) else {
        return;
    };
    // The 32 pictures, stacked top to bottom, go up once.
    if picture.is_none() {
        let side = (frames.len() / 4 / COMPASS_FRAMES as usize).isqrt() as u32;
        if side == 0 {
            return;
        }
        let mut image = Image::new(
            Extent3d { width: side, height: side * COMPASS_FRAMES, depth_or_array_layers: 1 },
            TextureDimension::D2,
            frames.to_vec(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::nearest();
        *picture = Some(images.add(image));
    }
    if let Some(handle) = picture.as_ref() {
        hud_images.insert_runtime(MATERIAL, handle.clone());
    }
    let n = COMPASS_FRAMES as f32;
    let t0 = (*shown % COMPASS_FRAMES) as f32 / n;
    let r = surface.apply_rect(-(SIZE + MARGIN), MARGIN, SIZE, SIZE, RIGHT, TOP);
    let cmds = vec![Draw2dCmd {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
        s0: 0.0,
        t0,
        s1: 1.0,
        t1: t0 + 1.0 / n,
        color: [1.0; 4],
        material: MATERIAL.to_owned(),
        material_namespace: HUD_CHROME_NAMESPACE,
        op: Draw2dOp::StretchPic,
        provenance: Draw2dProvenance::CgDraw { site: "minecraft_compass" },
        layer: 1,
    }];
    let _ = hud_images.get(HUD_CHROME_NAMESPACE, AssetRef::bare_name("white"), &mut images);
    let (quads, _) = tessellate_fonts(&Draw2dList { cmds }, &Default::default());
    if !quads.is_empty() {
        pass.minecraft_compass = TessJob::Quads(quads);
    }
}
