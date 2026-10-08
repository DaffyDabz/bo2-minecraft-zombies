//! The Minecraft map's inventory and hotbar, between the world that owns the
//! vanilla inventory (`render_anim`) and the HUD that draws it and takes the
//! player's clicks (`hud`).
use std::collections::HashMap;

use bevy::prelude::*;

/// Slots of the vanilla player inventory: 0..9 hotbar, 9..36 main, 36..40
/// armor (feet to head), 40 offhand.
pub const MC_INVENTORY_SLOTS: usize = 41;
pub const MC_HOTBAR: usize = 9;
/// Minecraft's compass pictures (`item/compass_00` to `_31`).
pub const COMPASS_FRAMES: u32 = 32;

/// One stack as the HUD shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct McStack {
    /// The item's id (`minecraft:dirt`), or `iw4:weapon/<index>` for a gun.
    pub id: String,
    pub count: u8,
    /// The MW2 weapon this item is, for a gun.
    pub weapon: Option<u32>,
    /// Durability left of the item's maximum, for a damaged tool.
    pub durability: Option<f32>,
}

/// bo2mc: the item picture a BO2 weapon without its own (the knife) shows.
pub const WEAPON_FALLBACK_ICON: &str = "minecraft:iron_sword";

/// A slot of the inventory screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McSlot {
    Inventory(usize),
    Crafting(usize),
    Result,
    /// bo2mc: a slot of the open furnace (0 input, 1 fuel, 2 output) or
    /// chest (0..27).
    Container(usize),
}

/// What the player did on the inventory screen, applied with vanilla's
/// container click rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McClick {
    /// A click on a slot; shift moves the stack across.
    Slot { slot: McSlot, right: bool, shift: bool },
    /// A click outside the window: throws the carried stack (or one of it).
    Outside { right: bool },
    /// A double click: gathers matching stacks onto the carried one.
    Gather { right: bool },
    /// A number key over a slot: swaps it with that hotbar slot.
    Swap { slot: McSlot, hotbar: usize },
    /// A drag across slots with a carried stack: shares it out.
    Spread { slots: Vec<usize>, right: bool },
    /// The screen closed: the carried stack and the crafting grid go back.
    Close,
}

