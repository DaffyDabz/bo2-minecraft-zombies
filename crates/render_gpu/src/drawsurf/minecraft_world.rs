//! The Minecraft map's terrain: MinecraftOSS section meshes drawn before the
//! exact material passes, with the same camera and scene depth range, so the
//! players, weapons and effects that follow sort against it. The sky, clouds,
//! lightmap and fog are MinecraftOSS's shaders driven by its environment
//! uniform, so the world has Minecraft's day and night.
use std::sync::Arc;

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, uniform_buffer_sized,
};
use bevy::render::render_resource::{
    AddressMode, BindGroup, BindGroupEntry, BindGroupLayoutDescriptor, BindingResource, BlendState,
    Buffer, BufferBinding, BufferDescriptor, BufferInitDescriptor, BufferUsages, ColorTargetState,
    ColorWrites, CompareFunction, DepthStencilState, Extent3d, FilterMode, IndexFormat,
    MipmapFilterMode, MultisampleState, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PrimitiveState, RawFragmentState, RawRenderPipelineDescriptor,
    RawVertexBufferLayout, RawVertexState, RenderPipeline, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp,
    TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, VertexAttribute, VertexFormat, VertexStepMode,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{Render, RenderApp, RenderSystems};

use super::depth_range::{GFX_DEPTH_RANGE_SCENE, reverse_z_viewport_depth};
use super::exact_pipeline::ExactPipelineRegistry;
use super::scene_depth::{SCENE_DEPTH_FORMAT, SceneDepthTexture};

/// Bytes of one MinecraftOSS `SectionVertex`: position, atlas uv, colour,
/// sky and block light (times 16), padding.
pub const MINECRAFT_VERTEX_BYTES: u64 = 28;

pub struct MinecraftSectionUpload {
    pub pos: [i32; 3],
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub transparent_start: Option<u32>,
}

pub struct MinecraftAtlasImage {
    pub width: u32,
    pub height: u32,
    /// RGBA8 levels, largest first.
    pub levels: Vec<Vec<u8>>,
}

/// Cloud geometry in blocks: position then colour per vertex.
pub struct MinecraftClouds {
    pub vertices: Vec<[f32; 7]>,
    pub indices: Vec<u32>,
}

/// One frame of the Minecraft world as the render world sees it.
#[derive(Resource, Default)]
pub struct MinecraftWorldFrame {
    pub active: bool,
    pub origin: [f64; 3],
    pub view_distance: f32,
    pub atlas: Option<Arc<MinecraftAtlasImage>>,
    pub uploads: Vec<MinecraftSectionUpload>,
    pub removed: Vec<[i32; 3]>,
    pub visible: Vec<[i32; 3]>,
    pub generation: u64,
    /// MinecraftOSS's environment uniform, in blocks.
    pub environment: [[f32; 4]; 16],
    /// Sun and moon phases side by side.
    pub celestial: Option<Arc<MinecraftAtlasImage>>,
    pub clouds: Option<Arc<MinecraftClouds>>,
    /// First block, then sky and block light as unorm pairs.
    pub light_volume: Option<Arc<([i32; 3], Vec<u8>)>>,
    pub eye_light: [f32; 2],
    /// Break particles: section vertex bytes and indices.
    pub particles: (Vec<u8>, Vec<u32>),
    /// Destroy stage cubes: position and strip uv, and indices.
    pub cracks: (Vec<[f32; 5]>, Vec<u32>),
    /// The ten destroy stages side by side.
    pub crack_texture: Option<Arc<MinecraftAtlasImage>>,
    /// Mob models (cut out, back-face culled, translucent) and entity
    /// shadows: MinecraftOSS `Vertex` bytes (44 each) and indices.
    pub entity_meshes: [(Vec<u8>, Vec<u32>); 4],
    /// A black card to draw, in blocks: behind the inventory's character.
    pub backdrop: Option<[[f32; 3]; 4]>,
    /// The first-person hand or held item in view space, and its projection.
    pub hand: (Vec<u8>, Vec<u32>),
    pub hand_clip: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainView {
    clip_from_rel: [f32; 16],
    rel_from_clip: [f32; 16],
    hand_clip: [f32; 16],
    /// View origin in map units.
    view: [f32; 4],
    /// The block at map origin.
    origin: [f32; 4],
    environment: [[f32; 4]; 16],
}

const VIEW_SIZE: u64 = std::mem::size_of::<TerrainView>() as u64;
const CLOUD_VERTEX_BYTES: u64 = 28;
const CRACK_VERTEX_BYTES: u64 = 20;
/// MinecraftOSS's full `Vertex`: position, uv, colour, sky and block light.
const ENTITY_VERTEX_BYTES: u64 = 44;

struct SectionGpu {
    vertices: Buffer,
    indices: Buffer,
    count: u32,
    transparent_start: u32,
}

#[derive(Resource, Default)]
struct TerrainGpu {
    generation: u64,
    view: Option<Buffer>,
    atlas: Option<(Arc<MinecraftAtlasImage>, TextureView)>,
    celestial: Option<(Arc<MinecraftAtlasImage>, TextureView)>,
    clouds: Option<(Arc<MinecraftClouds>, Buffer, Buffer, u32)>,
    crack_texture: Option<(Arc<MinecraftAtlasImage>, TextureView)>,
    /// This frame's particles and cracks: vertices, indices, index count.
    particles: Option<(Buffer, Buffer, u32)>,
    cracks: Option<(Buffer, Buffer, u32)>,
    /// This frame's entity meshes, in `MinecraftWorldFrame::entity_meshes` order.
    entities: [Option<(Buffer, Buffer, u32)>; 4],
    backdrop: Option<Buffer>,
    hand: Option<(Buffer, Buffer, u32)>,
    sampler: Option<Sampler>,
    bind: Option<BindGroup>,
    sections: HashMap<[i32; 3], SectionGpu>,
    visible: Vec<[i32; 3]>,
    pipelines: HashMap<(TextureFormat, u32), [RenderPipeline; 11]>,
}

pub(super) fn register(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<MinecraftWorldFrame>()
        .init_resource::<TerrainGpu>()
        .add_systems(Render, prepare_terrain.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            draw_terrain
                .in_set(Core3dSystems::MainPass)
                .before(super::draw::ExactColourDrawSet),
        );
}

fn layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "iw4l_minecraft_terrain",
        &[
            uniform_buffer_sized(false, std::num::NonZeroU64::new(VIEW_SIZE))
                .visibility(ShaderStages::VERTEX_FRAGMENT)
                .build(0, ShaderStages::VERTEX_FRAGMENT),
            texture_2d(TextureSampleType::Float { filterable: true })
                .visibility(ShaderStages::FRAGMENT)
                .build(1, ShaderStages::FRAGMENT),
            sampler(SamplerBindingType::Filtering)
                .visibility(ShaderStages::FRAGMENT)
                .build(2, ShaderStages::FRAGMENT),
            texture_2d(TextureSampleType::Float { filterable: true })
                .visibility(ShaderStages::FRAGMENT)
                .build(3, ShaderStages::FRAGMENT),
            texture_2d(TextureSampleType::Float { filterable: true })
                .visibility(ShaderStages::FRAGMENT)
                .build(4, ShaderStages::FRAGMENT),
        ],
    )
}

