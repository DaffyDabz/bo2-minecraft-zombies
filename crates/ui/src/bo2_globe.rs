//! bo2zm M4: Black Ops II's Zombies globe, drawn with its own shader
//! (`bo2_globe.wgsl`, ported from the globe material's pixel shader): the
//! front end turns it with the element's shader vector 2 and reveals it
//! with vector 0, as `ui_mp/t6/zombie/gameglobezombie.lua` drives it.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};

const SHADER_PATH: &str = "embedded://ui/bo2_globe.wgsl";

/// The globe's values: shader vectors 0 and 2, the element's colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, bevy::render::render_resource::ShaderType)]
pub(crate) struct GlobeParams {
    pub v0: Vec4,
    pub v2: Vec4,
    pub color: Vec4,
    /// x: how far a popup's blur smears it, across its picture (0 sharp).
    pub blur: Vec4,
}

/// The globe: its values, the mesh (the grid) and the Earth's day map.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub(crate) struct GlobeMaterial {
    #[uniform(0)]
    pub params: GlobeParams,
    #[texture(1)]
    #[sampler(2)]
    pub mesh: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub day: Handle<Image>,
}

impl UiMaterial for GlobeMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
}

/// The globe's node: its material made once, its values set each frame.
pub(crate) fn globe_node(
    materials: &mut Assets<GlobeMaterial>,
    params: GlobeParams,
    mesh: Handle<Image>,
    day: Handle<Image>,
) -> MaterialNode<GlobeMaterial> {
    MaterialNode(materials.add(GlobeMaterial { params, mesh, day }))
}

pub(crate) fn register_globe(app: &mut App) {
    bevy::asset::embedded_asset!(app, "bo2_globe.wgsl");
    app.add_plugins(UiMaterialPlugin::<GlobeMaterial>::default());
}
