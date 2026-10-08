//! Oak wall signs with their text, drawn as entity geometry each frame
//! (the board of `block/template_wall_sign`, the text as `SignRenderer`
//! lays it out) so a section remesh never loses them.
//!
//! The board takes `block/oak_sign` (26.x, the wall sign's block model
//! texture), else the older `entity/signs/oak`, else oak planks. The text
//! is Minecraft's own `font/ascii` bitmap font: glyph widths are measured
//! from its pixels the way `BitmapProvider` measures them.
use crate::{
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh, Vertex},
    pack::ResourceId,
    scene::BlockPos,
};
use glam::Vec3;

/// Lines a sign holds (`SignText.LINES`).
pub const MAX_LINES: usize = 4;

/// The textures a sign draws with, for the atlas.
pub const TEXTURES: [&str; 3] = ["minecraft:block/oak_sign", "minecraft:entity/signs/oak", "minecraft:font/ascii"];

/// The way a wall sign's text faces (its blockstate's `facing`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignFacing {
    North,
    South,
    East,
    West,
}

impl SignFacing {
    /// "north", "south", "east" or "west".
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "north" => Some(Self::North),
            "south" => Some(Self::South),
            "east" => Some(Self::East),
            "west" => Some(Self::West),
            _ => None,
        }
    }

    /// The text side's outward direction in blocks.
    fn normal(self) -> Vec3 {
        match self {
            Self::North => Vec3::NEG_Z,
            Self::South => Vec3::Z,
            Self::East => Vec3::X,
            Self::West => Vec3::NEG_X,
        }
    }
}

/// A wall sign in the cell `pos`, on the wall behind it (the block at
/// `pos` minus `facing`).
#[derive(Clone, Debug, PartialEq)]
pub struct WallSign {
    pub pos: BlockPos,
    pub facing: SignFacing,
    pub lines: Vec<String>,
}

/// `template_wall_sign`'s board, in blocks from the wall: 16 x 8 x 4/3
/// pixels, 4 1/3 pixels off the floor of its cell.
const BOARD_BOTTOM: f32 = 4.333_333 / 16.0;
const BOARD_TOP: f32 = 12.333_333 / 16.0;
const BOARD_BACK: f32 = 0.333_333 / 16.0;
const BOARD_FRONT: f32 = 1.666_667 / 16.0;
/// `SignRenderer`: the text's origin (TEXT_OFFSET after the wall
/// translate) and its scale, blocks per font pixel.
const TEXT_CENTER: f32 = 0.5 - 0.3125 + 0.333_333_34;
const TEXT_DEPTH: f32 = 0.5 - 0.4375 + 0.046_666_667;
const FONT_PIXEL: f32 = 0.015_625 * 0.666_666_7;
/// `SignBlockEntity`: line height and widest line, font pixels.
const LINE_HEIGHT: f32 = 10.0;
const MAX_LINE_WIDTH: f32 = 90.0;
/// Black dye's text colour on a sign that does not glow.
const TEXT_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// The space provider's advance.
const SPACE_ADVANCE: f32 = 4.0;

/// Board face rectangles in texture pixels (x, y, w, h) and the image size:
/// front, back, top, bottom, left side, right side (as seen from the front).
struct BoardSkin {
    region: [f32; 4],
    size: [f32; 2],
    faces: [[f32; 4]; 6],
}

