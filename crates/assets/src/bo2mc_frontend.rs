//! bo2mc: MINECRAFT next to NUKETOWN in Black Ops II's own map pick.
//!
//! An overlay on what the front end read from BO2's zones (never their
//! files on disk): a row for the Minecraft map in the maps table (its globe
//! spot, name and signpost), its start location (with its map card) and
//! Survival in the gametypes table, its English strings, and its pictures
//! (the signpost, the map card and the loading screen) made from
//! Minecraft's own title art (its panorama and logo, from the files fetched
//! from Mojang).
//!
//! The map's load name is `zm_minecraft`; START MATCH on it loads
//! `t6:zm_nuked` with the Minecraft world switched on (`bo2mc_switch`).
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::lane::T6Frontend;

/// The Minecraft map's load name in the front end (`ui_mapname`).
pub const MAP: &str = "zm_minecraft";
/// Its one start location (`ui_zm_mapstartlocation`).
pub const LOCATION: &str = "minecraft";
/// Nuketown, whose rows the Minecraft rows are made from.
const NUKETOWN: &str = "zm_nuked";
const NUKETOWN_LOCATION: &str = "nuked";

const MAPS_TABLE: &str = "zm/mapstable.csv";
const GAMETYPES_TABLE: &str = "zm/gametypestable.csv";

/// Where the MINECRAFT spot stands on BO2's globe (the maps table's globe
/// columns 16 and 17, in its degrees): Mojang's home, Stockholm, as
/// Nuketown's stands in Nevada.
const GLOBE_SPOT: (&str, &str) = ("-27", "55");
/// MINECRAFT's step from Nuketown's globe spot (columns 16, 17).
const NEXT_TO_NUKETOWN: (f32, f32) = (12.0, 0.0);

/// The pictures: the globe's signpost, the map card and the loading screen
/// (`loadscreen_<map>_<mode>_<location>`).
pub const SIGNPOST: &str = "menu_zm_map_signpost_minecraft";
pub const CARD: &str = "menu_zm_map_minecraft_blit_minecraft";
pub const LOADING: &str = "loadscreen_zm_minecraft_zstandard_minecraft";