static HIDES_MAP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the loaded IW4 map is only standing in for a Minecraft world, so
/// its own world surfaces are not drawn.
pub(super) fn hides_map() -> bool {
    HIDES_MAP.load(std::sync::atomic::Ordering::Relaxed)
}

fn prepare_terrain(
    mut frame: ResMut<MinecraftWorldFrame>,
    published: Option<Res<super::PublishedRenderFrame>>,
    registry: Res<ExactPipelineRegistry>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<TerrainGpu>,
) {
    HIDES_MAP.store(frame.active, std::sync::atomic::Ordering::Relaxed);
    if !frame.active {
        if !gpu.sections.is_empty() || gpu.atlas.is_some() {
            *gpu = TerrainGpu {
                pipelines: std::mem::take(&mut gpu.pipelines),
                ..TerrainGpu::default()
            };
        }
        return;
    }
    if gpu.generation != frame.generation {
        gpu.sections.clear();
        gpu.generation = frame.generation;
    }
    if let Some(atlas) = frame.atlas.clone()
        && gpu.atlas.as_ref().is_none_or(|(held, _)| !Arc::ptr_eq(held, &atlas))
    {
        let view = upload_atlas(&device, &queue, &atlas);
        gpu.atlas = Some((atlas, view));
        gpu.bind = None;
    }
    if let Some(celestial) = frame.celestial.clone()
        && gpu
            .celestial
            .as_ref()
            .is_none_or(|(held, _)| !Arc::ptr_eq(held, &celestial))
    {
        let view = upload_atlas(&device, &queue, &celestial);
        gpu.celestial = Some((celestial, view));
        gpu.bind = None;
    }
    if let Some(cracks) = frame.crack_texture.clone()
        && gpu
            .crack_texture
            .as_ref()
            .is_none_or(|(held, _)| !Arc::ptr_eq(held, &cracks))
    {
        let view = upload_atlas(&device, &queue, &cracks);
        gpu.crack_texture = Some((cracks, view));
        gpu.bind = None;
    }
    let (particle_vertices, particle_indices) = std::mem::take(&mut frame.particles);
    gpu.particles = (!particle_indices.is_empty()).then(|| {
        (
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("iw4l_minecraft_particle_vertices"),
                contents: &particle_vertices,
                usage: BufferUsages::VERTEX,
            }),
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("iw4l_minecraft_particle_indices"),
                contents: bytemuck::cast_slice(&particle_indices),
                usage: BufferUsages::INDEX,
            }),
            particle_indices.len() as u32,
        )
    });
    gpu.backdrop = frame.backdrop.take().map(|[a, b, c, d]| {
        let corners: [[f32; 3]; 6] = [a, b, c, a, c, d];
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("iw4l_minecraft_backdrop"),
            contents: bytemuck::cast_slice(&corners),
            usage: BufferUsages::VERTEX,
        })
    });
    let (hand_vertices, hand_indices) = std::mem::take(&mut frame.hand);
    gpu.hand = (!hand_indices.is_empty()).then(|| {
        (
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("iw4l_minecraft_hand_vertices"),
                contents: &hand_vertices,
                usage: BufferUsages::VERTEX,
            }),
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("iw4l_minecraft_hand_indices"),
                contents: bytemuck::cast_slice(&hand_indices),
                usage: BufferUsages::INDEX,
            }),
            hand_indices.len() as u32,
        )
    });
    let meshes = std::mem::take(&mut frame.entity_meshes);
    for (slot, (vertices, indices)) in gpu.entities.iter_mut().zip(meshes) {
        *slot = (!indices.is_empty()).then(|| {
            (
                device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("iw4l_minecraft_entity_vertices"),
                    contents: &vertices,
                    usage: BufferUsages::VERTEX,
                }),
                device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("iw4l_minecraft_entity_indices"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: BufferUsages::INDEX,
                }),
                indices.len() as u32,
            )
        });
    }
    let (crack_vertices, crack_indices) = std::mem::take(&mut frame.cracks);
    gpu.cracks = (!crack_indices.is_empty()).then(|| {
        (
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("iw4l_minecraft_crack_vertices"),
                contents: bytemuck::cast_slice(&crack_vertices),
                usage: BufferUsages::VERTEX,
            }),
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("iw4l_minecraft_crack_indices"),
                contents: bytemuck::cast_slice(&crack_indices),
                usage: BufferUsages::INDEX,
            }),
            crack_indices.len() as u32,
        )
    });
    match frame.clouds.clone() {
        Some(clouds) if gpu.clouds.as_ref().is_none_or(|(held, ..)| !Arc::ptr_eq(held, &clouds)) => {
            gpu.clouds = (!clouds.indices.is_empty()).then(|| {
                let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("iw4l_minecraft_cloud_vertices"),
                    contents: bytemuck::cast_slice(&clouds.vertices),
                    usage: BufferUsages::VERTEX,
                });
                let indices = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("iw4l_minecraft_cloud_indices"),
                    contents: bytemuck::cast_slice(&clouds.indices),
                    usage: BufferUsages::INDEX,
                });
                let count = clouds.indices.len() as u32;
                (clouds, vertices, indices, count)
            });
        }
        Some(_) => {}
        None => gpu.clouds = None,
    }
    if gpu.view.is_none() {
        gpu.view = Some(device.create_buffer(&BufferDescriptor {
            label: Some("iw4l_minecraft_terrain_view"),
            size: VIEW_SIZE,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        gpu.sampler = Some(device.create_sampler(&SamplerDescriptor {
            label: Some("iw4l_minecraft_terrain"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: MipmapFilterMode::Linear,
            ..default()
        }));
    }
    if gpu.bind.is_none()
        && let (Some(view), Some((_, atlas)), Some((_, celestial)), Some((_, cracks)), Some(sampler)) = (
            gpu.view.as_ref(),
            gpu.atlas.as_ref(),
            gpu.celestial.as_ref(),
            gpu.crack_texture.as_ref(),
            gpu.sampler.as_ref(),
        )
    {
        let layout = registry.bind_group_layout(&device, &layout());
        let bind = device.create_bind_group(
            "iw4l_minecraft_terrain",
            &layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: view,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(atlas),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(sampler),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(celestial),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::TextureView(cracks),
                },
            ],
        );
        gpu.bind = Some(bind);
    }
    for pos in std::mem::take(&mut frame.removed) {
        gpu.sections.remove(&pos);
    }
    for upload in std::mem::take(&mut frame.uploads) {
        if upload.indices.is_empty() || upload.vertices.is_empty() {
            gpu.sections.remove(&upload.pos);
            continue;
        }
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("iw4l_minecraft_section_vertices"),
            contents: &upload.vertices,
            usage: BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("iw4l_minecraft_section_indices"),
            contents: bytemuck::cast_slice(&upload.indices),
            usage: BufferUsages::INDEX,
        });
        let count = upload.indices.len() as u32;
        gpu.sections.insert(
            upload.pos,
            SectionGpu {
                vertices,
                indices,
                count,
                transparent_start: upload.transparent_start.unwrap_or(count).min(count),
            },
        );
    }
    gpu.visible = frame.visible.clone();

    let mut view = TerrainView {
        environment: frame.environment,
        hand_clip: frame.hand_clip,
        ..TerrainView::default()
    };
    if let Some(exec) = published.as_ref().map(|p| &p.exec_frame)
        && let Some(clip_from_world) = exec.clip_from_world
    {
        let o = exec.view_origin;
        let clip_from_rel = clip_from_world * Mat4::from_translation(o);
        view.clip_from_rel = clip_from_rel.to_cols_array();
        view.rel_from_clip = clip_from_rel.inverse().to_cols_array();
        view.view = [o.x, o.y, o.z, 1.0];
    }
    view.origin = [
        frame.origin[0] as f32,
        frame.origin[1] as f32,
        frame.origin[2] as f32,
        frame.view_distance * 16.0,
    ];
    if let Some(buffer) = gpu.view.as_ref() {
        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&view));
    }
}

