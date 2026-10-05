//! Minecraft lighting on MW2 draws. While a Minecraft world stands in for the
//! map, every exact material pass finds the block each pixel is in, reads the
//! sky and block light there from a volume around the player, and applies
//! the world's lightmap and fog to its colour, so players, weapons and the
//! view model are lit by the day and night of the world around them.
use std::num::NonZeroU64;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{sampler, texture_3d, uniform_buffer_sized};
use bevy::render::render_resource::{
    AddressMode, BindGroup, BindGroupEntry, BindGroupLayoutDescriptor, BindingResource, Buffer,
    BufferBinding, BufferDescriptor, BufferUsages, Extent3d, FilterMode, Origin3d, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, TexelCopyBufferLayout,
    TexelCopyTextureInfo, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureViewDescriptor,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderSystems};

use super::exact_pipeline::ExactPipelineRegistry;
use super::minecraft_world::MinecraftWorldFrame;

pub(super) const MINECRAFT_GROUP: usize = 2;
/// Blocks on a side of the light volume, as `render_anim`'s `LIGHT_VOLUME`.
pub const MINECRAFT_LIGHT_VOLUME: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct McLight {
    world_rel_from_clip: [f32; 16],
    /// The same for view model draws, which have their own projection.
    viewmodel_rel_from_clip: [f32; 16],
    /// View origin in map units; `w` is 1 while the lighting applies.
    view_origin: [f32; 4],
    /// The block at map origin.
    block_origin: [f32; 4],
    /// The light volume's first block; `w` is its size.
    volume: [f32; 4],
    /// MinecraftOSS's environment uniform.
    environment: [[f32; 4]; 16],
}

const LIGHT_SIZE: u64 = std::mem::size_of::<McLight>() as u64;

pub(super) fn layout_descriptor() -> BindGroupLayoutDescriptor {
    let both = ShaderStages::VERTEX_FRAGMENT;
    let fragment = ShaderStages::FRAGMENT;
    BindGroupLayoutDescriptor::new(
        "iw4l_minecraft_light",
        &[
            uniform_buffer_sized(false, NonZeroU64::new(LIGHT_SIZE))
                .visibility(both)
                .build(0, both),
            texture_3d(TextureSampleType::Float { filterable: true })
                .visibility(fragment)
                .build(1, fragment),
            sampler(SamplerBindingType::Filtering)
                .visibility(fragment)
                .build(2, fragment),
        ],
    )
}

/// The camera passes bind `camera`; shadow passes bind `off`, which leaves
/// colour untouched.
#[derive(Resource, Default)]
pub struct MinecraftLightGpu {
    resources: Option<Resources>,
}

struct Resources {
    camera_view: Buffer,
    volume: bevy::render::render_resource::Texture,
    volume_held: Option<Arc<([i32; 3], Vec<u8>)>>,
    volume_corner: [i32; 3],
    camera: BindGroup,
    off: BindGroup,
}

impl MinecraftLightGpu {
    pub(super) fn camera(&self) -> Option<&BindGroup> {
        self.resources.as_ref().map(|r| &r.camera)
    }

    pub(super) fn off(&self) -> Option<&BindGroup> {
        self.resources.as_ref().map(|r| &r.off)
    }
}

pub(super) fn register(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<MinecraftLightGpu>()
        .add_systems(Render, prepare_light.in_set(RenderSystems::PrepareResources));
}

