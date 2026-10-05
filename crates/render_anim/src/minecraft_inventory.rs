//! The player's inventory on the Minecraft map: MinecraftOSS's vanilla
//! `Inventory` (slots, stacking, container clicks, the 2x2 crafting grid),
//! published for the HUD to draw with item icons and names made the way the
//! viewer makes them. The player's MW2 guns are items in it too: selecting
//! one on the hotbar raises that gun.
use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use frame::{McClick, McSlot, McStack, MinecraftUi};
use glam::DVec3;
use minecraft_terrain::pack::PackStack;
use minecraftoss_player::inventory::{Inventory, ItemStack};

/// Items that are MW2 guns: `iw4:weapon/<weapon index>`.
const WEAPON_PREFIX: &str = "iw4:weapon/";
/// The item picture a weapon without its own shows (`frame::McStack`).
const WEAPON_FALLBACK_ICON: &str = frame::minecraft_ui::WEAPON_FALLBACK_ICON;
/// Icon cells: 32 pixels (a 16-pixel item at GUI scale 2), 16 to a row.
const ICON: u32 = 32;
const ATLAS: u32 = ICON * 16;

/// Minecraft's HUD sprites the bars draw (`textures/gui/sprites/...`).
const GUI_SPRITES: [&str; 15] = [
    "hud/heart/container",
    "hud/heart/full",
    "hud/heart/half",
    "hud/heart/absorbing_full",
    "hud/heart/absorbing_half",
    "hud/heart/hardcore_full",
    "hud/food_empty",
    "hud/food_full",
    "hud/food_half",
    "hud/armor_empty",
    "hud/armor_full",
    "hud/armor_half",
    "hud/air",
    "hud/air_bursting",
    "hud/heart/frozen_full",
];

/// Minecraft's ASCII font sheet (16 by 16 letters of 8 pixels).
fn font_sheet(packs: &PackStack) -> anyhow::Result<image::RgbaImage> {
    let id = minecraft_terrain::pack::ResourceId::parse("minecraft:font/ascii")?;
    let bytes = packs.texture(&id)?.ok_or_else(|| anyhow::anyhow!("no font/ascii"))?;
    Ok(image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8())
}

/// One letter of the sheet, four times its size and trimmed to its width
/// (as Minecraft measures it: up to its last lit column); a space is 3
/// pixels wide.
fn font_glyph(sheet: &image::RgbaImage, code: u32) -> Option<image::RgbaImage> {
    let cell = sheet.width() / 16;
    if cell == 0 {
        return None;
    }
    let (cx, cy) = ((code % 16) * cell, (code / 16) * cell);
    let lit = |x: u32| (0..cell).any(|y| sheet.get_pixel(cx + x, cy + y)[3] > 0);
    let width = if code == 32 { cell * 3 / 8 } else { (0..cell).rev().find(|&x| lit(x)).map_or(0, |x| x + 1) };
    if width == 0 {
        return None;
    }
    let part = image::imageops::crop_imm(sheet, cx, cy, width, cell).to_image();
    let scale = 32 / cell.max(1);
    Some(image::imageops::resize(&part, width * scale, cell * scale, image::imageops::FilterType::Nearest))
}

/// A HUD sprite, three times its size (9 pixels to 27, whole pixels).
fn gui_sprite(packs: &PackStack, path: &str) -> anyhow::Result<Option<image::RgbaImage>> {
    let id = minecraft_terrain::pack::ResourceId::parse(&format!("minecraft:gui/sprites/{path}"))?;
    let Some(bytes) = packs.texture(&id)? else {
        return Ok(None);
    };
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8();
    let scale = (ICON / img.width().max(img.height()).max(1)).max(1);
    Ok(Some(image::imageops::resize(&img, img.width() * scale, img.height() * scale, image::imageops::FilterType::Nearest)))
}

pub(crate) fn weapon_of(stack: &ItemStack) -> Option<u32> {
    stack.id.strip_prefix(WEAPON_PREFIX)?.parse().ok()
}