fn upload_atlas(device: &RenderDevice, queue: &RenderQueue, atlas: &MinecraftAtlasImage) -> TextureView {
    let levels = atlas.levels.len().max(1) as u32;
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("iw4l_minecraft_atlas"),
        size: Extent3d {
            width: atlas.width,
            height: atlas.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, texels) in atlas.levels.iter().enumerate() {
        let (w, h) = ((atlas.width >> level).max(1), (atlas.height >> level).max(1));
        if texels.len() < (w * h * 4) as usize {
            break;
        }
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            texels,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&TextureViewDescriptor::default())
}

fn draw_terrain(
    view: ViewQuery<(&ViewTarget, &SceneDepthTexture, &ExtractedView, Option<&Msaa>)>,
    registry: Res<ExactPipelineRegistry>,
    device: Res<RenderDevice>,
    mut gpu: ResMut<TerrainGpu>,
    mut context: RenderContext,
) {
    let Some(bind) = gpu.bind.clone() else {
        return;
    };
    let (target, depth, extracted_view, msaa) = view.into_inner();
    let format = target.main_texture_format();
    let samples = msaa.map_or(1, Msaa::samples);
    let [sky, opaque, translucent, clouds, crack, entity, entity_culled, entity_translucent, shadow, backdrop, hand] = gpu
        .pipelines
        .entry((format, samples))
        .or_insert_with(|| pipelines(&device, &registry, format, samples))
        .clone();
    let attachments = [Some(target.get_color_attachment())];
    let mut pass =
        context.begin_tracked_render_pass(bevy::render::render_resource::RenderPassDescriptor {
            label: Some("iw4l_minecraft_terrain"),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    let vp = extracted_view.viewport;
    let (depth_min, depth_max) = reverse_z_viewport_depth(GFX_DEPTH_RANGE_SCENE);
    pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, depth_min, depth_max);
    pass.set_bind_group(0, &bind, &[]);
    pass.set_render_pipeline(&opaque);
    for pos in &gpu.visible {
        let Some(section) = gpu.sections.get(pos) else {
            continue;
        };
        if section.transparent_start == 0 {
            continue;
        }
        pass.set_vertex_buffer(0, section.vertices.slice(..));
        pass.set_index_buffer(section.indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..section.transparent_start, 0, 0..1);
    }
    if let Some((vertices, indices, count)) = gpu.particles.as_ref() {
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
    }
    if let Some(card) = gpu.backdrop.as_ref() {
        let band = if render_frame::BACKDROP_IN_SCENE.load(std::sync::atomic::Ordering::Relaxed) {
            super::depth_range::GFX_DEPTH_RANGE_SCENE
        } else {
            super::depth_range::GFX_DEPTH_RANGE_VIEWMODEL
        };
        let (band_min, band_max) = reverse_z_viewport_depth(band);
        pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, band_min, band_max);
        pass.set_render_pipeline(&backdrop);
        pass.set_vertex_buffer(0, card.slice(..));
        pass.draw(0..6, 0..1);
        pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, depth_min, depth_max);
        pass.set_render_pipeline(&opaque);
    }
    // Mobs, then their shadows on what is drawn so far.
    for (pipeline, mesh) in [(&entity, &gpu.entities[0]), (&entity_culled, &gpu.entities[1]), (&shadow, &gpu.entities[3])] {
        if let Some((vertices, indices, count)) = mesh.as_ref() {
            pass.set_render_pipeline(pipeline);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..*count, 0, 0..1);
        }
    }
    // Sky wherever nothing has been drawn yet: a full-screen triangle at the
    // cleared depth.
    pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, 0.0, 0.0);
    pass.set_render_pipeline(&sky);
    pass.draw(0..3, 0..1);
    pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, depth_min, depth_max);
    pass.set_render_pipeline(&translucent);
    for pos in gpu.visible.iter().rev() {
        let Some(section) = gpu.sections.get(pos) else {
            continue;
        };
        if section.transparent_start >= section.count {
            continue;
        }
        pass.set_vertex_buffer(0, section.vertices.slice(..));
        pass.set_index_buffer(section.indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(section.transparent_start..section.count, 0, 0..1);
    }
    if let Some((vertices, indices, count)) = gpu.entities[2].as_ref() {
        pass.set_render_pipeline(&entity_translucent);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
    }
    if let Some((vertices, indices, count)) = gpu.cracks.as_ref() {
        pass.set_render_pipeline(&crack);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
    }
    // The hand in front of everything, as the view model is.
    if let Some((vertices, indices, count)) = gpu.hand.as_ref() {
        let (band_min, band_max) = reverse_z_viewport_depth(super::depth_range::GFX_DEPTH_RANGE_VIEWMODEL);
        pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, band_min, band_max);
        pass.set_render_pipeline(&hand);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
        pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, depth_min, depth_max);
    }
    if let Some((_, vertices, indices, count)) = gpu.clouds.as_ref() {
        pass.set_render_pipeline(&clouds);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
    }
}

