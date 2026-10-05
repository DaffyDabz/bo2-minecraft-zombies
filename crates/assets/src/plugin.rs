use bevy::prelude::*;

use crate::match_load::register_match_load_systems;
use crate::teardown::register_match_teardown;

pub struct AssetPlugin;

impl Plugin for AssetPlugin {
    fn build(&self, app: &mut App) {
        register_match_load_systems(app);
        register_match_teardown(app);
        // The Minecraft map's files come from Mojang in the background.
        app.add_systems(Startup, crate::minecraft_map::prepare);
    }
}