fn weapon_item(weapon: u32) -> ItemStack {
    let mut stack = ItemStack::new(format!("{WEAPON_PREFIX}{weapon}"), 1);
    stack.max = 1;
    stack
}

/// bo2mc: the furnace or chest whose screen is open.
pub(crate) enum OpenContainer<'a> {
    None,
    Furnace(&'a mut minecraftoss_player::furnace::Furnace),
    Chest(&'a mut minecraftoss_player::chest::Chest),
}

impl OpenContainer<'_> {
    fn slots(&mut self) -> Option<&mut [Option<ItemStack>]> {
        match self {
            OpenContainer::None => None,
            OpenContainer::Furnace(f) => Some(&mut f.slots[..]),
            OpenContainer::Chest(c) => Some(&mut c.slots[..]),
        }
    }
}

#[derive(Default)]
pub(crate) struct InventoryUi {
    atlas: Option<image::RgbaImage>,
    handle: Option<Handle<Image>>,
    cells: HashMap<String, u32>,
    failed: HashSet<String>,
    language: Option<HashMap<String, String>>,
    dirty: bool,
    /// The gun last asked for, until the player state holds it.
    pending_weapon: Option<u32>,
    last_selected: Option<usize>,
    /// Minecraft's `font/ascii` sheet, for the HUD's letters.
    font_sheet: Option<image::RgbaImage>,
}

/// What the player holds and where it looks, for throwing items.
pub(crate) struct Thrower {
    pub eye: DVec3,
    /// Minecraft yaw and pitch, in degrees.
    pub yaw: f32,
    pub pitch: f32,
}

impl InventoryUi {
    /// Keeps the player's MW2 guns in the inventory: each gun the player has
    /// is one item, placed on the first free hotbar slot when it arrives, and
    /// a gun the player no longer has leaves.
    /// Each owned weapon comes with its item's count (a grenade's is how
    /// many; a gun's 1). A gun that replaces one (a box or wall buy when
    /// the hands are full) takes the slot the old one left.
    pub(crate) fn sync_weapons(&mut self, inventory: &mut Inventory, owned: &[(u32, u8)]) {
        let count_of = |w: u32| owned.iter().find(|(o, _)| *o == w).map(|&(_, n)| n);
        let keep = |stack: &Option<ItemStack>| stack.as_ref().is_none_or(|s| weapon_of(s).is_none_or(|w| count_of(w).is_some()));
        let mut freed = Vec::new();
        for (i, slot) in inventory.slots.iter_mut().enumerate() {
            if !keep(slot) {
                *slot = None;
                freed.push(i);
            }
        }
        for slot in inventory.crafting.iter_mut() {
            if !keep(slot) {
                *slot = None;
            }
        }
        if !keep(&inventory.cursor) {
            inventory.cursor = None;
        }
        // One stack per weapon, at its count.
        let mut seen = HashSet::new();
        for slot in inventory
            .slots
            .iter_mut()
            .chain(inventory.crafting.iter_mut())
            .chain(std::iter::once(&mut inventory.cursor))
        {
            let Some(w) = slot.as_ref().and_then(weapon_of) else {
                continue;
            };
            if !seen.insert(w) {
                *slot = None;
                continue;
            }
            if let (Some(stack), Some(n)) = (slot.as_mut(), count_of(w)) {
                stack.count = n.max(1);
                stack.max = n.max(1);
            }
        }
        let held: HashSet<u32> = inventory
            .slots
            .iter()
            .chain(inventory.crafting.iter())
            .chain(std::iter::once(&inventory.cursor))
            .filter_map(|s| s.as_ref().and_then(weapon_of))
            .collect();
        for &(weapon, count) in owned {
            if held.contains(&weapon) {
                continue;
            }
            // A new gun belongs on the hotbar (playtest 1b): with the
            // hotbar full, its last Minecraft item moves to the main
            // inventory to make room.
            let hotbar = frame::minecraft_ui::MC_HOTBAR;
            if freed.is_empty()
                && (0..hotbar).all(|i| inventory.slots[i].is_some())
                && let Some(spare) = (hotbar..36).find(|&i| inventory.slots[i].is_none())
                && let Some(from) = (0..hotbar).rev().find(|&i| inventory.slots[i].as_ref().is_some_and(|s| weapon_of(s).is_none()))
            {
                inventory.slots[spare] = inventory.slots[from].take();
            }
            let free = freed
                .iter()
                .copied()
                .chain(0..frame::minecraft_ui::MC_HOTBAR)
                .chain(frame::minecraft_ui::MC_HOTBAR..36)
                .find(|&i| inventory.slots[i].is_none());
            if let Some(i) = free {
                let mut item = weapon_item(weapon);
                item.count = count.max(1);
                item.max = count.max(1);
                inventory.slots[i] = Some(item);
            }
        }
    }