const TERRAIN_WGSL: &str = r#"
// MinecraftOSS's environment uniform (viewer/src/sky.wgsl), in blocks.
struct Environment {
    forward: vec4<f32>, right: vec4<f32>, up: vec4<f32>, camera_pos: vec4<f32>,
    sky: vec4<f32>, fog: vec4<f32>, light: vec4<f32>, sunset: vec4<f32>,
    sun_dir: vec4<f32>, moon_dir: vec4<f32>, cloud: vec4<f32>, params: vec4<f32>,
    extra: vec4<f32>,
    fog_distances: vec4<f32>,
    ambient: vec4<f32>,
    block_tint: vec4<f32>,
}
struct TerrainView {
    clip_from_rel: mat4x4<f32>,
    rel_from_clip: mat4x4<f32>,
    hand_clip: mat4x4<f32>,
    view: vec4<f32>,
    origin: vec4<f32>,
    environment: Environment,
}
@group(0) @binding(0) var<uniform> view: TerrainView;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var celestials: texture_2d<f32>;
@group(0) @binding(4) var cracks: texture_2d<f32>;

// Map units per block, as `sim::voxel::BLOCK`.
const BLOCK: f32 = 36.0;

// Block space to map space relative to the view.
fn rel_from_block(position: vec3<f32>) -> vec3<f32> {
    let rel = position - view.origin.xyz;
    return vec3<f32>(rel.x, -rel.z, rel.y) * BLOCK - view.view.xyz;
}