fn board_skin(atlas: &Atlas) -> BoardSkin {
    let usable = |id: &str| {
        let id = ResourceId::parse(id).ok()?;
        (atlas.contains(&id) && !atlas.missing.contains(&id)).then(|| atlas.entity_region(&id))
    };
    if let Some(region) = usable("minecraft:block/oak_sign") {
        // `template_wall_sign`'s face uvs, doubled for the 32-pixel texture.
        BoardSkin {
            region,
            size: [32.0, 32.0],
            faces: [
                [0.0, 2.0, 24.0, 12.0],
                [0.0, 16.0, 24.0, 12.0],
                [0.0, 0.0, 24.0, 2.0],
                [0.0, 28.0, 24.0, 2.0],
                [24.0, 16.0, 2.0, 12.0],
                [24.0, 2.0, 2.0, 12.0],
            ],
        }
    } else if let Some(region) = usable("minecraft:entity/signs/oak") {
        // `SignRenderer.createSignLayer`'s 24 x 12 x 2 box at (0, 0).
        BoardSkin {
            region,
            size: [64.0, 32.0],
            faces: [
                [2.0, 2.0, 24.0, 12.0],
                [28.0, 2.0, 24.0, 12.0],
                [2.0, 0.0, 24.0, 2.0],
                [26.0, 0.0, 24.0, 2.0],
                [0.0, 2.0, 2.0, 12.0],
                [26.0, 2.0, 2.0, 12.0],
            ],
        }
    } else {
        let region = atlas.region(&ResourceId::parse("minecraft:block/oak_planks").unwrap());
        BoardSkin {
            region,
            size: [16.0, 16.0],
            faces: [
                [0.0, 4.0, 16.0, 8.0],
                [0.0, 4.0, 16.0, 8.0],
                [0.0, 0.0, 16.0, 1.0],
                [0.0, 15.0, 16.0, 1.0],
                [0.0, 4.0, 1.0, 8.0],
                [15.0, 4.0, 1.0, 8.0],
            ],
        }
    }
}

/// The ascii font in the atlas: its region, the glyph cell in atlas pixels
/// and each character's width in font pixels.
struct Font {
    region: [f32; 4],
    widths: [u8; 128],
}

fn font(atlas: &Atlas) -> Option<Font> {
    let id = ResourceId::parse("minecraft:font/ascii").ok()?;
    if !atlas.contains(&id) || atlas.missing.contains(&id) {
        return None;
    }
    let region = atlas.entity_region(&id);
    let (w, h) = (atlas.pixels.width() as f32, atlas.pixels.height() as f32);
    let x0 = (region[0] * w).round() as u32;
    let y0 = (region[1] * h).round() as u32;
    let cell = (((region[2] - region[0]) * w).round() as u32 / 16).max(1);
    let mut widths = [0u8; 128];
    for (code, width) in widths.iter_mut().enumerate() {
        let (col, row) = (code as u32 % 16, code as u32 / 16);
        // `BitmapProvider`: the rightmost column with any pixel, plus one.
        let filled = (0..cell).rev().find(|&x| {
            (0..cell).any(|y| {
                let (px, py) = (x0 + col * cell + x, y0 + row * cell + y);
                px < atlas.pixels.width() && py < atlas.pixels.height() && atlas.pixels.get_pixel(px, py)[3] > 0
            })
        });
        *width = filled.map_or(0, |x| (((x + 1) * 8).div_ceil(cell)) as u8);
    }
    Some(Font { region, widths })
}

impl Font {
    fn glyph(c: char) -> u8 {
        if (' '..='~').contains(&c) { c as u8 } else { b'?' }
    }

    fn advance(&self, code: u8) -> f32 {
        if code == b' ' { SPACE_ADVANCE } else { self.widths[code as usize] as f32 + 1.0 }
    }

    fn width(&self, line: &str) -> f32 {
        line.chars().map(|c| self.advance(Self::glyph(c))).sum()
    }
}

fn push_quad(mesh: &mut ChunkMesh, corners: [Vec3; 4], uv: [[f32; 2]; 4], color: [f32; 4], light: [f32; 2]) {
    let start = mesh.vertices.len() as u32;
    for (corner, uv) in corners.into_iter().zip(uv) {
        mesh.vertices.push(Vertex { position: corner.to_array(), uv, color, sky_light: light[0], block_light: light[1] });
    }
    mesh.indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    mesh.faces += 1;
}

/// Atlas uvs of a texture-pixel rectangle, corners in `push_quad` order
/// (top-left, bottom-left, bottom-right, top-right).
fn rect_uv(region: [f32; 4], size: [f32; 2], [x, y, w, h]: [f32; 4]) -> [[f32; 2]; 4] {
    let u = |px: f32| region[0] + (region[2] - region[0]) * px / size[0];
    let v = |py: f32| region[1] + (region[3] - region[1]) * py / size[1];
    [[u(x), v(y)], [u(x), v(y + h)], [u(x + w), v(y + h)], [u(x + w), v(y)]]
}