fn view_buffer(device: &RenderDevice, label: &'static str) -> Buffer {
    device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size: LIGHT_SIZE,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create(device: &RenderDevice, queue: &RenderQueue, registry: &ExactPipelineRegistry) -> Resources {
    let n = MINECRAFT_LIGHT_VOLUME;
    let volume = device.create_texture(&TextureDescriptor {
        label: Some("iw4l_minecraft_light_volume"),
        size: Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: n,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D3,
        format: TextureFormat::Rg8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let volume_view = volume.create_view(&TextureViewDescriptor::default());
    let sampler: Sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("iw4l_minecraft_light_volume"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let camera_view = view_buffer(device, "iw4l_minecraft_light_camera");
    let off_view = view_buffer(device, "iw4l_minecraft_light_off");
    queue.write_buffer(&off_view, 0, bytemuck::bytes_of(&McLight::default()));
    queue.write_buffer(&camera_view, 0, bytemuck::bytes_of(&McLight::default()));
    let layout = registry.bind_group_layout(device, &layout_descriptor());
    let bind = |view: &Buffer| {
        device.create_bind_group(
            "iw4l_minecraft_light",
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
                    resource: BindingResource::TextureView(&volume_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&sampler),
                },
            ],
        )
    };
    let camera = bind(&camera_view);
    let off = bind(&off_view);
    Resources {
        camera_view,
        volume,
        volume_held: None,
        volume_corner: [0; 3],
        camera,
        off,
    }
}

fn prepare_light(
    world: Res<MinecraftWorldFrame>,
    frame: Option<Res<super::PublishedRenderFrame>>,
    registry: Res<ExactPipelineRegistry>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<MinecraftLightGpu>,
) {
    let resources = gpu
        .resources
        .get_or_insert_with(|| create(&device, &queue, &registry));
    let mut light = McLight::default();
    let exec = frame.as_ref().map(|frame| &frame.exec_frame);
    if world.active
        && let Some(exec) = exec
        && let Some(clip_from_world) = exec.clip_from_world
    {
        if let Some(volume) = world.light_volume.clone()
            && resources
                .volume_held
                .as_ref()
                .is_none_or(|held| !Arc::ptr_eq(held, &volume))
        {
            let n = MINECRAFT_LIGHT_VOLUME;
            if volume.1.len() >= (n * n * n * 2) as usize {
                queue.write_texture(
                    TexelCopyTextureInfo {
                        texture: &resources.volume,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect: TextureAspect::All,
                    },
                    &volume.1,
                    TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(n * 2),
                        rows_per_image: Some(n),
                    },
                    Extent3d {
                        width: n,
                        height: n,
                        depth_or_array_layers: n,
                    },
                );
                resources.volume_corner = volume.0;
            }
            resources.volume_held = Some(volume);
        }
        let origin = exec.view_origin;
        let clip_from_rel = clip_from_world * Mat4::from_translation(origin);
        light.world_rel_from_clip = clip_from_rel.inverse().to_cols_array();
        let viewmodel_clip_from_world = exec.viewmodel_clip_from_world.unwrap_or(clip_from_world);
        light.viewmodel_rel_from_clip = (viewmodel_clip_from_world * Mat4::from_translation(origin))
            .inverse()
            .to_cols_array();
        // Without a volume yet, the pixel light is the eye's.
        let lit = resources.volume_held.is_some();
        light.view_origin = [origin.x, origin.y, origin.z, if lit { 1.0 } else { 2.0 }];
        light.block_origin = [
            world.origin[0] as f32,
            world.origin[1] as f32,
            world.origin[2] as f32,
            0.0,
        ];
        let c = resources.volume_corner;
        light.volume = [
            c[0] as f32,
            c[1] as f32,
            c[2] as f32,
            MINECRAFT_LIGHT_VOLUME as f32,
        ];
        light.environment = world.environment;
        // The eye's light, for passes before the volume arrives.
        light.environment[3][3] = world.eye_light[0];
        light.block_origin[3] = world.eye_light[1];
    } else if !world.active {
        resources.volume_held = None;
    }
    queue.write_buffer(&resources.camera_view, 0, bytemuck::bytes_of(&light));
}

const HOOK_WGSL: &str = r#"
struct McLight {
    world_rel_from_clip: mat4x4<f32>,
    viewmodel_rel_from_clip: mat4x4<f32>,
    view_origin: vec4<f32>,
    block_origin: vec4<f32>,
    volume: vec4<f32>,
    environment: array<vec4<f32>, 16>,
}
@group(2) @binding(0) var<uniform> mc_light: McLight;
@group(2) @binding(1) var mc_volume: texture_3d<f32>;
@group(2) @binding(2) var mc_volume_sampler: sampler;

fn mc_light_brightness(level: f32) -> f32 {
    return level / (4.0 - 3.0 * level);
}

// MinecraftOSS's port of 26.3 lightmap.fsh.
fn mc_lightmap(sky_level: f32, block_level: f32) -> vec3<f32> {
    let ambient = mc_light.environment[14];
    let sky_light = mc_light.environment[6];
    let block_tint = mc_light.environment[15];
    let sky = sky_level / 15.0;
    let block = block_level / 15.0;
    var color = ambient.rgb;
    color += sky_light.rgb * (mc_light_brightness(sky) * sky_light.w);
    let parabolic = (2.0 * block - 1.0) * (2.0 * block - 1.0);
    let block_color = mix(block_tint.rgb, vec3<f32>(1.0), 0.9 * parabolic);
    color += block_color * (mc_light_brightness(block) * block_tint.w);
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let greatest = max(color.r, max(color.g, color.b));
    let inverted = 1.0 - greatest;
    let gamma = color * ((1.0 - inverted * inverted * inverted * inverted) / max(greatest, 0.00001));
    // Models take the Moody brightness, which leaves daylight alone and keeps
    // dark places dark; the terrain keeps the default brightness.
    return mix(color, gamma, 0.0);
}

fn mc_linear_fog(distance: f32, start: f32, end: f32) -> f32 {
    if distance <= start { return 0.0; }
    if distance >= end { return 1.0; }
    return (distance - start) / (end - start);
}

// 26.3 fog.glsl: spherical environment fog or cylindrical render fog.
fn mc_fog(pos: vec3<f32>) -> f32 {
    let distances = mc_light.environment[13];
    let spherical = length(pos);
    let cylindrical = max(length(pos.xz), abs(pos.y));
    return max(
        mc_linear_fog(spherical, distances.x, distances.y),
        mc_linear_fog(cylindrical, distances.z, distances.w),
    );
}

struct McPixel {
    p: vec3<f32>,
    n: vec3<f32>,
}

// View model pixels sit in the nearest band of window depth.
const MC_VIEWMODEL_DEPTH: f32 = 1.0 - 0.015625;

// Taken on entry, where derivatives are in uniform control flow: the pixel in
// map space and its surface normal, facing the eye. `clip` is the vertex
// clip position and `depth` the window depth, which tells view model pixels,
// projected their own way, from the rest.
fn mc_enter(clip: vec4<f32>, depth: f32) -> McPixel {
    var h = mc_light.world_rel_from_clip * clip;
    if depth > MC_VIEWMODEL_DEPTH {
        h = mc_light.viewmodel_rel_from_clip * clip;
    }
    let rel = h.xyz / h.w;
    var n = cross(dpdx(rel), dpdy(rel));
    if dot(n, rel) > 0.0 {
        n = -n;
    }
    return McPixel(mc_light.view_origin.xyz + rel, n);
}

// Minecraft's entity lighting (entity.vsh minecraft_mix_light): ambient 0.4
// and two fixed lights of 0.6, in block axes.
fn mc_entity_shade(n_map: vec3<f32>) -> f32 {
    let len = length(n_map);
    if len < 1e-12 {
        return 1.0;
    }
    let n = vec3<f32>(n_map.x, n_map.z, -n_map.y) / len;
    let light0 = normalize(vec3<f32>(0.2, 1.0, -0.7));
    let light1 = normalize(vec3<f32>(-0.2, 1.0, 0.7));
    return min(1.0, 0.4 + 0.6 * (max(dot(n, light0), 0.0) + max(dot(n, light1), 0.0)));
}

fn mc_exit(px: McPixel, colour: vec4<f32>) -> vec4<f32> {
    let p = px.p;
    let mode = mc_light.view_origin.w;
    if mode == 0.0 {
        return colour;
    }
    // Map units to blocks (36 a block, as `sim::voxel::BLOCK`): X east, Z up
    // to Y up, Y north to -Z.
    let block = mc_light.block_origin.xyz + vec3<f32>(p.x, p.z, -p.y) / 36.0;
    var level = vec2<f32>(mc_light.environment[3].w, mc_light.block_origin.w);
    if mode == 1.0 {
        let rel = (block - mc_light.volume.xyz) / mc_light.volume.w;
        level = textureSampleLevel(mc_volume, mc_volume_sampler, vec3<f32>(rel.x, rel.z, rel.y), 0.0).rg * 15.0;
    }
    let lit = colour.rgb * mc_lightmap(level.x, level.y) * mc_entity_shade(px.n);
    let fogged = mix(lit, mc_light.environment[5].rgb, mc_fog(block - mc_light.environment[3].xyz));
    return vec4<f32>(fogged, colour.a);
}
"#;

/// The exact pass shader with the Minecraft lighting added, or `None` when
/// its shape is not the generator's or the result does not validate.
fn inject(source: &str) -> Option<String> {
    const VARYING_LIMIT: usize = 16;
    let struct_start = source.find("struct Sm3Varyings {")?;
    let base_line = source[struct_start..].find("sm3_constant_base: u32,")? + struct_start;
    let line_start = source[..base_line].rfind('\n')? + 1;
    let location_at = source[line_start..base_line].find("@location(")? + line_start + 10;
    let location_end = source[location_at..].find(')')? + location_at;
    let base_location: usize = source[location_at..location_end].parse().ok()?;
    let world_location = base_location + 1;
    if world_location >= VARYING_LIMIT {
        return None;
    }
    let line_end = source[base_line..].find('\n')? + base_line + 1;

    let ret = source.find("    return Sm3Varyings(\n")?;
    let pos_start = ret + "    return Sm3Varyings(\n".len();
    let pos_end = source[pos_start..].find(",\n")? + pos_start;
    let position = source[pos_start..pos_end].trim();
    let vertex_close = source[pos_start..].find("        sm3_constant_base,\n    );")? + pos_start;

    let mut out = String::with_capacity(source.len() + HOOK_WGSL.len() + 1024);
    out.push_str(HOOK_WGSL);
    out.push_str(&source[..line_end]);
    out.push_str(&format!("    @location({world_location}) mc_world: vec4<f32>,\n"));
    out.push_str(&source[line_end..vertex_close]);
    out.push_str(&format!(
        "        sm3_constant_base,\n        {position},\n    );"
    ));
    let rest = &source[vertex_close + "        sm3_constant_base,\n    );".len()..];

    // Each fragment entry: find the pixel on entry, light it on the way out.
    const SIGNATURE_END: &str = ") -> @location(0) vec4<f32> {\n";
    let mut tail = String::with_capacity(rest.len() + 1024);
    let mut cursor = 0;
    while let Some(found) = rest[cursor..].find(SIGNATURE_END) {
        let open = cursor + found + SIGNATURE_END.len();
        tail.push_str(&rest[cursor..open]);
        tail.push_str("    let mc_px = mc_enter(varyings.mc_world, varyings.position.z);\n");
        let body_end = rest[open..].find("\n}\n").map(|i| open + i)?;
        let body = &rest[open..body_end];
        let ret_at = body.rfind("    return ")?;
        let value = body[ret_at + "    return ".len()..].trim_end().strip_suffix(';')?;
        tail.push_str(&body[..ret_at]);
        tail.push_str(&format!("    return mc_exit(mc_px, {value});"));
        cursor = body_end;
    }
    tail.push_str(&rest[cursor..]);
    out.push_str(&tail);

    match validate(&out) {
        Ok(()) => {
            INJECTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(out)
        }
        Err(error) => {
            static LOGGED: std::sync::Once = std::sync::Once::new();
            LOGGED.call_once(|| {
                diag::warn!(World, "minecraft light: injected pass shader rejected: {error}");
                let _ = std::fs::create_dir_all("iw4l-artifacts");
                let _ = std::fs::write("iw4l-artifacts/minecraft_rejected.wgsl", &out);
            });
            None
        }
    }
}

static INJECTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SKIPPED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `source` with the Minecraft lighting, or unchanged when it cannot be
/// added.
pub(super) fn hooked(source: &str) -> std::borrow::Cow<'_, str> {
    use std::sync::atomic::Ordering::Relaxed;
    match inject(source) {
        Some(out) => std::borrow::Cow::Owned(out),
        None => {
            let skipped = SKIPPED.fetch_add(1, Relaxed) + 1;
            if skipped.is_power_of_two() {
                diag::info!(
                    World,
                    "minecraft light: {skipped} pass shaders without it, {} with",
                    INJECTED.load(Relaxed)
                );
            }
            std::borrow::Cow::Borrowed(source)
        }
    }
}

fn validate(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| e.emit_to_string(source))
}