fn light_brightness(level: f32) -> f32 {
    return level / (4.0 - 3.0 * level);
}

// Minecraft 26.3 lightmap.fsh as MinecraftOSS ports it: ambient, sky light
// scaled by the sky factor, and tinted block light, then the brightness
// option's notGamma blend.
fn lightmap(sky_level: f32, block_level: f32) -> vec3<f32> {
    let environment = view.environment;
    let sky = sky_level / 15.0;
    let block = block_level / 15.0;
    var color = environment.ambient.rgb;
    color += environment.light.rgb * (light_brightness(sky) * environment.light.w);
    let parabolic = (2.0 * block - 1.0) * (2.0 * block - 1.0);
    let block_color = mix(environment.block_tint.rgb, vec3<f32>(1.0), 0.9 * parabolic);
    color += block_color * (light_brightness(block) * environment.block_tint.w);
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let greatest = max(color.r, max(color.g, color.b));
    let inverted = 1.0 - greatest;
    let gamma = color * ((1.0 - inverted * inverted * inverted * inverted) / max(greatest, 0.00001));
    return mix(color, gamma, environment.cloud.w);
}

// Minecraft 26.3 fog.glsl: the larger of spherical environmental fog and
// cylindrical render-distance fog, each linear between its start and end.
fn linear_fog_value(vertex_distance: f32, fog_start: f32, fog_end: f32) -> f32 {
    if vertex_distance <= fog_start { return 0.0; }
    if vertex_distance >= fog_end { return 1.0; }
    return (vertex_distance - fog_start) / (fog_end - fog_start);
}

fn fog_value(world_pos: vec3<f32>) -> f32 {
    let environment = view.environment;
    let pos = world_pos - environment.camera_pos.xyz;
    let spherical = length(pos);
    let cylindrical = max(length(pos.xz), abs(pos.y));
    return max(
        linear_fog_value(spherical, environment.fog_distances.x, environment.fog_distances.y),
        linear_fog_value(cylindrical, environment.fog_distances.z, environment.fog_distances.w),
    );
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) world_pos: vec3<f32>,
}

@vertex
fn vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) light: vec2<f32>,
) -> Out {
    var out: Out;
    out.clip = view.clip_from_rel * vec4<f32>(rel_from_block(position), 1.0);
    out.uv = uv;
    // Unorm of level * 16 back to a level in 0..15.
    let level = light * (255.0 / 16.0);
    out.colour = vec4<f32>(colour.rgb * lightmap(level.x, level.y), colour.a);
    out.world_pos = position;
    return out;
}

// Minecraft's values are display values, as the target's are.
fn shade(in: Out, texel: vec4<f32>) -> vec4<f32> {
    let lit = texel.rgb * in.colour.rgb;
    return vec4<f32>(mix(lit, view.environment.fog.rgb, fog_value(in.world_pos)), texel.a * in.colour.a);
}

@fragment
fn opaque(in: Out) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv);
    if texel.a < 0.5 {
        discard;
    }
    return vec4<f32>(shade(in, texel).rgb, 1.0);
}

@fragment
fn translucent(in: Out) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv);
    return shade(in, texel);
}

// viewer/src/sky.wgsl, with the view ray taken from the MW2 camera.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn sky_vertex(@builtin(vertex_index) index: u32) -> SkyOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: SkyOut;
    out.ndc = uv * 2.0 - 1.0;
    out.clip = vec4<f32>(out.ndc, 0.0, 1.0);
    return out;
}