    /// Applies the HUD's clicks, selection and drops with vanilla's rules.
    /// Stacks thrown out of the inventory are returned, except guns, which
    /// stay.
    pub(crate) fn apply_input(
        &mut self,
        ui: &mut MinecraftUi,
        inventory: &mut Inventory,
        selected: &mut usize,
        mut container: OpenContainer<'_>,
    ) -> Vec<ItemStack> {
        let mut thrown = Vec::new();
        for click in std::mem::take(&mut ui.clicks) {
            match click {
                // bo2mc: the open furnace's or chest's slots, and shift on
                // the inventory moves a stack across into it.
                McClick::Slot { slot: McSlot::Container(index), right, shift } => match &mut container {
                    OpenContainer::Furnace(f) => f.click_slot(index, right, shift, inventory),
                    OpenContainer::Chest(c) => c.click_slot(index, right, shift, inventory),
                    OpenContainer::None => {}
                },
                McClick::Slot { slot: McSlot::Inventory(index), shift: true, .. }
                    if index < 36 && !matches!(container, OpenContainer::None) =>
                {
                    match &mut container {
                        OpenContainer::Furnace(f) => f.quick_move_from_inventory(index, inventory),
                        OpenContainer::Chest(c) => c.quick_move_from_inventory(index, inventory),
                        OpenContainer::None => {}
                    }
                }
                McClick::Slot { slot: McSlot::Inventory(index), right, shift } => {
                    if let Some(stack) = inventory.click(Some(index), right, shift) {
                        thrown.push(stack);
                    }
                }
                McClick::Slot { slot: McSlot::Crafting(index), right, shift } if ui.workbench => {
                    inventory.click_workbench_slot(index, right, shift);
                }
                McClick::Slot { slot: McSlot::Result, shift, .. } if ui.workbench => {
                    inventory.take_workbench_output(shift);
                }
                McClick::Slot { slot: McSlot::Crafting(index), right, shift } => {
                    inventory.click_crafting_slot(index, right, shift);
                }
                McClick::Slot { slot: McSlot::Result, shift, .. } => {
                    inventory.take_crafting_output(shift);
                }
                McClick::Outside { right } => {
                    if let Some(stack) = inventory.click(None, right, false) {
                        thrown.push(stack);
                    }
                }
                McClick::Gather { right } => inventory.pickup_all(right),
                McClick::Swap { slot: McSlot::Inventory(index), hotbar } => inventory.number_swap(index, hotbar),
                McClick::Swap { .. } => {}
                McClick::Spread { slots, right } => inventory.distribute(&slots, right),
                McClick::Close => {
                    thrown.extend(inventory.settle_crafting());
                    thrown.extend(inventory.settle_workbench());
                    if let Some(rest) = inventory.settle_cursor() {
                        thrown.push(rest);
                    }
                }
            }
        }
        thrown.extend(inventory.take_pending_drops());
        // A gun stays with the player: one put in a furnace or chest comes
        // straight back.
        if let Some(slots) = container.slots() {
            for slot in slots.iter_mut() {
                if slot.as_ref().is_some_and(|s| weapon_of(s).is_some())
                    && let Some(stack) = slot.take()
                    && let Some(back) = inventory.add_item(stack, *selected)
                {
                    *slot = Some(back);
                }
            }
        }
        if let Some(slot) = ui.select.take() {
            *selected = slot.min(frame::minecraft_ui::MC_HOTBAR - 1);
        }
        if let Some(whole) = ui.drop_selected.take()
            && let Some(stack) = inventory.drop_selected(*selected, whole)
        {
            thrown.push(stack);
        }
        // A gun never leaves the inventory.
        let mut out = Vec::new();
        for stack in thrown {
            if weapon_of(&stack).is_some() {
                if let Some(back) = inventory.add_item(stack, *selected) {
                    inventory.cursor = Some(back);
                }
            } else {
                out.push(stack);
            }
        }
        out
    }

