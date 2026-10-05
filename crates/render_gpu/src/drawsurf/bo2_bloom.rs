//! bo2zm: Black Ops II's bloom, after the fallback pass on a Black Ops II
//! map. BO2 (`hdr_bloom_remap`, `hdr_bloom_apply`, fxc disassembly) takes
//! the scene's bright parts by a smooth threshold on luminance, blurs them
//! at a quarter of the frame and puts them back as sqrt(scene^2 + 4 bloom^2)
//! before its tone curve. The fallback pass writes display values, so the
//! threshold is on those, set by eye (the map's vision file names its
//! ranges, but not which of them the bloom reads).
//!
//! bo2zm M4: the same pass blurs the whole world while BO2's menus ask for
//! it (`Engine.BlurWorld`, `frame::WorldBlur`): the frame at a quarter size,
//! blurred twice as wide, put back in its place. The menus draw after it.

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::RenderStartup;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, uniform_buffer_sized,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer, BufferDescriptor,
    BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FilterMode,
    FragmentState, LoadOp, Operations, PipelineCache, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, StoreOp, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, VertexState,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::ViewTarget;
use bevy::shader::Shader;
use std::collections::HashMap;
use std::num::NonZeroU64;

const SHADER_PATH: &str = "embedded://render_gpu/drawsurf/bo2_bloom.wgsl";
const PARAMS_BYTES: u64 = 32;
const QUARTER_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// Display luminance where the bright pass starts, where it is full, and
/// the strength bloom goes back in with (BO2's apply uses 4 on its HDR
/// bloom; display values are smaller, so the same 4).
const THRESHOLD: [f32; 4] = [0.62, 0.95, 4.0, 0.0];

/// BO2 maps only (set by the fallback pass's geometry: the map has T6
/// lighting).
#[derive(Resource, Default)]
pub(super) struct Bo2BloomEnabled(pub bool);

/// How much BO2's menus blur the world this frame (0 = none; 2 = full).
#[derive(Resource, Default)]
struct Bo2MenuBlur(f32);

/// bo2zm: BO2's brightness, its `r_gamma` (0.5 to 1.5, 1 = as made): the
/// finished picture raised to 1/gamma. His saved brightness (-0.2 to 0.2)
/// is that slider.
#[derive(Resource)]
struct Bo2Gamma(f32);

impl Default for Bo2Gamma {
    fn default() -> Self {
        Self(1.0)
    }
}

/// bo2zm: his FXAA row (or the anti-aliasing row set above off): the
/// finished picture's edges smoothed as it goes back into the frame.
#[derive(Resource, Default)]
struct Bo2Fxaa(bool);

fn extract_menu_blur(
    (blur, settings): (
        bevy::render::Extract<Option<Res<frame::WorldBlur>>>,
        bevy::render::Extract<Option<Res<frame::GameSettings>>>,
    ),
    mut out: ResMut<Bo2MenuBlur>,
    (mut gamma, mut fxaa): (ResMut<Bo2Gamma>, ResMut<Bo2Fxaa>),
) {
    out.0 = blur.as_ref().map_or(0.0, |b| b.0);
    gamma.0 = settings.as_ref().map_or(1.0, |s| (1.0 + s.brightness * 2.5).clamp(0.5, 1.5));
    fxaa.0 = settings.as_ref().is_some_and(|s| s.fxaa || s.aa_samples > 1);
}

#[derive(Resource)]
struct Bo2Bloom {
    layout: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    sampler: Option<Sampler>,
    /// Quarter-size ping-pong targets, for the frame size they were made at.
    quarter: Option<(UVec2, [(Texture, TextureView); 2])>,
    /// extract, blur, composite: one parameter buffer each (blur twice);
    /// then the menu blur's: shrink, blur four times, put back.
    params: Option<[Buffer; 10]>,
    extract: Option<CachedRenderPipelineId>,
    blur: Option<CachedRenderPipelineId>,
    composite: HashMap<TextureFormat, CachedRenderPipelineId>,
    blurred: HashMap<TextureFormat, CachedRenderPipelineId>,
}

fn init(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(Bo2Bloom {
        layout: BindGroupLayoutDescriptor::new(
            "bo2zm_bloom",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, texture_2d(TextureSampleType::Float { filterable: true })),
                    (1, sampler(SamplerBindingType::Filtering)),
                    (2, uniform_buffer_sized(false, NonZeroU64::new(PARAMS_BYTES))),
                ),
            ),
        ),
        composite_layout: BindGroupLayoutDescriptor::new(
            "bo2zm_bloom_composite",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, texture_2d(TextureSampleType::Float { filterable: true })),
                    (1, sampler(SamplerBindingType::Filtering)),
                    (2, uniform_buffer_sized(false, NonZeroU64::new(PARAMS_BYTES))),
                    (3, texture_2d(TextureSampleType::Float { filterable: true })),
                ),
            ),
        ),
        shader: asset_server.load(SHADER_PATH),
        sampler: None,
        quarter: None,
        params: None,
        extract: None,
        blur: None,
        composite: HashMap::new(),
        blurred: HashMap::new(),
    });
}

