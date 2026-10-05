//! bo2zm: Black Ops II materials into the engine's material catalog.
//!
//! BO2 materials carry D3D11 techniques the engine cannot run, so a row here
//! is only what the renderer's fallback draw samples: the material's colour
//! map, read from the game's image packs (IWI v27) into a GPU image in its
//! own block format (BC1-3). Rows have no technique set; their surfaces keep
//! drawing through the fallback pass, which samples the colour map.

use std::collections::HashMap;
use std::sync::Arc;

use asset_core::{AssetEdge, AssetNamespace, AssetRef, T6Draw, ZoneOwner};
use asset_material::{
    AuthoredImage, AuthoredMaterial, MaterialCatalog, MaterialTextureBinding, TS_COLOR_MAP,
    TS_NORMAL_MAP, TS_SPECULAR_MAP,
};
use asset_t6::{ImageRef, ImageSource, PackSet, ZoneCapture};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::Image;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

/// `R_HashString("colorMap")`, the colour map's slot in a texture table.
const COLOR_MAP_HASH: u32 = 0xa0ab_1041;

/// `R_HashString`: case-folded, xor with 33 times the running hash.
const fn name_hash(name: &str) -> u32 {
    let b = name.as_bytes();
    let mut h = 0u32;
    let mut i = 0;
    while i < b.len() {
        h = (b[i] | 0x20) as u32 ^ h.wrapping_mul(33);
        i += 1;
    }
    h
}

/// A layered material's extra colour maps (`colorMap1`, `colorMap2`) ride
/// in the catalog under these private semantics; the fallback draw reads
/// them by semantic. Real T6 semantics stay below 0x20.
pub const T6_LAYER1_SEMANTIC: u8 = 0xf1;
pub const T6_LAYER2_SEMANTIC: u8 = 0xf2;
const LAYER_MAPS: [(u32, u8); 2] = [
    (name_hash("colorMap1"), T6_LAYER1_SEMANTIC),
    (name_hash("colorMap2"), T6_LAYER2_SEMANTIC),
];

/// What linking did: capture material index -> catalog material index.
pub(crate) struct T6Materials {
    pub local: HashMap<usize, usize>,
    pub report: Vec<String>,
}

/// A GPU image over an IWI's own blocks: every mip level, largest first.
/// A cube map (the sky's) is six faces, each with its mips, read linear:
/// the sky shader decodes it as rgb / a in linear space.
fn gpu_image(iwi: &ipak_t6::IwiImage<'_>) -> Result<Image, String> {
    use ipak_t6::IwiFormat as F;
    let cube = iwi.is_cube();
    let format = match (iwi.format, cube) {
        (F::Dxt1, false) => TextureFormat::Bc1RgbaUnormSrgb,
        (F::Dxt3, false) => TextureFormat::Bc2RgbaUnormSrgb,
        (F::Dxt5, false) => TextureFormat::Bc3RgbaUnormSrgb,
        // Normal maps: two channels, read as stored.
        (F::Dxn, false) => TextureFormat::Bc5RgUnorm,
        (F::Dxt1, true) => TextureFormat::Bc1RgbaUnorm,
        (F::Dxt3, true) => TextureFormat::Bc2RgbaUnorm,
        (F::Dxt5, true) => TextureFormat::Bc3RgbaUnorm,
        // Uncompressed: widened to RGBA8 below.
        (F::Rgba8 | F::Rgb8 | F::LuminanceAlpha | F::Luminance | F::Alpha, false) => {
            TextureFormat::Rgba8UnormSrgb
        }
        (other, _) => return Err(format!("{other:?} colour maps are not drawn yet")),
    };
    let widen = |level: &[u8]| -> Vec<u8> {
        match iwi.format {
            F::Rgba8 => level.to_vec(),
            F::Rgb8 => level
                .chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            F::LuminanceAlpha => level
                .chunks_exact(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            F::Luminance => level.iter().flat_map(|&l| [l, l, l, 255]).collect(),
            F::Alpha => level.iter().flat_map(|&a| [255, 255, 255, a]).collect(),
            _ => level.to_vec(),
        }
    };
    if iwi.is_volume() {
        return Err("a volume image".to_owned());
    }
    if iwi.format.is_block_compressed() && (iwi.width % 4 != 0 || iwi.height % 4 != 0) {
        return Err(format!(
            "{}x{} is not whole 4x4 blocks",
            iwi.width, iwi.height
        ));
    }
    // The fast quality setting starts at the second level (half size)
    // where that is still whole blocks; the sky keeps every level.
    let skip = u32::from(
        asset_core::t6_fast() && !cube && iwi.levels > 1 && iwi.width >= 128 && iwi.height >= 128,
    );
    // IWI keeps each level's faces together; the GPU image wants each face
    // with all its levels (layer-major).
    let faces = iwi.faces as usize;
    let mut data = Vec::new();
    for face in 0..faces {
        for level in skip..iwi.levels {
            let all = iwi.level(level);
            let one = all.len() / faces;
            data.extend_from_slice(&widen(&all[face * one..(face + 1) * one]));
        }
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: iwi.width >> skip,
            height: iwi.height >> skip,
            depth_or_array_layers: iwi.faces,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = iwi.levels - skip;
    if cube {
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..Default::default()
        });
    }
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..Default::default()
    });
    Ok(image)
}