fn celestial(ray: vec3<f32>, direction: vec3<f32>, half_size: f32, slot: f32, moon: bool) -> vec4<f32> {
    let facing = dot(ray, direction);
    if facing <= 0.0 { return vec4<f32>(0.0); }
    // SkyRenderer rotates the XZ quad by Y=-90 degrees and then by the
    // celestial X angle: local +X becomes world +Z, local +Z becomes
    // (-direction.y, direction.x, 0). Its quads sit 100 units from the eye.
    let tangent_u = vec3<f32>(0.0, 0.0, 1.0);
    let tangent_v = vec3<f32>(-direction.y, direction.x, 0.0);
    let projected = ray / facing;
    var uv = vec2<f32>(dot(projected, tangent_u), dot(projected, tangent_v)) / (2.0 * half_size) + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) { return vec4<f32>(0.0); }
    // The moon phase quad reverses both texture axes in buildMoonPhases.
    if moon { uv = vec2<f32>(1.0) - uv; }
    let atlas_uv = vec2<f32>((slot + uv.x) / 9.0, uv.y);
    return textureSampleLevel(celestials, atlas_sampler, atlas_uv, 0.0);
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn stars(ray: vec3<f32>) -> f32 {
    let angle = view.environment.params.w;
    let ca = cos(angle);
    let sa = sin(angle);
    let turned = vec3<f32>(ray.x, ray.y * ca - ray.z * sa, ray.y * sa + ray.z * ca);
    let spherical = vec2<f32>(atan2(turned.z, turned.x) / 6.2831853 + 0.5, asin(clamp(turned.y, -1.0, 1.0)) / 3.14159265 + 0.5);
    let cell_uv = spherical * vec2<f32>(320.0, 160.0);
    let cell = floor(cell_uv);
    let seed = hash2(cell);
    if seed < 0.982 { return 0.0; }
    let center = vec2<f32>(hash2(cell + 9.0), hash2(cell + 31.0));
    let size = 0.045 + hash2(cell + 51.0) * 0.035;
    let offset = abs(fract(cell_uv) - center);
    return select(0.0, view.environment.params.z, all(offset < vec2<f32>(size)));
}

@fragment
fn sky_fragment(in: SkyOut) -> @location(0) vec4<f32> {
    let environment = view.environment;
    let far = view.rel_from_clip * vec4<f32>(in.ndc, 0.5, 1.0);
    let map_ray = far.xyz / far.w;
    let ray = normalize(vec3<f32>(map_ray.x, map_ray.z, -map_ray.y));
    // SkyRenderer's 16-block-high fan has a 512-block radius. Its fog value
    // interpolates between the center and rim vertex distances.
    var color = environment.fog.rgb;
    if ray.y > 0.0 {
        let radius = 16.0 * length(ray.xz) / ray.y;
        if radius < 512.0 {
            let vertex_distance = mix(16.0, length(vec2<f32>(512.0, 16.0)), radius / 512.0);
            color = mix(environment.sky.rgb, environment.fog.rgb, clamp(vertex_distance / environment.fog.w, 0.0, 1.0));
        }
    }
    if ray.y > -0.01 {
        let star = stars(ray) * smoothstep(-0.01, 0.07, ray.y);
        color = mix(color, vec3<f32>(1.0), star);
    }
    let sun_horizontal = normalize(vec3<f32>(environment.sun_dir.x, 0.0, environment.sun_dir.z + 0.0001));
    let view_horizontal = normalize(vec3<f32>(ray.x, 0.0, ray.z + 0.0001));
    let sunset = environment.sunset.a * pow(max(dot(sun_horizontal, view_horizontal), 0.0), 8.0)
        * (1.0 - smoothstep(0.0, 0.35, abs(ray.y)));
    color = mix(color, environment.sunset.rgb, sunset);
    let sun = celestial(ray, environment.sun_dir.xyz, 0.3, 0.0, false);
    // Minecraft's celestial pipeline uses OVERLAY (source alpha, destination one).
    color = min(color + sun.rgb * sun.a * environment.sun_dir.w, vec3<f32>(1.0));
    let moon = celestial(ray, environment.moon_dir.xyz, 0.2, environment.extra.x + 1.0, true);
    color = min(color + moon.rgb * moon.a * environment.moon_dir.w, vec3<f32>(1.0));
    return vec4<f32>(color, 1.0);
}

// viewer/src/clouds.wgsl.
struct CloudOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
    @location(1) distance: f32,
}

@vertex
fn cloud_vertex(@location(0) position: vec3<f32>, @location(1) colour: vec4<f32>) -> CloudOut {
    // CloudRenderer moves the texture at 0.03 blocks per game tick, with a
    // fixed 3.96-block Z phase. Geometry is rebuilt only on cell boundaries.
    let world = position - vec3<f32>(view.environment.extra.y, 0.0, 3.96);
    var out: CloudOut;
    out.clip = view.clip_from_rel * vec4<f32>(rel_from_block(world), 1.0);
    out.colour = colour;
    out.distance = distance(world, view.environment.camera_pos.xyz);
    return out;
}