/// Adds the Minecraft map to the front end. Nothing changes when
/// Nuketown's rows are not there to copy (the picker then shows Nuketown
/// alone, as before).
pub fn add_minecraft(fe: &mut T6Frontend) {
    let mut report = Vec::new();
    if add_minecraft_rows(&mut fe.ui, &mut report) {
        // Its pictures.
        let made = pictures(&fe.icons.0);
        report.push(format!(
            "{} pictures ({})",
            made.len(),
            made.iter()
                .map(|(n, i)| format!(
                    "{n} {}x{}",
                    i.texture_descriptor.size.width, i.texture_descriptor.size.height
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (name, image) in made {
            fe.icons.0.retain(|(n, _)| *n != name);
            fe.icons.0.push((name, Arc::new(image)));
        }
    }
    for line in report {
        fe.report.push(format!("bo2mc front end: {line}"));
    }
}

/// The Minecraft map's table rows and strings (no pictures): the front end
/// and a game's HUD (its scoreboard names the map: "Survival - The
/// Overworld"). False when Nuketown's rows are not there to copy.
pub fn add_minecraft_rows(ui: &mut crate::T6Ui, report: &mut Vec<String>) -> bool {
    let tables = Arc::make_mut(&mut ui.tables);
    // The maps table (columns: load name, factions, name, signpost, index,
    // description, compass, ..., globe spot 16/17, signpost side 19):
    // Nuketown's row as the Minecraft map's.
    let Some(maps) = tables.iter_mut().find(|t| t.0 == MAPS_TABLE) else {
        report.push("no maps table".to_owned());
        return false;
    };
    if find_row(maps, |r| cell(r, 0) == MAP).is_some() {
        return true;
    }
    let Some(mut row) = find_row(maps, |r| cell(r, 0) == NUKETOWN) else {
        report.push("no Nuketown row in the maps table".to_owned());
        return false;
    };
    let index = bump_count(maps, "maxnum_map");
    for (column, value) in [
        (0, MAP.to_owned()),
        (3, "ZMUI_MINECRAFT".to_owned()),
        (4, SIGNPOST.to_owned()),
        (5, index.to_string()),
        (6, "ZMUI_DESC_MAP_MINECRAFT".to_owned()),
    ] {
        set_cell(&mut row, column, value);
    }
    // His ask (10-05): MINECRAFT stands next to NUKETOWN on the globe, so
    // both picks show at once (GLOBE_SPOT, Stockholm, sat on the far side).
    for (column, step) in [(16, NEXT_TO_NUKETOWN.0), (17, NEXT_TO_NUKETOWN.1)] {
        let value = match cell(&row, column).trim().parse::<f32>() {
            Ok(nuketown) => format!("{}", nuketown + step),
            Err(_) => if column == 16 { GLOBE_SPOT.0 } else { GLOBE_SPOT.1 }.to_owned(),
        };
        set_cell(&mut row, column, value);
    }
    report.push(format!("maps row {}", row.join(",")));
    push_row(maps, row);
    // The gametypes table: Nuketown's start location (row kind 5: index,
    // map, location, name, description, card picture, ..., name 16) and
    // its Survival (kind 6: index, map, location, mode).
    if let Some(types) = tables.iter_mut().find(|t| t.0 == GAMETYPES_TABLE) {
        if let Some(mut loc) = find_row(types, |r| {
            cell(r, 0) == "5" && cell(r, 2) == NUKETOWN && cell(r, 3) == NUKETOWN_LOCATION
        }) {
            let index = bump_count(types, "maxnum_startloc");
            for (column, value) in [
                (1, index.to_string()),
                (2, MAP.to_owned()),
                (3, LOCATION.to_owned()),
                (4, "ZMUI_MINECRAFT_STARTLOC_CAPS".to_owned()),
                (5, "ZMUI_MINECRAFT_STARTLOC_DESC".to_owned()),
                (6, CARD.to_owned()),
                (16, "ZMUI_MINECRAFT_STARTLOC".to_owned()),
            ] {
                set_cell(&mut loc, column, value);
            }
            report.push(format!("start location row {}", loc.join(",")));
            push_row(types, loc);
        }
        if let Some(mut mode) = find_row(types, |r| {
            cell(r, 0) == "6"
                && cell(r, 2) == NUKETOWN
                && cell(r, 3) == NUKETOWN_LOCATION
                && cell(r, 4) == "zstandard"
        }) {
            // Its index stays Nuketown's: the menus keep "seen" bits by it
            // (a new one would carry BO2's NEW badge, a picture the
            // Zombies install does not hold).
            for (column, value) in [
                (2, MAP.to_owned()),
                (3, LOCATION.to_owned()),
            ] {
                set_cell(&mut mode, column, value);
            }
            report.push(format!("game mode row {}", mode.join(",")));
            push_row(types, mode);
        }
    }
    // Its strings, in Nuketown's style (capitals where Nuketown's are).
    let strings = Arc::make_mut(&mut ui.strings);
    let nuketown = |key: &str| {
        strings
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    };
    let as_minecraft = |text: String| {
        if text.is_empty() {
            return "MINECRAFT".to_owned();
        }
        text.replace("NUKETOWN", "MINECRAFT")
            .replace("Nuketown", "Minecraft")
            .replace("nuketown", "minecraft")
    };
    let desc = "Black Ops II zombies in a Minecraft world. Every night is a round; every day, build to survive.";
    let added = [
        // Under the map's name, where Nuketown says "Nevada, U.S.A.".
        ("ZMUI_MINECRAFT", "The Overworld".to_owned()),
        (
            "ZMUI_MINECRAFT_STARTLOC_CAPS",
            as_minecraft(nuketown("ZMUI_NUKED_STARTLOC_CAPS")),
        ),
        (
            "ZMUI_MINECRAFT_STARTLOC",
            as_minecraft(nuketown("ZMUI_NUKED_STARTLOC")),
        ),
        ("ZMUI_DESC_MAP_MINECRAFT", desc.to_owned()),
        ("ZMUI_MINECRAFT_STARTLOC_DESC", desc.to_owned()),
    ];
    for (key, text) in added {
        report.push(format!("string {key} = {text}"));
        strings.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        strings.push((key.to_owned(), text));
    }
    true
}

type StringTable = (String, usize, usize, Vec<String>);

fn cell(row: &[String], column: usize) -> &str {
    row.get(column).map_or("", String::as_str)
}

fn set_cell(row: &mut [String], column: usize, value: String) {
    if let Some(c) = row.get_mut(column) {
        *c = value;
    }
}

fn find_row(table: &StringTable, want: impl Fn(&[String]) -> bool) -> Option<Vec<String>> {
    table
        .3
        .chunks(table.1.max(1))
        .find(|r| want(r))
        .map(<[String]>::to_vec)
}

fn push_row(table: &mut StringTable, row: Vec<String>) {
    table.3.extend(row);
    table.2 += 1;
}

/// A `<name> | <count>` row (`maxnum_map | 7`): one more, and the new
/// one's index (the old count).
fn bump_count(table: &mut StringTable, name: &str) -> usize {
    let cols = table.1.max(1);
    for r in table.3.chunks_mut(cols) {
        if cell(r, 0) == name
            && let Some(n) = r.get(1).and_then(|n| n.parse::<usize>().ok())
        {
            r[1] = (n + 1).to_string();
            return n;
        }
    }
    0
}

/// The Minecraft map's pictures, from Minecraft's title art: the globe
/// signpost and the map card at Nuketown's pictures' sizes, the loading
/// screen at 1280 x 720 (BO2's), and every other picture the menus name
/// from Nuketown's load name and location (`menu_zm_nuked_map_blur`, the
/// lobby's `menu_zm_nuked_zsurvival_nuked`) as the Minecraft map's. A
/// glow is BO2's own (Nuketown's).
fn pictures(icons: &[(String, Arc<Image>)]) -> Vec<(String, Image)> {
    let size_of = |name: &str| {
        icons.iter().find(|(n, _)| n == name).map(|(_, i)| {
            let s = i.texture_descriptor.size;
            (s.width.max(1), s.height.max(1))
        })
    };
    let mut out = Vec::new();
    let mut named: Vec<(String, u32, u32)> = Vec::new();
    for (n, image) in icons {
        if !n.contains(NUKETOWN) {
            continue;
        }
        let new = n
            .replace(NUKETOWN, MAP)
            .replace(&format!("_{NUKETOWN_LOCATION}"), &format!("_{LOCATION}"));
        if n.to_ascii_lowercase().contains("glow") {
            out.push((new, (**image).clone()));
        } else if new != LOADING {
            let s = image.texture_descriptor.size;
            named.push((new, s.width.max(1), s.height.max(1)));
        }
    }
    let Some(art) = TitleArt::load() else {
        return out;
    };
    for (name, w, h) in named {
        let rgba = if name.contains("blur") {
            art.blurred(w, h)
        } else {
            art.card(w, h)
        };
        out.push((name, rgba_image(w, h, rgba)));
    }
    // The globe's signpost: Nuketown's is its emblem (the atom) on
    // nothing; the Minecraft map's is Minecraft's grass block as its
    // inventory draws it, from the block's own textures.
    let (w, h) = size_of("menu_zm_map_signpost_nuketown").unwrap_or((512, 512));
    match grass_block(w, h) {
        Some(icon) => out.push((SIGNPOST.to_owned(), rgba_image(w, h, icon))),
        None => out.push((SIGNPOST.to_owned(), rgba_image(w, h, art.card(w, h)))),
    }
    let (w, h) = size_of("menu_zm_map_nuked_blit_nuked").unwrap_or((512, 256));
    out.push((CARD.to_owned(), rgba_image(w, h, art.card(w, h))));
    out.push((LOADING.to_owned(), rgba_image(1280, 720, art.loading(1280, 720))));
    out
}

/// Minecraft's grass block as its inventory draws it (three faces, the top
/// brightest, the sides 0.8 and 0.6), filling 80% of a w x h picture's
/// height, on nothing; the top tinted plains green as Minecraft tints it.
fn grass_block(w: u32, h: u32) -> Option<Vec<u8>> {
    let top = Pixels::decode(&crate::minecraft_setup::pack_file(
        "minecraft/textures/block/grass_block_top.png",
    )?)?;
    let side = Pixels::decode(&crate::minecraft_setup::pack_file(
        "minecraft/textures/block/grass_block_side.png",
    )?)?;
    const TINT: [f32; 3] = [145.0 / 255.0, 189.0 / 255.0, 89.0 / 255.0];
    // Screen axes (y down): block +x down-right, +z down-left, +y up; the
    // cube's centre at the picture's centre.
    let s = h.min(w) as f32 * 0.4;
    let (c, k) = (0.866_025_4 * s, 0.5 * s);
    let ax = [c, k];
    let az = [-c, k];
    let ay = [0.0, -s];
    let origin = [w as f32 * 0.5, h as f32 * 0.5];
    // p = origin + a*x + b*z + d*y; per face, solve the two free axes.
    let solve = |p: [f32; 2], base: [f32; 2], e1: [f32; 2], e2: [f32; 2]| {
        let (rx, ry) = (p[0] - base[0], p[1] - base[1]);
        let det = e1[0] * e2[1] - e1[1] * e2[0];
        let a = (rx * e2[1] - ry * e2[0]) / det;
        let b = (e1[0] * ry - e1[1] * rx) / det;
        ((0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b)).then_some((a, b))
    };
    let at = |o: [f32; 2], v: [f32; 2], t: f32| [o[0] + v[0] * t, o[1] + v[1] * t];
    // Corner (0,0,0) of the cube relative to its centre.
    let corner = [
        origin[0] - 0.5 * (ax[0] + az[0] + ay[0]),
        origin[1] - 0.5 * (ax[1] + az[1] + ay[1]),
    ];
    let texel = |img: &Pixels, u: f32, v: f32| {
        let x = ((u * img.w as f32) as u32).min(img.w - 1);
        let y = ((v * img.h as f32) as u32).min(img.h - 1);
        let i = ((y * img.w + x) * 4) as usize;
        [
            f32::from(img.rgba[i]),
            f32::from(img.rgba[i + 1]),
            f32::from(img.rgba[i + 2]),
            f32::from(img.rgba[i + 3]),
        ]
    };
    let shade = |p: [f32; 2]| -> Option<[f32; 4]> {
        // Top (y = 1): along x and z.
        if let Some((x, z)) = solve(p, at(corner, ay, 1.0), ax, az) {
            let t = texel(&top, x, z);
            return Some([t[0] * TINT[0], t[1] * TINT[1], t[2] * TINT[2], t[3]]);
        }
        // Right (x = 1): along z (leftwards on screen) and y.
        if let Some((z, y)) = solve(p, at(corner, ax, 1.0), az, ay) {
            let t = texel(&side, 1.0 - z, 1.0 - y);
            return Some([t[0] * 0.6, t[1] * 0.6, t[2] * 0.6, t[3]]);
        }
        // Left (z = 1): along x and y.
        if let Some((x, y)) = solve(p, at(corner, az, 1.0), ax, ay) {
            let t = texel(&side, x, 1.0 - y);
            return Some([t[0] * 0.8, t[1] * 0.8, t[2] * 0.8, t[3]]);
        }
        None
    };
    let mut out = vec![0u8; (w * h * 4) as usize];
    const N: u32 = 4;
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 4];
            for j in 0..N {
                for i in 0..N {
                    let p = [
                        x as f32 + (i as f32 + 0.5) / N as f32,
                        y as f32 + (j as f32 + 0.5) / N as f32,
                    ];
                    if let Some(c) = shade(p) {
                        let a = c[3] / 255.0;
                        for k in 0..3 {
                            sum[k] += c[k] * a;
                        }
                        sum[3] += a;
                    }
                }
            }
            if sum[3] > 0.0 {
                let i = ((y * w + x) * 4) as usize;
                for k in 0..3 {
                    out[i + k] = (sum[k] / sum[3]).clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (sum[3] / (N * N) as f32 * 255.0) as u8;
            }
        }
    }
    Some(out)
}

fn rgba_image(w: u32, h: u32, rgba: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// An RGBA picture.
struct Pixels {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

impl Pixels {
    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().ok()?;
        let mut pixels = vec![0; reader.output_buffer_size()?];
        let frame = reader.next_frame(&mut pixels).ok()?;
        let channels = frame.color_type.samples();
        let rgba = pixels[..frame.buffer_size()]
            .chunks_exact(channels)
            .flat_map(|p| match channels {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            })
            .collect();
        Some(Self {
            w: frame.width,
            h: frame.height,
            rgba,
        })
    }

    /// Side by side (same height).
    fn beside(&self, other: &Self) -> Self {
        let (w, h) = (self.w + other.w, self.h.min(other.h));
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let a = (y * self.w * 4) as usize;
            rgba.extend_from_slice(&self.rgba[a..a + (self.w * 4) as usize]);
            let b = (y * other.w * 4) as usize;
            rgba.extend_from_slice(&other.rgba[b..b + (other.w * 4) as usize]);
        }
        Self { w, h, rgba }
    }

    /// Bilinear sample at (u, v) in 0..1.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = (u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let px = |x: u32, y: u32| {
            let i = ((y * self.w + x) * 4) as usize;
            [
                f32::from(self.rgba[i]),
                f32::from(self.rgba[i + 1]),
                f32::from(self.rgba[i + 2]),
                f32::from(self.rgba[i + 3]),
            ]
        };
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bottom - top) * fy;
        }
        out
    }

    /// Box-averaged sample over the source area one output pixel covers
    /// (a big picture made small stays smooth).
    fn area(&self, u0: f32, v0: f32, u1: f32, v1: f32) -> [f32; 4] {
        let steps_x = (((u1 - u0) * self.w as f32).ceil() as u32).clamp(1, 8);
        let steps_y = (((v1 - v0) * self.h as f32).ceil() as u32).clamp(1, 8);
        let mut sum = [0.0; 4];
        for j in 0..steps_y {
            for i in 0..steps_x {
                let u = u0 + (u1 - u0) * (i as f32 + 0.5) / steps_x as f32;
                let v = v0 + (v1 - v0) * (j as f32 + 0.5) / steps_y as f32;
                let s = self.sample(u, v);
                for k in 0..4 {
                    sum[k] += s[k];
                }
            }
        }
        let n = (steps_x * steps_y) as f32;
        sum.map(|s| s / n)
    }

    /// This picture covering a w x h one (centred crop to its shape).
    fn cover(&self, w: u32, h: u32, focus_x: f32) -> Vec<u8> {
        let src_aspect = self.w as f32 / self.h as f32;
        let dst_aspect = w as f32 / h as f32;
        let (span_u, span_v) = if src_aspect > dst_aspect {
            (dst_aspect / src_aspect, 1.0)
        } else {
            (1.0, src_aspect / dst_aspect)
        };
        let u_start = ((1.0 - span_u) * focus_x).clamp(0.0, 1.0 - span_u);
        let v_start = (1.0 - span_v) * 0.5;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let u0 = u_start + span_u * x as f32 / w as f32;
                let u1 = u_start + span_u * (x + 1) as f32 / w as f32;
                let v0 = v_start + span_v * y as f32 / h as f32;
                let v1 = v_start + span_v * (y + 1) as f32 / h as f32;
                let s = self.area(u0, v0, u1, v1);
                out.extend_from_slice(&[s[0] as u8, s[1] as u8, s[2] as u8, 255]);
            }
        }
        out
    }

    /// Draws this picture (with its alpha) over `dst` (w x h RGBA) in the
    /// rectangle at (x, y) of size (rw, rh).
    fn over(&self, dst: &mut [u8], w: u32, h: u32, rect: [f32; 4]) {
        let [rx, ry, rw, rh] = rect;
        let (x0, y0) = (rx.floor().max(0.0) as u32, ry.floor().max(0.0) as u32);
        let x1 = ((rx + rw).ceil() as u32).min(w);
        let y1 = ((ry + rh).ceil() as u32).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let u0 = (x as f32 - rx) / rw;
                let v0 = (y as f32 - ry) / rh;
                let u1 = (x as f32 + 1.0 - rx) / rw;
                let v1 = (y as f32 + 1.0 - ry) / rh;
                if u1 <= 0.0 || v1 <= 0.0 || u0 >= 1.0 || v0 >= 1.0 {
                    continue;
                }
                let s = self.area(u0.max(0.0), v0.max(0.0), u1.min(1.0), v1.min(1.0));
                let a = s[3] / 255.0;
                let i = ((y * w + x) * 4) as usize;
                for k in 0..3 {
                    dst[i + k] = (f32::from(dst[i + k]) * (1.0 - a) + s[k] * a) as u8;
                }
            }
        }
    }
}