/// The normal and specular maps BO2's lit shaders read (the shine), by
/// their material slot hashes (`normalMap`, `specularMap`).
/// (`SpecularAndGloss`, 0x8c297e80: a weapon camo's specular colour and
/// gloss, which BO2's camo shader reads where the lit one reads its
/// specular map: Pack-a-Punch's blue and gold sheen.)
const SHINE_MAPS: [(u32, u8); 3] = [
    (0x59d3_0d0f, TS_NORMAL_MAP),
    (0x34ec_ccb3, TS_SPECULAR_MAP),
    (0x8c29_7e80, TS_SPECULAR_MAP),
];

/// The private semantic the reflection probes' cube array is linked under
/// (a material named `t6/reflection_probes` draws nothing).
pub const T6_PROBES_SEMANTIC: u8 = 0xf3;
pub const T6_PROBES_MATERIAL: &str = "t6/reflection_probes";

/// The world's reflection probes as one cube-array image. BO2 keeps each
/// probe's cube in the zone: BC3, 128x128, six faces each with its whole
/// mip chain, the order a GPU cube array is uploaded in. A probe in any
/// other form (Nuketown's probe 0 is a 4x4 stand-in) is black. Linked as
/// the private semantic of a material that draws nothing.
pub(crate) fn link_reflection_probes(
    catalog: &mut MaterialCatalog,
    capture: &ZoneCapture,
    world: &asset_t6::WorldRef,
    zone: &str,
) -> Option<String> {
    const SIZE: u32 = 128;
    const LEVELS: u32 = 8;
    if world.reflection_probes.is_empty() {
        return None;
    }
    let face_bytes: usize = (0..LEVELS)
        .map(|l| {
            let blocks = ((SIZE >> l).max(1) as usize).div_ceil(4);
            blocks * blocks * 16
        })
        .sum();
    let probe_bytes = face_bytes * 6;
    let mut data = Vec::with_capacity(probe_bytes * world.reflection_probes.len());
    let mut real = 0usize;
    for probe in &world.reflection_probes {
        let embedded = probe
            .image
            .and_then(|k| capture.images.get(k.index))
            .and_then(|img| img.embedded.as_ref())
            .filter(|e| matches!(e.dxgi_format, 77 | 78) && e.data.len() == probe_bytes);
        match embedded {
            Some(e) => {
                data.extend_from_slice(&e.data);
                real += 1;
            }
            None => data.resize(data.len() + probe_bytes, 0),
        }
    }
    let layers = 6 * world.reflection_probes.len() as u32;
    let mut image = Image::new_uninit(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        TextureFormat::Bc3RgbaUnorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = LEVELS;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::CubeArray),
        ..Default::default()
    });
    image.data = Some(data);
    let slot = catalog.link_image(AuthoredImage {
        namespace: AssetNamespace::T6,
        name: AssetRef::Real(T6_PROBES_MATERIAL.to_owned()),
        map_type: 5,
        semantic: T6_PROBES_SEMANTIC,
        category: 0,
        use_srgb_reads: false,
        width: SIZE as u16,
        height: SIZE as u16,
        depth: 1,
        level_count: LEVELS as u8,
        format: 0,
        payload: Arc::new(Vec::new()),
        decoded: Some(Arc::new(image)),
        common_owned: false,
        decoded_variant: None,
        decoded_by: None,
        pending_decode: None,
    });
    catalog.link_material(AuthoredMaterial {
        name: AssetRef::Real(T6_PROBES_MATERIAL.to_owned()),
        namespace: AssetNamespace::T6,
        technique_set: AssetRef::default(),
        technique_set_edge: AssetEdge::Absent,
        draw_surf: 0,
        sort_key: 0,
        info_game_flags: 0,
        texture_atlas: None,
        surface_type_bits: None,
        t5_layered_surface_types: None,
        state_flags: 0,
        camera_region: 0,
        state_bits: Vec::new(),
        state_bits_entry: None,
        t5_state_bits_entry: None,
        iw5_state_bits_entry: None,
        technique_table: None,
        route: None,
        textures: vec![MaterialTextureBinding {
            name_hash: 0,
            name_start: 0,
            name_end: 0,
            sampler_state: 0,
            semantic: T6_PROBES_SEMANTIC,
            image: Some(slot),
        }],
        constants: Vec::new(),
        zone: ZoneOwner::intern(zone),
        t6_draw: None,
    });
    Some(format!(
        "t6 reflection probes: {} ({real} cube maps, {} stand-ins black), one {SIZE}x{SIZE} cube array, {LEVELS} mips",
        world.reflection_probes.len(),
        world.reflection_probes.len() - real
    ))
}