@fragment
fn cloud_fragment(in: CloudOut) -> @location(0) vec4<f32> {
    let alpha = 0.8 * (1.0 - clamp(in.distance / 1024.0, 0.0, 1.0));
    return vec4<f32>(view.environment.cloud.rgb * in.colour.rgb, alpha);
}

// Mobs: shader.wgsl `vs_main` with `fs_entity` (the entity overlay rides in
// vertex alpha: negative for the hurt flash's red, else the white's alpha)
// and `fs_entity_translucent`; entity textures have no mipmaps.
struct EntityOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) world_pos: vec3<f32>,
    @location(3) light: vec3<f32>,
}

@vertex
fn entity_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) sky_light: f32,
    @location(4) block_light: f32,
) -> EntityOut {
    var out: EntityOut;
    out.clip = view.clip_from_rel * vec4<f32>(rel_from_block(position), 1.0);
    out.uv = uv;
    let light = lightmap(sky_light, block_light);
    out.colour = vec4<f32>(colour.rgb * light, colour.a);
    out.world_pos = position;
    out.light = light;
    return out;
}

@fragment
fn entity_fragment(in: EntityOut) -> @location(0) vec4<f32> {
    let texel = textureSampleLevel(atlas, atlas_sampler, in.uv, 0.0);
    if texel.a < 0.1 {
        discard;
    }
    let keep = abs(in.colour.a);
    let overlay = select(vec3<f32>(1.0), vec3<f32>(1.0, 0.0, 0.0), in.colour.a < 0.0);
    let lit = texel.rgb * in.colour.rgb * keep + overlay * (1.0 - keep) * in.light;
    return vec4<f32>(mix(lit, view.environment.fog.rgb, fog_value(in.world_pos)), 1.0);
}

@fragment
fn entity_translucent_fragment(in: EntityOut) -> @location(0) vec4<f32> {
    let texel = textureSampleLevel(atlas, atlas_sampler, in.uv, 0.0);
    if texel.a < 0.1 {
        discard;
    }
    let lit = texel.rgb * in.colour.rgb;
    return vec4<f32>(mix(lit, view.environment.fog.rgb, fog_value(in.world_pos)), texel.a * in.colour.a);
}

// The first-person hand: view-space vertices under the hand projection,
// lit by the lightmap, cut out as items are (below 0.1).
@vertex
fn hand_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) light: vec2<f32>,
) -> Out {
    var out: Out;
    out.clip = view.hand_clip * vec4<f32>(position, 1.0);
    out.uv = uv;
    let level = light * (255.0 / 16.0);
    out.colour = vec4<f32>(colour.rgb * lightmap(level.x, level.y), colour.a);
    out.world_pos = view.environment.camera_pos.xyz;
    return out;
}

@fragment
fn hand_fragment(in: Out) -> @location(0) vec4<f32> {
    let texel = textureSampleLevel(atlas, atlas_sampler, in.uv, 0.0);
    if texel.a < 0.1 {
        discard;
    }
    return vec4<f32>(texel.rgb * in.colour.rgb, 1.0);
}

// The card behind the inventory's character.
@vertex
fn backdrop_vertex(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    // The far end of its depth band, whatever its distance.
    let clip = view.clip_from_rel * vec4<f32>(rel_from_block(position), 1.0);
    return vec4<f32>(clip.xy, 0.0, clip.w);
}

@fragment
fn backdrop_fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.012, 0.011, 0.010, 1.0);
}

// shadow.wgsl: black, as dark as the shadow sprite and the vertex alpha.
@fragment
fn shadow_fragment(in: EntityOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, textureSample(atlas, atlas_sampler, in.uv).a * in.colour.a);
}