#[derive(Resource, Default)]
pub struct MinecraftUi {
    /// A Minecraft world is in play and the player is alive.
    pub active: bool,
    /// The match's Minecraft world is still being generated; the loading
    /// screen holds until it is in, so its setup never lands mid-match.
    pub loading_world: bool,
    /// The inventory screen is open (the HUD's to change).
    pub inventory_open: bool,
    /// The selected hotbar slot.
    pub selected: usize,
    pub slots: Vec<Option<McStack>>,
    /// The crafting grid: 2x2, or 3x3 at a crafting table.
    pub crafting: Vec<Option<McStack>>,
    pub result: Option<McStack>,
    /// The stack on the cursor.
    pub cursor: Option<McStack>,
    /// Item icons: one atlas, and each item's rectangle in it (s0 t0 s1 t1).
    pub icons: Option<Handle<Image>>,
    pub icon_rects: HashMap<String, [f32; 4]>,
    /// Display names by item id.
    pub names: HashMap<String, String>,
    /// Clicks the HUD took this frame, for the inventory's owner.
    pub clicks: Vec<McClick>,
    /// A hotbar slot the player picked (scroll or number key).
    pub select: Option<usize>,
    /// A drop of the selected stack (`Q`; with the whole stack on control).
    pub drop_selected: Option<bool>,
    /// The mouse in the inventory's character box, -1..1 across and down,
    /// which the character's gaze follows.
    pub gaze: [f32; 2],
    /// The character box in window pixels (centre x and y, width, height),
    /// for placing the character.
    pub character_box: Option<[f32; 4]>,
    /// The name of the item just selected and how long ago, for the hotbar.
    pub selected_name: Option<(String, f32)>,
    /// The MW2 gun the hotbar selection asks the player to raise.
    pub weapon_request: Option<u32>,
    /// The selection is not a gun: the hand or a held item shows, and the
    /// gun neither fires nor aims.
    pub holding_item: bool,
    /// The selected slot is empty: MW2's bare hands show, without the gun.
    pub empty_hand: bool,
    /// How far through its swing the hand is, 0 to 1.
    pub hand_swing: f32,
    /// The minimap's picture of the world and the map points of its
    /// north-west and south-east corners.
    pub minimap: Option<(Handle<Image>, [f32; 2], [f32; 2])>,
    /// bo2mc: a line of text centred on the screen (a warning).
    pub notice: Option<String>,
    /// bo2mc: the compass at the top right (his 10-08): Minecraft's compass
    /// pictures stacked top to bottom (square, RGBA), and the one showing.
    pub compass: Option<(std::sync::Arc<Vec<u8>>, u32)>,
    /// bo2mc: a door is in reach under the crosshair, or a BO2 use prompt
    /// is up (a wall buy, the box, a machine), so the use key (E in his
    /// layout) opens or buys rather than opening the inventory.
    pub door_in_reach: bool,
    /// bo2mc: Minecraft Zombies is on (BO2's guns, grenades and knife are
    /// hotbar items; V does not knife).
    pub bo2mc: bool,
    /// bo2mc: what the left click does with the selected item: 0 the gun
    /// or the Minecraft item, 1 throw the grenade, 2 swing the knife.
    pub held_action: u8,
    /// bo2mc: Minecraft's hearts, hunger, armor and air bars.
    pub vitals: Option<McVitals>,
    /// The screen is a crafting table's (a 3x3 grid; `McSlot::Crafting`
    /// 0..9 and `McSlot::Result` are its).
    pub workbench: bool,
    /// bo2mc: the screen is a furnace's (1) or a chest's (2), whose slots
    /// are `container` (`McSlot::Container`); 0 = none.
    pub container_kind: u8,
    pub container: Vec<Option<McStack>>,
    /// The open furnace's fire left and its cooking done, 0 to 1.
    pub furnace_burn: f32,
    pub furnace_cook: f32,
    /// bo2mc: Minecraft's chat (T, or / to start a command) is open and
    /// takes the keyboard; `chat_line` is what is typed so far.
    pub chat_open: bool,
    pub chat_line: String,
    /// Lines the player sent this frame (Enter), for the world to answer.
    pub chat_submitted: Vec<String>,
    /// What the chat shows: the newest last; each fades 10 s after `at`.
    pub chat_log: Vec<McChatLine>,
}

/// bo2mc: one line of chat, its colour (RGB 0..1) and when it came (seconds).
#[derive(Clone, Debug)]
pub struct McChatLine {
    pub text: String,
    pub color: [f32; 3],
    pub at: f64,
}

/// bo2mc: what the hearts, hunger, armor and air bars show.
#[derive(Clone, Copy, Debug, Default)]
pub struct McVitals {
    /// Health in Minecraft points (2 a heart); above 20 shows as golden.
    pub health: f32,
    pub max_health: f32,
    /// Food level 0..20 (2 a drumstick).
    pub food: u8,
    /// Armor points 0..20.
    pub armor: u8,
    /// Air 0..300 while under water.
    pub air: Option<i32>,
}

/// The player's own MW2 body, drawn standing in the inventory's character
/// box and looking towards the mouse.
#[derive(Resource, Default, Clone)]
pub struct InventoryPuppet {
    pub active: bool,
    pub client: u32,
    /// Where the body stands, in world space (in front of the camera).
    pub root: Mat4,
    /// The aim pitch its upper body and head take, in degrees.
    pub pitch: f32,
    /// bo2mc: the Black Ops II body (a script model's entity number) that
    /// stands at `root` instead of the MW2 body.
    pub bo2_entnum: Option<u32>,
}