/// Decode one captured image from the packs.
pub(crate) fn decode(packs: &PackSet, image: &ImageRef) -> Result<Image, String> {
    match packs.locate(image) {
        Some(ImageSource::Pack(i, entry)) => {
            let bytes = packs.packs[i].read(entry)?;
            let iwi = ipak_t6::parse_iwi(&bytes).map_err(|e| e.to_string())?;
            gpu_image(&iwi)
        }
        Some(ImageSource::Embedded) => Err("in-zone colour maps are not drawn yet".to_owned()),
        Some(ImageSource::Empty) => Err("no pixels".to_owned()),
        None => Err("in no pack".to_owned()),
    }
}

/// The texture a material is coloured with: its `colorMap`; else, among its
/// colour-map-semantic textures (layered and ember materials name their
/// slots differently), the one whose image is named as a colour map (`_c`,
/// `_col`), not an ember or glow layer (`_e`); else the first of them.
pub(crate) fn colour_texture<'a>(
    material: &'a asset_t6::MaterialRef,
    capture: &ZoneCapture,
) -> Option<&'a asset_t6::MaterialTexture> {
    // A colour map is named `_c` / `_col`; the burning-ember materials also
    // carry an animated glow noise named `_c` (`glowcycle_random_01_c`),
    // which sorts first in their table, so it never counts as colour.
    // Masks (`c_gen_arm_rim_mask_c`, 32x32 on the viewmodel arms) are named
    // like colour maps too; the real one is the largest.
    let colour_named = |t: &&asset_t6::MaterialTexture| {
        t.image
            .and_then(|k| capture.images.get(k.index))
            .is_some_and(|img| {
                (img.name.ends_with("_c") || img.name.ends_with("_col"))
                    && !img.name.contains("glowcycle")
                    && !img.name.contains("_mask")
            })
    };
    let area = |t: &&asset_t6::MaterialTexture| {
        t.image
            .and_then(|k| capture.images.get(k.index))
            .map_or(0u32, |img| u32::from(img.width) * u32::from(img.height))
    };
    let semantic = || {
        material
            .textures
            .iter()
            .filter(|t| t.semantic == TS_COLOR_MAP && t.image.is_some())
    };
    material
        .textures
        .iter()
        .find(|t| t.name_hash == COLOR_MAP_HASH && t.image.is_some())
        .or_else(|| semantic().filter(colour_named).max_by_key(area))
        .or_else(|| semantic().next())
}