    /// The gun the hotbar selection asks for, if the player holds another;
    /// and the selection follows a gun MW2 raised on its own.
    pub(crate) fn weapon_request(
        &mut self,
        inventory: &Inventory,
        selected: &mut usize,
        held: u32,
        is_gun: &dyn Fn(u32) -> bool,
    ) -> Option<u32> {
        let selected_gun = inventory.slots[*selected].as_ref().and_then(weapon_of).filter(|&w| is_gun(w));
        let changed = self.last_selected != Some(*selected);
        self.last_selected = Some(*selected);
        if self.pending_weapon == Some(held) {
            self.pending_weapon = None;
        }
        if changed {
            if let Some(gun) = selected_gun.filter(|&gun| gun != held) {
                self.pending_weapon = Some(gun);
            }
        } else if self.pending_weapon.is_none()
            && selected_gun.is_some_and(|gun| gun != held)
            && let Some(slot) = (0..frame::minecraft_ui::MC_HOTBAR)
                .find(|&i| inventory.slots[i].as_ref().and_then(weapon_of) == Some(held))
        {
            // MW2 changed weapons (out of ammo, a pickup): the hotbar follows.
            *selected = slot;
            self.last_selected = Some(slot);
        }
        self.pending_weapon
    }

    /// Publishes the inventory for the HUD, with icons for every item in it.
    pub(crate) fn publish(
        &mut self,
        ui: &mut MinecraftUi,
        inventory: &Inventory,
        selected: usize,
        container: &[Option<ItemStack>],
        packs: &PackStack,
        images: &mut Assets<Image>,
    ) {
        let language = self
            .language
            .get_or_insert_with(|| minecraft_terrain::item_icons::language(packs).unwrap_or_default());
        let mut stacks = Vec::new();
        let mut convert = |stack: &Option<ItemStack>| {
            stack.as_ref().map(|s| {
                stacks.push(s.id.clone());
                McStack {
                    id: s.id.clone(),
                    count: s.count,
                    weapon: weapon_of(s),
                    durability: None,
                }
            })
        };
        ui.slots = inventory.slots.iter().take(frame::minecraft_ui::MC_INVENTORY_SLOTS).map(&mut convert).collect();
        if ui.workbench {
            ui.crafting = inventory.workbench.iter().map(&mut convert).collect();
            ui.result = convert(&inventory.workbench_output());
        } else {
            ui.crafting = inventory.crafting.iter().map(&mut convert).collect();
            ui.result = convert(&inventory.crafting_output());
        }
        ui.cursor = convert(&inventory.cursor);
        ui.container = container.iter().map(&mut convert).collect();
        if ui.selected != selected {
            let name = inventory.slots[selected]
                .as_ref()
                .filter(|s| weapon_of(s).is_none())
                .map(|s| minecraft_terrain::item_icons::item_name(language, &s.id));
            ui.selected_name = name.map(|name| (name, 0.0));
        }
        ui.selected = selected;
        // Minecraft's HUD sprites (hearts, hunger, armor, air) ride in the
        // same atlas as `gui:<sprite path>`.
        // And Minecraft's font (`font:<code>`, each letter trimmed to its
        // width), as a BO2 map has no IW4 HUD fonts.
        let glyphs = (32u32..127).map(|c| format!("font:{c}"));
        // bo2mc: a BO2 weapon without a picture of its own (the knife) shows
        // as an iron sword (the HUD falls back to it).
        let fallback = std::iter::once(WEAPON_FALLBACK_ICON.to_owned());
        for id in stacks.into_iter().chain(GUI_SPRITES.iter().map(|s| format!("gui:{s}"))).chain(glyphs).chain(fallback) {
            if id.starts_with(WEAPON_PREFIX) {
                continue;
            }
            let gui = id.strip_prefix("gui:").map(str::to_owned);
            let glyph = id.strip_prefix("font:").and_then(|c| c.parse::<u32>().ok());
            if gui.is_none() && glyph.is_none() && !ui.names.contains_key(&id) {
                ui.names.insert(id.clone(), minecraft_terrain::item_icons::item_name(language, &id));
            }
            if self.cells.contains_key(&id) || self.failed.contains(&id) {
                continue;
            }
            let cell = self.cells.len() as u32;
            if cell >= (ATLAS / ICON) * (ATLAS / ICON) {
                continue;
            }
            let made = match (gui.as_deref(), glyph) {
                (Some(path), _) => gui_sprite(packs, path).map_err(|e| e.to_string()),
                (None, Some(code)) => {
                    let sheet = self.font_sheet.get_or_insert_with(|| font_sheet(packs).unwrap_or_default());
                    Ok(font_glyph(sheet, code))
                }
                (None, None) => minecraft_terrain::item_icons::item_icon(packs, &id, ICON as usize).map_err(|e| e.to_string()),
            };
            match made {
                Ok(Some(icon)) => {
                    let atlas = self.atlas.get_or_insert_with(|| image::RgbaImage::new(ATLAS, ATLAS));
                    let (x, y) = ((cell % (ATLAS / ICON)) * ICON, (cell / (ATLAS / ICON)) * ICON);
                    // A sprite keeps its own size (scaled whole); an item
                    // fills the cell.
                    let (w, h) = if gui.is_some() || glyph.is_some() {
                        (icon.width().min(ICON), icon.height().min(ICON))
                    } else {
                        (ICON, ICON)
                    };
                    let icon = image::imageops::resize(&icon, w, h, image::imageops::FilterType::Nearest);
                    image::imageops::replace(atlas, &icon, i64::from(x), i64::from(y));
                    let size = ATLAS as f32;
                    ui.icon_rects.insert(
                        id.clone(),
                        [x as f32 / size, y as f32 / size, (x + w) as f32 / size, (y + h) as f32 / size],
                    );
                    self.cells.insert(id, cell);
                    self.dirty = true;
                }
                Ok(None) => {
                    diag::warn!(World, "Minecraft item icon: no model for `{id}`");
                    self.failed.insert(id);
                }
                Err(error) => {
                    diag::warn!(World, "Minecraft item icon for `{id}` failed: {error}");
                    self.failed.insert(id);
                }
            }
        }
        if self.dirty
            && let Some(atlas) = self.atlas.as_ref()
        {
            self.dirty = false;
            let image = Image {
                sampler: ImageSampler::nearest(),
                ..Image::new(
                    Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    atlas.as_raw().clone(),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                )
            };
            match self.handle.as_ref() {
                Some(handle) => {
                    let _ = images.insert(handle.id(), image);
                }
                None => self.handle = Some(images.add(image)),
            }
            ui.icons = self.handle.clone();
        }
    }
}

/// Throws stacks from the player's eye, as `Player.drop` does.
pub(crate) fn throw(items: &mut minecraftoss_player::items::WorldItems, stacks: Vec<ItemStack>, thrower: &Thrower) {
    for stack in stacks {
        items.toss(stack, thrower.eye, thrower.yaw, thrower.pitch);
    }
}