/// Minecraft's title art: two panorama faces side by side (a wide view)
/// and the MINECRAFT logo.
struct TitleArt {
    wide: Pixels,
    logo: Pixels,
}

impl TitleArt {
    fn load() -> Option<Self> {
        let Some((faces, logo)) = crate::minecraft_setup::title_art(&[2, 3]) else {
            diag::warn!(World, "bo2mc front end: Minecraft's title art is not here (the MINECRAFT card has no picture)");
            return None;
        };
        let a = Pixels::decode(&faces[0])?;
        let b = Pixels::decode(&faces[1])?;
        let mut logo = Pixels::decode(&logo)?;
        // The logo's art fills the top of its sheet (MINECRAFT over an
        // empty lower part): keep the rows with any opaque pixel.
        let used = (0..logo.h)
            .rev()
            .find(|y| {
                (0..logo.w).any(|x| logo.rgba[((y * logo.w + x) * 4 + 3) as usize] > 8)
            })
            .map_or(logo.h, |y| y + 1);
        logo.rgba.truncate((used * logo.w * 4) as usize);
        logo.h = used;
        Some(Self {
            wide: a.beside(&b),
            logo,
        })
    }

    /// The map card: the panorama's sunny valley, the logo across it.
    fn card(&self, w: u32, h: u32) -> Vec<u8> {
        let mut out = self.wide.cover(w, h, 0.85);
        self.put_logo(&mut out, w, h, 0.8, 0.5);
        out
    }