// viewer/src/block_overlay.wgsl `fs_crack`, blended as the crumbling render
// type: source times destination, twice.
struct CrackOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn crack_vertex(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> CrackOut {
    var out: CrackOut;
    out.clip = view.clip_from_rel * vec4<f32>(rel_from_block(position), 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn crack_fragment(in: CrackOut) -> @location(0) vec4<f32> {
    let colour = textureSample(cracks, atlas_sampler, in.uv);
    if colour.a < 0.1 {
        discard;
    }
    return colour;
}
"#;

fn pipelines(
    device: &RenderDevice,
    registry: &ExactPipelineRegistry,
    format: TextureFormat,
    samples: u32,
) -> [RenderPipeline; 11] {
    let shader = unsafe {
        device.create_shader_module(ShaderModuleDescriptor {
            label: Some("iw4l_minecraft_terrain"),
            source: ShaderSource::Wgsl(TERRAIN_WGSL.into()),
        })
    };
    let layout = registry.bind_group_layout(device, &layout());
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("iw4l_minecraft_terrain"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let attributes = [
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        VertexAttribute {
            format: VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
        VertexAttribute {
            format: VertexFormat::Unorm8x4,
            offset: 20,
            shader_location: 2,
        },
        VertexAttribute {
            format: VertexFormat::Unorm8x2,
            offset: 24,
            shader_location: 3,
        },
    ];
    let options = PipelineCompilationOptions {
        constants: &[],
        zero_initialize_workgroup_memory: false,
    };
    let buffers = [RawVertexBufferLayout {
        array_stride: MINECRAFT_VERTEX_BYTES,
        step_mode: VertexStepMode::Vertex,
        attributes: &attributes,
    }];
    let cloud_attributes = [
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        VertexAttribute {
            format: VertexFormat::Float32x4,
            offset: 12,
            shader_location: 1,
        },
    ];
    let cloud_buffers = [RawVertexBufferLayout {
        array_stride: CLOUD_VERTEX_BYTES,
        step_mode: VertexStepMode::Vertex,
        attributes: &cloud_attributes,
    }];
    let crack_attributes = [
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        VertexAttribute {
            format: VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
    ];
    let crack_buffers = [RawVertexBufferLayout {
        array_stride: CRACK_VERTEX_BYTES,
        step_mode: VertexStepMode::Vertex,
        attributes: &crack_attributes,
    }];
    let entity_attributes = [
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        VertexAttribute {
            format: VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
        VertexAttribute {
            format: VertexFormat::Float32x4,
            offset: 20,
            shader_location: 2,
        },
        VertexAttribute {
            format: VertexFormat::Float32,
            offset: 36,
            shader_location: 3,
        },
        VertexAttribute {
            format: VertexFormat::Float32,
            offset: 40,
            shader_location: 4,
        },
    ];
    let entity_buffers = [RawVertexBufferLayout {
        array_stride: ENTITY_VERTEX_BYTES,
        step_mode: VertexStepMode::Vertex,
        attributes: &entity_attributes,
    }];
    let backdrop_attributes = [VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    }];
    let backdrop_buffers = [RawVertexBufferLayout {
        array_stride: 12,
        step_mode: VertexStepMode::Vertex,
        attributes: &backdrop_attributes,
    }];
    let crumbling = BlendState {
        color: bevy::render::render_resource::BlendComponent {
            src_factor: bevy::render::render_resource::BlendFactor::Dst,
            dst_factor: bevy::render::render_resource::BlendFactor::Src,
            operation: bevy::render::render_resource::BlendOperation::Add,
        },
        alpha: bevy::render::render_resource::BlendComponent {
            src_factor: bevy::render::render_resource::BlendFactor::One,
            dst_factor: bevy::render::render_resource::BlendFactor::Zero,
            operation: bevy::render::render_resource::BlendOperation::Add,
        },
    };
    let make = |vertex: &str, fragment: &str, buffers: &[RawVertexBufferLayout], depth_write: bool, compare: CompareFunction, blend: Option<BlendState>, cull: Option<bevy::render::render_resource::Face>| {
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("iw4l_minecraft_terrain"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some(vertex),
                buffers,
                compilation_options: options.clone(),
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some(fragment),
                targets: &[Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: options.clone(),
            }),
            primitive: PrimitiveState {
                cull_mode: cull,
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: SCENE_DEPTH_FORMAT,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(compare),
                stencil: default(),
                bias: default(),
            }),
            multisample: MultisampleState {
                count: samples,
                ..default()
            },
            multiview_mask: None,
            cache: None,
        })
    };
    [
        make("sky_vertex", "sky_fragment", &[], false, CompareFunction::Equal, None, None),
        make("vertex", "opaque", &buffers, true, CompareFunction::GreaterEqual, None, None),
        make(
            "vertex",
            "translucent",
            &buffers,
            false,
            CompareFunction::GreaterEqual,
            Some(BlendState::ALPHA_BLENDING),
            None,
        ),
        make(
            "cloud_vertex",
            "cloud_fragment",
            &cloud_buffers,
            false,
            CompareFunction::GreaterEqual,
            Some(BlendState::ALPHA_BLENDING),
            None,
        ),
        make(
            "crack_vertex",
            "crack_fragment",
            &crack_buffers,
            false,
            CompareFunction::GreaterEqual,
            Some(crumbling),
            None,
        ),
        make("entity_vertex", "entity_fragment", &entity_buffers, true, CompareFunction::GreaterEqual, None, None),
        make(
            "entity_vertex",
            "entity_fragment",
            &entity_buffers,
            true,
            CompareFunction::GreaterEqual,
            None,
            Some(bevy::render::render_resource::Face::Back),
        ),
        make(
            "entity_vertex",
            "entity_translucent_fragment",
            &entity_buffers,
            false,
            CompareFunction::GreaterEqual,
            Some(BlendState::ALPHA_BLENDING),
            None,
        ),
        make(
            "entity_vertex",
            "shadow_fragment",
            &entity_buffers,
            false,
            CompareFunction::GreaterEqual,
            Some(BlendState::ALPHA_BLENDING),
            None,
        ),
        make("backdrop_vertex", "backdrop_fragment", &backdrop_buffers, true, CompareFunction::GreaterEqual, None, None),
        make("hand_vertex", "hand_fragment", &buffers, true, CompareFunction::GreaterEqual, None, None),
    ]
}