/// A 1x1 stand-in for a colour map with no pixels here, by its name: white,
/// black, else mid grey ("grey").
fn flat_image(name: &str) -> Image {
    let n = name.to_ascii_lowercase();
    let v = if n.contains("white") {
        255
    } else if n.contains("black") {
        0
    } else {
        128
    };
    let mut image = Image::new_uninit(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(vec![v, v, v, 255]);
    image
}

/// An objective material's two colours as a 2x1 sRGB picture (linear
/// colours in, so a sampled texel gives them back).
fn objective_image(lo: [f32; 4], hi: [f32; 4]) -> Image {
    let enc = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    };
    let data = vec![
        enc(lo[0]),
        enc(lo[1]),
        enc(lo[2]),
        255,
        enc(hi[0]),
        enc(hi[1]),
        enc(hi[2]),
        255,
    ];
    let mut image = Image::new_uninit(
        Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image
}

/// Link the wanted capture materials into `catalog`, each with its colour
/// map decoded.
pub(crate) fn link_materials(
    catalog: &mut MaterialCatalog,
    capture: &ZoneCapture,
    packs: Option<&PackSet>,
    wanted: impl IntoIterator<Item = usize>,
    zone: &str,
) -> T6Materials {
    link_materials_with(catalog, capture, packs, wanted, zone, None)
}

/// `link_materials`, with the real images of other zones by name: a
/// comma-named image is a reference to one another zone defines.
pub(crate) fn link_materials_with(
    catalog: &mut MaterialCatalog,
    capture: &ZoneCapture,
    packs: Option<&PackSet>,
    wanted: impl IntoIterator<Item = usize>,
    zone: &str,
    foreign_images: Option<&HashMap<String, ImageRef>>,
) -> T6Materials {
    let owner = ZoneOwner::intern(zone);
    let mut images: HashMap<usize, Option<usize>> = HashMap::new();
    let mut local = HashMap::new();
    let (mut coloured, mut uncoloured, mut decoded) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    let mut draws: HashMap<T6Draw, usize> = HashMap::new();
    for mi in wanted {
        if local.contains_key(&mi) {
            continue;
        }
        let Some(material) = capture.materials.get(mi) else {
            continue;
        };
        let technique_set = material
            .technique_set
            .and_then(|k| capture.technique_sets.get(k.index))
            .map_or("", |t| t.name.as_str());
        // The main technique's draw state: lit with sun shadow, lit with
        // sun, lit, emissive, then unlit (the first the material draws in).
        let state = [6usize, 5, 4, 3, 2].iter().find_map(|&t| {
            let entry = *material.state_bits_entry.get(t)?;
            material.state_bits.get(usize::from(entry)).copied()
        });
        let mut draw = T6Draw::from_technique_set(technique_set, material.sort_key, state);
        // Unlit shaders scale their colour by the material's `scaleRGB`
        // (the light dome's is 32); powers of two up to 128 are kept.
        if draw.unlit {
            let scale = material
                .constants
                .iter()
                .find(|c| c.1.starts_with("scaleRGB"))
                .map_or(1.0, |c| c.2[0]);
            draw.unlit_scale_exp = scale.max(1.0).log2().round().clamp(0.0, 7.0) as u8;
        }
        *draws
            .entry(T6Draw {
                sort_key: 0,
                polygon_offset: 0,
                unlit_scale_exp: 0,
                layers: [0, 0],
                cull: asset_core::T6Cull::None,
                ..draw
            })
            .or_default() += 1;
        let mut textures = Vec::new();
        let mut link = |key: asset_t6::AssetKey,
                        catalog: &mut MaterialCatalog,
                        failures: &mut Vec<String>,
                        decoded: &mut usize|
         -> Option<_> {
            *images.entry(key.index).or_insert_with(|| {
                let mut image = capture.images.get(key.index)?;
                if let Some(real) = image
                    .name
                    .strip_prefix(',')
                    .and_then(|name| foreign_images?.get(name))
                {
                    image = real;
                }
                let decoded_image = match packs.map(|p| decode(p, image)) {
                    Some(Ok(gpu)) => {
                        *decoded += 1;
                        Some(Arc::new(gpu))
                    }
                    Some(Err(e)) => {
                        failures.push(format!("{}: {e}", image.name));
                        None
                    }
                    None => None,
                };
                let decoded_image = decoded_image?;
                Some(catalog.link_image(AuthoredImage {
                    namespace: AssetNamespace::T6,
                    name: AssetRef::Real(image.name.clone()),
                    map_type: image.map_type,
                    semantic: TS_COLOR_MAP,
                    category: 0,
                    use_srgb_reads: true,
                    width: image.width,
                    height: image.height,
                    depth: image.depth,
                    level_count: image.level_count,
                    format: 0,
                    payload: Arc::new(Vec::new()),
                    decoded: Some(decoded_image),
                    common_owned: false,
                    decoded_variant: None,
                    decoded_by: None,
                    pending_decode: None,
                }))
            })
        };
        // bo2zm M3: an objective material (the box's question marks) draws
        // its two pulse colours, not its (black) colour map: they go in a
        // 2x1 picture its shader reads (texel 0 colorObjMin, 1 colorObjMax).
        if draw.objective {
            let colour = |name: &str| {
                material
                    .constants
                    .iter()
                    .find(|c| c.1 == name)
                    .map_or([0.0; 4], |c| c.2)
            };
            let image = objective_image(colour("colorObjMin"), colour("colorObjMax"));
            let slot = catalog.link_image(AuthoredImage {
                namespace: AssetNamespace::T6,
                name: AssetRef::Real(format!("{}#objective", material.name)),
                map_type: 0,
                semantic: TS_COLOR_MAP,
                category: 0,
                use_srgb_reads: true,
                width: 2,
                height: 1,
                depth: 1,
                level_count: 1,
                format: 0,
                payload: Arc::new(Vec::new()),
                decoded: Some(Arc::new(image)),
                common_owned: false,
                decoded_variant: None,
                decoded_by: None,
                pending_decode: None,
            });
            textures.push(MaterialTextureBinding {
                name_hash: 0xa0ab_1041,
                name_start: 0,
                name_end: 0,
                sampler_state: 0x14,
                semantic: TS_COLOR_MAP,
                image: Some(slot),
            });
        } else if let Some(texture) = colour_texture(material, capture)
            && let Some(key) = texture.image
        {
            // bo2zm M3 fix list 1: a colour map whose pixels are in no pack
            // here (a reference into another zone, e.g. the leftover Maya
            // `lambert1` material's "grey") draws flat, as its name says,
            // not in the untextured diagnostic colours (his "weird
            // splotches ... of random colors").
            let slot = link(key, catalog, &mut failures, &mut decoded).unwrap_or_else(|| {
                let name = capture
                    .images
                    .get(key.index)
                    .map_or("", |i| i.name.as_str());
                catalog.link_image(AuthoredImage {
                    namespace: AssetNamespace::T6,
                    name: AssetRef::Real(format!("{name}#flat")),
                    map_type: 0,
                    semantic: TS_COLOR_MAP,
                    category: 0,
                    use_srgb_reads: true,
                    width: 1,
                    height: 1,
                    depth: 1,
                    level_count: 1,
                    format: 0,
                    payload: Arc::new(Vec::new()),
                    decoded: Some(Arc::new(flat_image(name))),
                    common_owned: false,
                    decoded_variant: None,
                    decoded_by: None,
                    pending_decode: None,
                })
            });
            textures.push(MaterialTextureBinding {
                name_hash: texture.name_hash,
                name_start: 0,
                name_end: 0,
                sampler_state: texture.sampler_state,
                semantic: TS_COLOR_MAP,
                image: Some(slot),
            });
        }
        // A layered material's extra colour maps (the fast quality setting
        // draws the base layer alone).
        if asset_core::t6_fast() {
            draw.layers = [0, 0];
        }
        if draw.layers.iter().any(|&k| k != 0) {
            for (hash, semantic) in LAYER_MAPS {
                if let Some(texture) = material.textures.iter().find(|t| t.name_hash == hash)
                    && let Some(key) = texture.image
                    && let Some(slot) = link(key, catalog, &mut failures, &mut decoded)
                {
                    textures.push(MaterialTextureBinding {
                        name_hash: texture.name_hash,
                        name_start: 0,
                        name_end: 0,
                        sampler_state: texture.sampler_state,
                        semantic,
                        image: Some(slot),
                    });
                }
            }
        }
        // The normal and specular maps BO2's lit shaders read (the shine);
        // the fast quality setting draws without them.
        // (`IW4L_T6_NO_SHINE` leaves them out: a test aid for comparing.)
        if !asset_core::t6_fast() && !draw.unlit && std::env::var_os("IW4L_T6_NO_SHINE").is_none() {
            for (hash, semantic) in SHINE_MAPS {
                if let Some(texture) = material.textures.iter().find(|t| t.name_hash == hash)
                    && let Some(key) = texture.image
                    && let Some(slot) = link(key, catalog, &mut failures, &mut decoded)
                {
                    textures.push(MaterialTextureBinding {
                        name_hash: texture.name_hash,
                        name_start: 0,
                        name_end: 0,
                        sampler_state: texture.sampler_state,
                        semantic,
                        image: Some(slot),
                    });
                }
            }
        }
        if textures.is_empty() {
            uncoloured += 1;
        } else {
            coloured += 1;
        }
        let index = catalog.link_material(AuthoredMaterial {
            name: AssetRef::Real(material.name.clone()),
            namespace: AssetNamespace::T6,
            technique_set: AssetRef::default(),
            technique_set_edge: AssetEdge::Absent,
            draw_surf: 0,
            sort_key: material.sort_key,
            info_game_flags: 0,
            texture_atlas: None,
            surface_type_bits: None,
            t5_layered_surface_types: None,
            state_flags: 0,
            camera_region: 0,
            state_bits: Vec::new(),
            state_bits_entry: None,
            t5_state_bits_entry: None,
            iw5_state_bits_entry: None,
            technique_table: None,
            route: None,
            textures,
            constants: material
                .constants
                .iter()
                .map(|(hash, name, literal)| {
                    let mut short = [0u8; 12];
                    for (d, b) in short.iter_mut().zip(name.bytes()) {
                        *d = b;
                    }
                    asset_material::MaterialConstant {
                        name_hash: *hash,
                        name: short,
                        literal: *literal,
                    }
                })
                .collect(),
            zone: owner,
            t6_draw: Some(draw),
        });
        local.insert(mi, index);
    }
    let mut report = vec![format!(
        "t6 materials: {} linked ({coloured} with a colour map, {uncoloured} without), {decoded} colour maps decoded from packs, {} failed; draws {:?}",
        local.len(),
        failures.len(),
        draws
    )];
    for f in failures.iter().take(8) {
        report.push(format!("t6 colour map gap: {f}"));
    }
    T6Materials { local, report }
}

/// bo2zm M4: whether a material adds its colour onto what is under it
/// (blend ONE/ONE, or SRC_ALPHA/ONE) - BO2's menu brackets and glows,
/// black where nothing shows.
pub(crate) fn is_additive(material: &asset_t6::MaterialRef) -> bool {
    (0..36).filter_map(|t| material.draw_state(t)).any(|d| d.dst_blend == 2 && matches!(d.src_blend, 2 | 5))
}

/// bo2zm M4: an additive picture for a 2D layer that only alpha-blends: its
/// top level as RGBA with the brightness as alpha (black = see-through),
/// colour divided back out.
pub(crate) fn additive_to_alpha(image: &Image) -> Option<Image> {
    let w = image.texture_descriptor.size.width as usize;
    let h = image.texture_descriptor.size.height as usize;
    let data = image.data.as_ref()?;
    let (block, decode): (usize, fn(&[u8], &mut [u8], usize)) = match image.texture_descriptor.format {
        TextureFormat::Bc1RgbaUnormSrgb | TextureFormat::Bc1RgbaUnorm => (8, bcdec_rs::bc1),
        TextureFormat::Bc2RgbaUnormSrgb | TextureFormat::Bc2RgbaUnorm => (16, bcdec_rs::bc2),
        TextureFormat::Bc3RgbaUnormSrgb | TextureFormat::Bc3RgbaUnorm => (16, bcdec_rs::bc3),
        TextureFormat::Rgba8UnormSrgb => (0, |_, _, _| {}),
        _ => return None,
    };
    let mut rgba = vec![0u8; w * h * 4];
    if block == 0 {
        rgba.copy_from_slice(data.get(..w * h * 4)?);
    } else {
        let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
        let mut tile = [0u8; 64];
        for by in 0..bh {
            for bx in 0..bw {
                let at = (by * bw + bx) * block;
                decode(data.get(at..at + block)?, &mut tile, 16);
                for y in 0..4 {
                    for x in 0..4 {
                        let (px, py) = (bx * 4 + x, by * 4 + y);
                        if px < w && py < h {
                            let d = (py * w + px) * 4;
                            let s = (y * 4 + x) * 4;
                            rgba[d..d + 4].copy_from_slice(&tile[s..s + 4]);
                        }
                    }
                }
            }
        }
    }
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[0].max(px[1]).max(px[2]));
        let alpha = a * u32::from(px[3]) / 255;
        if a > 0 {
            for c in &mut px[..3] {
                *c = (u32::from(*c) * 255 / a).min(255) as u8;
            }
        }
        px[3] = alpha as u8;
    }
    let mut out = Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    out.sampler = image.sampler.clone();
    Some(out)
}