    /// The loading screen: the wide panorama, the logo at the top as on
    /// Minecraft's title screen.
    fn loading(&self, w: u32, h: u32) -> Vec<u8> {
        let mut out = self.wide.cover(w, h, 0.5);
        self.put_logo(&mut out, w, h, 0.5, 0.18);
        out
    }

    /// The panorama blurred and darkened (the menus' backdrop behind a
    /// map, as BO2's `_map_blur` pictures are).
    fn blurred(&self, w: u32, h: u32) -> Vec<u8> {
        let (sw, sh) = (48, 27);
        let small = Pixels {
            w: sw,
            h: sh,
            rgba: self.wide.cover(sw, sh, 0.5),
        };
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let s = small.sample((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                out.extend_from_slice(&[
                    (s[0] * 0.45) as u8,
                    (s[1] * 0.45) as u8,
                    (s[2] * 0.45) as u8,
                    255,
                ]);
            }
        }
        out
    }

    /// The logo `width` of the picture wide, centred at `centre_y` (0..1).
    fn put_logo(&self, out: &mut [u8], w: u32, h: u32, width: f32, centre_y: f32) {
        let lw = w as f32 * width;
        let lh = lw * self.logo.h as f32 / self.logo.w as f32;
        let rect = [(w as f32 - lw) * 0.5, h as f32 * centre_y - lh * 0.5, lw, lh];
        self.logo.over(out, w, h, rect);
    }
}