/// Appends each sign's board and text to `mesh` (a cut-out entity mesh),
/// lit by the light in its own cell.
pub fn append_wall_signs<'a>(mesh: &mut ChunkMesh, signs: impl IntoIterator<Item = &'a WallSign>, atlas: &Atlas, light: &SkyLight) {
    let mut signs = signs.into_iter().peekable();
    if signs.peek().is_none() {
        return;
    }
    let skin = board_skin(atlas);
    let font = font(atlas);
    for sign in signs {
        let (x, y, z) = sign.pos;
        let lit = [light.get(sign.pos) as f32, light.get_block(sign.pos) as f32];
        let n = sign.facing.normal();
        // The viewer's right, looking at the text.
        let r = Vec3::new(n.z, 0.0, -n.x);
        let wall = Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5) - n * 0.5;
        // A point `s` right of centre, `h` up from the cell's floor, `d`
        // out from the wall.
        let at = |s: f32, h: f32, d: f32| wall + r * s + Vec3::Y * h + n * d;
        let side_shade = if n.x != 0.0 { 0.8 } else { 0.6 };
        let face_shade = if n.x != 0.0 { 0.6 } else { 0.8 };
        let shaded = |shade: f32| [shade, shade, shade, 1.0];
        let (b, t, back, front) = (BOARD_BOTTOM, BOARD_TOP, BOARD_BACK, BOARD_FRONT);
        let boxes: [([Vec3; 4], f32); 6] = [
            ([at(-0.5, t, front), at(-0.5, b, front), at(0.5, b, front), at(0.5, t, front)], face_shade),
            ([at(0.5, t, back), at(0.5, b, back), at(-0.5, b, back), at(-0.5, t, back)], face_shade),
            ([at(-0.5, t, back), at(-0.5, t, front), at(0.5, t, front), at(0.5, t, back)], 1.0),
            ([at(-0.5, b, front), at(-0.5, b, back), at(0.5, b, back), at(0.5, b, front)], 0.5),
            ([at(-0.5, t, back), at(-0.5, b, back), at(-0.5, b, front), at(-0.5, t, front)], side_shade),
            ([at(0.5, t, front), at(0.5, b, front), at(0.5, b, back), at(0.5, t, back)], side_shade),
        ];
        for ((corners, shade), rect) in boxes.into_iter().zip(skin.faces) {
            push_quad(mesh, corners, rect_uv(skin.region, skin.size, rect), shaded(shade), lit);
        }
        let Some(font) = &font else { continue };
        for (i, line) in sign.lines.iter().take(MAX_LINES).enumerate() {
            let width = font.width(line);
            if width <= 0.0 {
                continue;
            }
            // Too wide a line is shrunk to fit the board (vanilla cuts it).
            let k = (MAX_LINE_WIDTH / width).min(1.0);
            let top = i as f32 * LINE_HEIGHT - 2.0 * LINE_HEIGHT;
            let mid = top + 4.0;
            // Font pixels (right, down from the text origin) to the wall.
            let px = |fx: f32, fy: f32| at(fx * k * FONT_PIXEL, TEXT_CENTER - (mid + (fy - mid) * k) * FONT_PIXEL, TEXT_DEPTH);
            let mut cx = -width / 2.0;
            for c in line.chars() {
                let code = Font::glyph(c);
                let glyph_w = font.widths[code as usize] as f32;
                if code != b' ' && glyph_w > 0.0 {
                    let (col, row) = ((code % 16) as f32, (code / 16) as f32);
                    let uv = rect_uv(font.region, [128.0, 128.0], [col * 8.0, row * 8.0, glyph_w, 8.0]);
                    let corners = [px(cx, top), px(cx, top + 8.0), px(cx + glyph_w, top + 8.0), px(cx + glyph_w, top)];
                    push_quad(mesh, corners, uv, TEXT_COLOR, lit);
                }
                cx += font.advance(code);
            }
        }
    }
}
