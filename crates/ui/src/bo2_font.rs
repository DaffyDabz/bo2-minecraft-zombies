//! bo2zm: text in Black Ops II's own HUD fonts (`assets::T6HudFonts`): each
//! letter a piece of the font sheet, placed as the font's glyph table says
//! (its box against the pen and the baseline, then the pen moves on by the
//! letter's advance).

use std::collections::HashMap;

use bevy::prelude::*;

/// The sheet as an image and each font by the name the HUD's Lua scripts
/// give it (Default, Big, Morris, ...).
#[derive(Resource)]
pub(crate) struct Bo2Fonts {
    pub sheet: Handle<Image>,
    fonts: HashMap<String, Bo2Font>,
    /// bo2zm: the PlayStation face buttons (□ ○ △ ×), drawn here: BO2 on PC
    /// has only Xbox button pictures and its fonts have no such shapes.
    icons: HashMap<char, Handle<Image>>,
}

/// The characters drawn as button icons: the PlayStation shapes and BO2's
/// Xbox button pictures (`frame::BO2_XBOX_GLYPHS`).
pub(crate) fn is_button_icon(c: char) -> bool {
    matches!(c, '□' | '○' | '△' | '×' | '\u{E100}'..='\u{E114}')
}

/// One PlayStation face button, 64 x 64: a dark disc with a light rim and
/// the shape in its own colour (square pink, circle red, triangle green,
/// cross blue), edges smoothed.
fn button_icon(c: char) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const N: usize = 64;
    let (rgb, kind) = match c {
        '□' => ([0.95, 0.55, 0.82], 0),
        '○' => ([1.0, 0.33, 0.36], 1),
        '△' => ([0.3, 0.86, 0.72], 2),
        _ => ([0.48, 0.62, 1.0], 3),
    };
    let mut px = vec![0u8; N * N * 4];
    let edge = |d: f32| (0.5 - d).clamp(0.0, 1.0);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
            let r = (fx * fx + fy * fy).sqrt();
            // The disc and its rim.
            let disc = edge(r - 30.5);
            let rim = edge((r - 29.0).abs() - 1.5);
            // The shape's stroke (distance from its outline, 4.5 wide).
            let d = match kind {
                0 => (fx.abs().max(fy.abs()) - 13.0).abs(),
                1 => (r - 14.0).abs(),
                2 => {
                    // An upward triangle, centre slightly low.
                    let (qx, qy) = (fx, fy - 3.0);
                    let k = 3f32.sqrt();
                    let a = (k * qx.abs() - qy) * 0.5 - 8.0;
                    let b = qy - 8.0;
                    a.max(b).abs()
                }
                _ => {
                    let a = (fx - fy).abs() / 2f32.sqrt();
                    let b = (fx + fy).abs() / 2f32.sqrt();
                    let inside = fx.abs().max(fy.abs()) <= 13.0;
                    if inside { a.min(b) } else { 99.0 }
                }
            };
            let shape = edge(d - 2.25);
            let base = [0.08, 0.08, 0.09];
            let mut col = [0.0f32; 3];
            for i in 0..3 {
                let with_rim = base[i] + (0.85 - base[i]) * rim * 0.6;
                col[i] = with_rim + (rgb[i] - with_rim) * shape;
            }
            let o = (y * N + x) * 4;
            for i in 0..3 {
                px[o + i] = (col[i].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            px[o + 3] = (disc * 255.0).round() as u8;
        }
    }
    Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

struct Bo2Font {
    pixel_height: f32,
    /// Above and below the baseline, over every glyph's box.
    ascent: f32,
    descent: f32,
    glyphs: HashMap<char, Bo2Glyph>,
}

#[derive(Clone, Copy)]
struct Bo2Glyph {
    x0: f32,
    y0: f32,
    dx: f32,
    w: f32,
    h: f32,
    rect: Rect,
}

impl Bo2Fonts {
    fn font(&self, name: &str) -> Option<&Bo2Font> {
        self.fonts.get(name).or_else(|| self.fonts.get("Default"))
    }

    /// Each font's pixel height, letter advances and space advance (the
    /// LUI scripts' text measure).
    pub fn advances(&self) -> HashMap<String, (f32, HashMap<char, f32>, f32)> {
        self.fonts
            .iter()
            .map(|(name, f)| {
                let space = f.glyphs.get(&' ').map_or(f.pixel_height * 0.3, |g| g.dx);
                let mut adv: HashMap<char, f32> = f.glyphs.iter().map(|(c, g)| (*c, g.dx)).collect();
                // Button pictures: a little wider than the font is tall.
                for c in self.icons.keys() {
                    adv.insert(*c, f.pixel_height * 1.1);
                }
                (name.clone(), (f.pixel_height, adv, space))
            })
            .collect()
    }

    /// How wide `text` is in `font` at `px` (as `spawn_line` lays it out).
    pub fn line_width(&self, font: &str, text: &str, px: f32) -> f32 {
        let Some(f) = self.font(font) else {
            return 0.0;
        };
        let s = px / f.pixel_height.max(1.0);
        let space = f.glyphs.get(&' ').map_or(f.pixel_height * 0.3, |g| g.dx);
        text.chars()
            .map(|c| match f.glyphs.get(&c).or_else(|| f.glyphs.get(&'?')) {
                _ if is_button_icon(c) => f.pixel_height * 1.1,
                Some(g) if c != ' ' => g.dx,
                _ => space,
            })
            .sum::<f32>()
            * s
    }

