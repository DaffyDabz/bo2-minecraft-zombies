//! bo2mc: Minecraft's chat, drawn like Minecraft's: the lines at the bottom
//! left on dark glass, in its font, fading ten seconds after they came; with
//! the chat open, every recent line and the box being typed in.

use asset_core::AssetRef;
use bevy::prelude::*;
use frame::MinecraftUi;

use crate::draw2d::{Draw2dCmd, Draw2dList, Draw2dOp, Draw2dProvenance, tessellate_fonts};
use crate::gpu_list::{HudTessPass, TessJob};
use crate::images::{HUD_CHROME_NAMESPACE, HudImages};

#[derive(Component)]
pub(crate) struct MinecraftChatRaster;

const ICONS: &str = "mc_item_icons";
/// The viewable screen's left and bottom edges.
const LEFT: i32 = 1;
const BOTTOM: i32 = 3;
/// Letter height, line pitch and the chat's width, in virtual pixels.
const PX: f32 = 7.0;
const PITCH: f32 = 9.0;
const WIDTH: f32 = 300.0;
/// How long a line shows with the chat shut, and the fade at its end.
const SHOW: f64 = 10.0;
const FADE: f64 = 1.0;
/// Lines shown shut and open.
const SHUT_LINES: usize = 10;
const OPEN_LINES: usize = 20;

struct Canvas<'a> {
    surface: &'a crate::surface::Hud2dSurface,
    cmds: Vec<Draw2dCmd>,
}

impl Canvas<'_> {
    fn quad(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4], material: &str, st: [f32; 4]) {
        let r = self.surface.apply_rect(x, y, w, h, LEFT, BOTTOM);
        self.cmds.push(Draw2dCmd {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
            s0: st[0],
            t0: st[1],
            s1: st[2],
            t1: st[3],
            color,
            material: material.to_owned(),
            material_namespace: HUD_CHROME_NAMESPACE,
            op: Draw2dOp::StretchPic,
            provenance: Draw2dProvenance::CgDraw { site: "minecraft_chat" },
            layer: 1,
        });
    }

    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.quad(x, y, w, h, color, "white", [0.0, 0.0, 1.0, 1.0]);
    }
}

/// A letter of Minecraft's font in the item atlas and its width in Minecraft
/// pixels (as the inventory draws them).
fn glyph(ui: &MinecraftUi, c: char) -> Option<([f32; 4], f32)> {
    ui.icon_rects.get(&format!("font:{}", c as u32)).map(|r| (*r, ((r[2] - r[0]) * 512.0 / 4.0).round()))
}

fn advance(ui: &MinecraftUi, c: char) -> f32 {
    glyph(ui, c).map_or(4.0, |(_, w)| w + 1.0) * PX / 8.0
}

/// Text with Minecraft's drop shadow; where the pen ends.
fn text(canvas: &mut Canvas<'_>, ui: &MinecraftUi, x: f32, y: f32, color: [f32; 4], line: &str) -> f32 {
    let unit = PX / 8.0;
    let mut end = x;
    for (offset, tint) in [(unit, [color[0] * 0.25, color[1] * 0.25, color[2] * 0.25, color[3]]), (0.0, color)] {
        let mut pen = x + offset;
        for c in line.chars() {
            if let Some((rect, w)) = glyph(ui, c).filter(|_| c != ' ') {
                canvas.quad(pen, y + offset, w * unit, PX, tint, ICONS, rect);
            }
            pen += advance(ui, c);
        }
        end = pen - offset;
    }
    end
}

/// A line broken into rows that fit the chat's width.
fn wrap(ui: &MinecraftUi, line: &str, width: f32) -> Vec<String> {
    let mut rows = vec![String::new()];
    let mut pen = 0.0;
    for word in line.split(' ') {
        let w: f32 = word.chars().map(|c| advance(ui, c)).sum();
        let space = advance(ui, ' ');
        let row = rows.last_mut().unwrap();
        if !row.is_empty() && pen + space + w > width {
            rows.push(word.to_owned());
            pen = w;
        } else {
            if !row.is_empty() {
                row.push(' ');
                pen += space;
            }
            row.push_str(word);
            pen += w;
        }
    }
    rows
}

pub(crate) fn update(
    surface: Res<crate::surface::Hud2dSurface>,
    ui: Option<Res<MinecraftUi>>,
    time: Res<Time>,
    mut pass: ResMut<HudTessPass>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
) {
    pass.minecraft_chat = TessJob::Hide;
    let Some(ui) = ui else {
        return;
    };
    if !ui.active || !ui.bo2mc || !surface.is_ready() || (!ui.chat_open && ui.chat_log.is_empty()) {
        return;
    }
    let now = time.elapsed_secs_f64();
    let mut canvas = Canvas { surface: &surface, cmds: Vec::new() };
    // The typing box along the bottom, over the hotbar as Minecraft's is.
    let bottom = if ui.chat_open {
        let full = surface.width() / surface.scale_virtual_to_real()[0];
        canvas.fill(2.0, -14.0, full - 4.0, 12.0, [0.0, 0.0, 0.0, 0.5]);
        let end = text(&mut canvas, &ui, 4.0, -12.0, [1.0, 1.0, 1.0, 1.0], &ui.chat_line);
        // Minecraft's blinking underscore.
        if (now * 3.0) as i64 % 2 == 0 {
            text(&mut canvas, &ui, end + PX / 8.0, -12.0, [1.0, 1.0, 1.0, 1.0], "_");
        }
        -16.0
    } else {
        -40.0
    };
    // The lines, newest at the bottom.
    let mut rows: Vec<(String, [f32; 4])> = Vec::new();
    let limit = if ui.chat_open { OPEN_LINES } else { SHUT_LINES };
    for line in ui.chat_log.iter().rev() {
        let age = now - line.at;
        let alpha = if ui.chat_open {
            1.0
        } else if age >= SHOW {
            break;
        } else {
            ((SHOW - age) / FADE).clamp(0.0, 1.0) as f32
        };
        let color = [line.color[0], line.color[1], line.color[2], alpha];
        for row in wrap(&ui, &line.text, WIDTH - 4.0).into_iter().rev() {
            rows.push((row, color));
        }
        if rows.len() >= limit {
            break;
        }
    }
    rows.truncate(limit);
    for (i, (row, color)) in rows.iter().enumerate() {
        let y = bottom - PITCH * (i as f32 + 1.0);
        canvas.fill(2.0, y, WIDTH, PITCH, [0.0, 0.0, 0.0, 0.5 * color[3]]);
        text(&mut canvas, &ui, 4.0, y + 1.0, *color, row);
    }
    if canvas.cmds.is_empty() {
        return;
    }
    let _ = hud_images.get(HUD_CHROME_NAMESPACE, AssetRef::bare_name("white"), &mut images);
    let list = Draw2dList { cmds: canvas.cmds };
    let (quads, _) = tessellate_fonts(&list, &Default::default());
    if !quads.is_empty() {
        pass.minecraft_chat = TessJob::Quads(quads);
    }
}