fn pipeline(
    bloom: &Bo2Bloom,
    entry: &'static str,
    format: TextureFormat,
    composite: bool,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some(format!("bo2zm_bloom_{entry}").into()),
        layout: vec![if composite {
            bloom.composite_layout.clone()
        } else {
            bloom.layout.clone()
        }],
        immediate_size: 0,
        vertex: VertexState {
            shader: bloom.shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some("vertex".into()),
            buffers: Vec::new(),
        },
        fragment: Some(FragmentState {
            shader: bloom.shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some(entry.into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        zero_initialize_workgroup_memory: false,
    }
}

fn draw_bo2_bloom(
    view: ViewQuery<&ViewTarget>,
    enabled: Res<Bo2BloomEnabled>,
    menu_blur: Res<Bo2MenuBlur>,
    gamma: Res<Bo2Gamma>,
    fxaa: Res<Bo2Fxaa>,
    bloom: Option<ResMut<Bo2Bloom>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut context: RenderContext,
) {
    if !enabled.0 || !crate::drawsurf::geometry_diagnostic_enabled() {
        return;
    }
    let Some(mut bloom) = bloom else {
        return;
    };
    let target = view.into_inner();
    let format = target.main_texture_format();
    // Pipelines, once (the composite per target format).
    if bloom.extract.is_none() {
        let d = pipeline(&bloom, "extract", QUARTER_FORMAT, false);
        bloom.extract = Some(cache.queue_render_pipeline(d));
        let d = pipeline(&bloom, "blur", QUARTER_FORMAT, false);
        bloom.blur = Some(cache.queue_render_pipeline(d));
    }
    if !bloom.composite.contains_key(&format) {
        let d = pipeline(&bloom, "composite", format, true);
        let id = cache.queue_render_pipeline(d);
        bloom.composite.insert(format, id);
        let d = pipeline(&bloom, "blurred", format, true);
        let id = cache.queue_render_pipeline(d);
        bloom.blurred.insert(format, id);
    }
    let (Some(extract), Some(blur), Some(composite)) = (
        bloom.extract.and_then(|id| cache.get_render_pipeline(id)),
        bloom.blur.and_then(|id| cache.get_render_pipeline(id)),
        bloom.composite.get(&format).and_then(|&id| cache.get_render_pipeline(id)),
    ) else {
        return;
    };
    let size = target.main_texture().size();
    let full = UVec2::new(size.width, size.height);
    let quarter = (full / 4).max(UVec2::ONE);
    if bloom.quarter.as_ref().is_none_or(|(have, _)| *have != full) {
        let make = |label: &'static str| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: quarter.x,
                    height: quarter.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: QUARTER_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&TextureViewDescriptor::default());
            (texture, view)
        };
        bloom.quarter = Some((full, [make("bo2zm_bloom_a"), make("bo2zm_bloom_b")]));
    }
    let sampler = bloom
        .sampler
        .get_or_insert_with(|| {
            device.create_sampler(&SamplerDescriptor {
                label: Some("bo2zm_bloom_sampler"),
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..Default::default()
            })
        })
        .clone();
    let params = bloom
        .params
        .get_or_insert_with(|| {
            std::array::from_fn(|_| {
                device.create_buffer(&BufferDescriptor {
                    label: Some("bo2zm_bloom_params"),
                    size: PARAMS_BYTES,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
        })
        .clone();
    let write_with = |buffer: &Buffer, texel: Vec2, dir: Vec2, threshold: [f32; 4]| {
        let data: [f32; 8] = [
            texel.x, texel.y, dir.x, dir.y, threshold[0], threshold[1], threshold[2], threshold[3],
        ];
        queue.write_buffer(buffer, 0, bytemuck::cast_slice(&data));
    };
    let write = |buffer: &Buffer, texel: Vec2, dir: Vec2| write_with(buffer, texel, dir, THRESHOLD);
    let full_texel = Vec2::new(1.0 / full.x as f32, 1.0 / full.y as f32);
    let quarter_texel = Vec2::new(1.0 / quarter.x as f32, 1.0 / quarter.y as f32);
    write(&params[0], full_texel, Vec2::ZERO);
    write(&params[1], quarter_texel, Vec2::new(1.0, 0.0));
    write(&params[2], quarter_texel, Vec2::new(0.0, 1.0));
    // The composite's w: BO2's gamma (its brightness slider); z: FXAA on.
    // bo2mc: on a Minecraft world BO2's bloom haze is off (its colours
    // stay as bright and clear as Minecraft's); the brightness slider and
    // FXAA stay.
    let strength = if super::minecraft_world::hides_map() { 0.0 } else { THRESHOLD[2] };
    write_with(
        &params[3],
        full_texel,
        Vec2::new(if fxaa.0 { 1.0 } else { 0.0 }, 0.0),
        [THRESHOLD[0], THRESHOLD[1], strength, gamma.0],
    );
    let Some((_, [(_, a), (_, b)])) = bloom.quarter.as_ref() else {
        return;
    };
    let layout = cache.get_bind_group_layout(&bloom.layout);
    let composite_layout = cache.get_bind_group_layout(&bloom.composite_layout);
    let post = target.post_process_write();
    let encoder = context.command_encoder();
    let mut run = |label: &'static str,
                   pipe: &bevy::render::render_resource::RenderPipeline,
                   bind: &bevy::render::render_resource::BindGroup,
                   out: &TextureView| {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: out,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(LinearRgba::BLACK.into()),
                    store: StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    };
    let bind = |source: &TextureView, buffer: &Buffer| {
        device.create_bind_group(
            "bo2zm_bloom",
            &layout,
            &BindGroupEntries::with_indices(((0, source), (1, &sampler), (2, buffer.as_entire_binding()))),
        )
    };
    run("bo2zm_bloom_extract", extract, &bind(post.source, &params[0]), a);
    run("bo2zm_bloom_blur_h", blur, &bind(a, &params[1]), b);
    run("bo2zm_bloom_blur_v", blur, &bind(b, &params[2]), a);
    let composite_bind = device.create_bind_group(
        "bo2zm_bloom_composite",
        &composite_layout,
        &BindGroupEntries::with_indices((
            (0, post.source),
            (1, &sampler),
            (2, params[3].as_entire_binding()),
            (3, a),
        )),
    );
    run("bo2zm_bloom_composite", composite, &composite_bind, post.destination);
    // The menus' world blur: the whole frame (no threshold), shrunk,
    // blurred along x and y at one and then two texels, put back.
    let amount = (menu_blur.0 / 2.0).clamp(0.0, 1.0);
    let Some(blurred) = bloom.blurred.get(&format).and_then(|&id| cache.get_render_pipeline(id))
    else {
        return;
    };
    if amount <= 0.0 {
        return;
    }
    let all = [-1.0, 0.0, 0.0, amount];
    write_with(&params[4], full_texel, Vec2::ZERO, all);
    write_with(&params[5], quarter_texel, Vec2::new(1.0, 0.0), all);
    write_with(&params[6], quarter_texel, Vec2::new(0.0, 1.0), all);
    write_with(&params[7], quarter_texel, Vec2::new(2.0, 0.0), all);
    write_with(&params[8], quarter_texel, Vec2::new(0.0, 2.0), all);
    write_with(&params[9], full_texel, Vec2::ZERO, all);
    let post = target.post_process_write();
    run("bo2zm_menu_blur_shrink", extract, &bind(post.source, &params[4]), a);
    run("bo2zm_menu_blur_h1", blur, &bind(a, &params[5]), b);
    run("bo2zm_menu_blur_v1", blur, &bind(b, &params[6]), a);
    run("bo2zm_menu_blur_h2", blur, &bind(a, &params[7]), b);
    run("bo2zm_menu_blur_v2", blur, &bind(b, &params[8]), a);
    let blurred_bind = device.create_bind_group(
        "bo2zm_menu_blur",
        &composite_layout,
        &BindGroupEntries::with_indices((
            (0, post.source),
            (1, &sampler),
            (2, params[9].as_entire_binding()),
            (3, a),
        )),
    );
    run("bo2zm_menu_blur", blurred, &blurred_bind, post.destination);
}

pub(super) fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "bo2_bloom.wgsl");
    let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
        return;
    };
    render_app
        .init_resource::<Bo2BloomEnabled>()
        .init_resource::<Bo2MenuBlur>()
        .init_resource::<Bo2Gamma>()
        .init_resource::<Bo2Fxaa>()
        .add_systems(RenderStartup, init)
        .add_systems(bevy::render::ExtractSchedule, extract_menu_blur)
        .add_systems(
            Core3d,
            draw_bo2_bloom
                .in_set(Core3dSystems::PostProcess)
                .before(super::postfx::PostFxSet),
        );
}