    /// One line of coloured runs in `font`, `px` tall for the font's own
    /// pixel height, as a node of its size holding a glyph image per letter
    /// (a dark copy under each, offset by `shadow` pixels, when it is not
    /// zero). Returns the line's size.
    pub fn spawn_line(
        &self,
        parent: &mut ChildSpawnerCommands,
        font: &str,
        runs: &[(String, Color)],
        px: f32,
        shadow: f32,
    ) -> Vec2 {
        let Some(f) = self.font(font) else {
            return Vec2::ZERO;
        };
        let s = px / f.pixel_height.max(1.0);
        let space = f.glyphs.get(&' ').map_or(f.pixel_height * 0.3, |g| g.dx);
        let mut placed: Vec<(Bo2Glyph, f32, Color)> = Vec::new();
        let mut icons: Vec<(char, f32)> = Vec::new();
        let mut pen = 0.0f32;
        for (text, color) in runs {
            for c in text.chars() {
                if is_button_icon(c) && self.icons.contains_key(&c) {
                    icons.push((c, pen));
                    pen += f.pixel_height * 1.1;
                    continue;
                }
                match f.glyphs.get(&c).or_else(|| f.glyphs.get(&'?')) {
                    Some(g) if c != ' ' => {
                        placed.push((*g, pen, *color));
                        pen += g.dx;
                    }
                    _ => pen += space,
                }
            }
        }
        let size = Vec2::new(pen * s, (f.ascent + f.descent) * s);
        let sheet = self.sheet.clone();
        parent
            .spawn(Node {
                width: Val::Px(size.x),
                height: Val::Px(size.y),
                ..default()
            })
            .with_children(|line| {
                let mut glyph = |g: &Bo2Glyph, pen: f32, color: Color, off: f32| {
                    line.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px((pen + g.x0) * s + off),
                            top: Val::Px((f.ascent + g.y0) * s + off),
                            width: Val::Px(g.w * s),
                            height: Val::Px(g.h * s),
                            ..default()
                        },
                        ImageNode {
                            image: sheet.clone(),
                            rect: Some(g.rect),
                            color,
                            ..default()
                        },
                    ));
                };
                if shadow != 0.0 {
                    for (g, pen, color) in &placed {
                        let a = color.alpha() * 0.75;
                        glyph(g, *pen, Color::srgba(0.0, 0.0, 0.0, a), shadow);
                    }
                }
                for (g, pen, color) in &placed {
                    glyph(g, *pen, *color, 0.0);
                }
                // Button icons: a font's height, centred on the line.
                let side = f.pixel_height * s;
                for (c, pen) in &icons {
                    let Some(image) = self.icons.get(c) else { continue };
                    line.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(pen * s + side * 0.05),
                            top: Val::Px(((f.ascent + f.descent) * s - side) * 0.5),
                            width: Val::Px(side),
                            height: Val::Px(side),
                            ..default()
                        },
                        ImageNode::new(image.clone()),
                    ));
                }
            });
        size
    }
}

/// Once the map's fonts arrive: the sheet as an image, each font's glyphs.
pub(crate) fn load_bo2_fonts(
    mut commands: Commands,
    staged: Option<Res<assets::T6HudFonts>>,
    hud_icons: Option<Res<assets::T6HudIcons>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(staged) = staged else {
        return;
    };
    if !staged.is_changed() && !hud_icons.as_ref().is_some_and(|i| i.is_changed()) {
        return;
    }
    let Some(sheet) = staged.sheet.as_ref() else {
        commands.remove_resource::<Bo2Fonts>();
        return;
    };
    let w = sheet.texture_descriptor.size.width as f32;
    let h = sheet.texture_descriptor.size.height as f32;
    let fonts = staged
        .fonts
        .iter()
        .map(|font| {
            let glyphs: HashMap<char, Bo2Glyph> = font
                .glyphs
                .iter()
                .filter_map(|(letter, m, st)| {
                    let c = char::from_u32(u32::from(*letter))?;
                    Some((
                        c,
                        Bo2Glyph {
                            x0: f32::from(m[0]),
                            y0: f32::from(m[1]),
                            dx: f32::from(m[2]),
                            w: f32::from(m[3]),
                            h: f32::from(m[4]),
                            rect: Rect::new(st[0] * w, st[1] * h, st[2] * w, st[3] * h),
                        },
                    ))
                })
                .collect();
            let ascent = glyphs
                .values()
                .map(|g| -g.y0)
                .fold(font.pixel_height * 0.8, f32::max);
            let descent = glyphs.values().map(|g| g.y0 + g.h).fold(0.0, f32::max);
            (
                font.name.clone(),
                Bo2Font {
                    pixel_height: font.pixel_height,
                    ascent,
                    descent,
                    glyphs,
                },
            )
        })
        .collect();
    let mut icons: HashMap<char, Handle<Image>> = ['□', '○', '△', '×']
        .into_iter()
        .map(|c| (c, images.add(button_icon(c))))
        .collect();
    // BO2's own Xbox button pictures, once the HUD's pictures are in.
    if let Some(hud) = hud_icons.as_deref() {
        for (c, name) in frame::BO2_XBOX_GLYPHS {
            if let Some((_, image)) = hud.0.iter().find(|(n, _)| n == name) {
                icons.insert(c, images.add((**image).clone()));
            }
        }
    }
    commands.insert_resource(Bo2Fonts {
        sheet: images.add((**sheet).clone()),
        fonts,
        icons,
    });
}
