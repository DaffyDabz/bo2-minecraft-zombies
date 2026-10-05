//! bo2zm M3: the Black Ops II Zombies HUD, drawn over the game while a BO2
//! map runs (its server sends `bo2zm_round`): the round bottom-left in
//! red, the player's points and ammo bottom-right, and in the middle the
//! hint of the use trigger he faces ("Hold [F] for M14 [Cost: 500]", the
//! game's own English text with its colour codes).

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::{ClientSet, HudInputView};
use net::{LocalPresentClient, PresentedSnapshot};

use crate::layers::{GameUiFont, UiLayer, UiLayerVisibility, game_text_font};

#[derive(Component)]
struct ZmHudRoot;

#[derive(Component)]
struct ZmRound;

#[derive(Component)]
struct ZmPoints;

#[derive(Component)]
struct ZmAmmo;

#[derive(Component)]
struct ZmHint;

/// bo2zm: a line drawn in Black Ops II's own font (0 round, 1 points,
/// 2 ammo, 3 hint), beside the stand-in text it replaces.
#[derive(Component)]
struct ZmLine(u8);

/// The values on screen.
#[derive(Clone, Debug, Default, PartialEq)]
struct ZmHudValues {
    round: String,
    points: i32,
    clip: i32,
    /// A dual-wield weapon's left clip.
    clip_left: Option<i32>,
    stock: i32,
    hint: String,
    /// His grenades and mines (`name:count` by `;`).
    offhand: String,
}

/// A clip from the player's clip table (rows of 12 bytes: the weapon, the
/// right hand's count, the left hand's).
fn clip_of(table: &[u8], weapon: i32, hand: usize) -> i32 {
    table
        .chunks_exact(12)
        .find(|row| i32::from_le_bytes([row[0], row[1], row[2], row[3]]) == weapon)
        .map_or(0, |row| {
            let o = 4 + hand.min(1) * 4;
            i32::from_le_bytes([row[o], row[o + 1], row[o + 2], row[o + 3]])
        })
}

const ROUND_RED: Color = Color::srgb(0.62, 0.04, 0.02);
const POINTS_WHITE: Color = Color::srgb(0.95, 0.93, 0.85);

fn shadow() -> TextShadow {
    TextShadow {
        offset: Vec2::splat(2.0),
        color: Color::linear_rgba(0.0, 0.0, 0.0, 0.8),
        ..default()
    }
}

/// A BO2 colour code (`^1`..`^9`, `^0`).
fn code_color(c: char) -> Option<Color> {
    Some(match c {
        '0' => Color::srgb(0.0, 0.0, 0.0),
        '1' => Color::srgb(1.0, 0.27, 0.27),
        '2' => Color::srgb(0.4, 1.0, 0.4),
        '3' => Color::srgb(1.0, 0.9, 0.2),
        '4' => Color::srgb(0.35, 0.5, 1.0),
        '5' => Color::srgb(0.3, 0.9, 1.0),
        '6' => Color::srgb(1.0, 0.45, 0.9),
        '7' => Color::srgb(1.0, 1.0, 1.0),
        '8' | '9' => Color::srgb(0.7, 0.7, 0.7),
        _ => return None,
    })
}

/// Split a hint into coloured runs; `[{+activate}]`-style key names become
/// the player's own key ("[F]").
fn hint_runs(text: &str, keys: &HudInputView) -> Vec<(String, Color)> {
    let mut text = text.to_owned();
    while let Some(start) = text.find("[{") {
        let Some(len) = text[start..].find("}]") else {
            break;
        };
        let command = &text[start + 2..start + len];
        let key = keys
            .binding_keys
            .get(command)
            .cloned()
            .or_else(|| {
                (command == "+activate" || command == "+usereload")
                    .then(|| keys.use_key.clone())
                    .flatten()
            })
            .unwrap_or_else(|| {
                if command == "+activate" {
                    "F".to_owned()
                } else {
                    command.trim_start_matches('+').to_uppercase()
                }
            });
        // A pad button shows as its icon, without brackets (as BO2 shows its
        // pad pictures).
        let shown = if key.chars().count() == 1 && key.chars().all(crate::bo2_font::is_button_icon) {
            key
        } else {
            format!("[{key}]")
        };
        text.replace_range(start..start + len + 2, &shown);
    }
    let mut runs: Vec<(String, Color)> = Vec::new();
    let mut color = Color::WHITE;
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '^'
            && let Some(&next) = chars.peek()
            && let Some(c) = code_color(next)
        {
            chars.next();
            if !cur.is_empty() {
                runs.push((std::mem::take(&mut cur), color));
            }
            color = c;
            continue;
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        runs.push((cur, color));
    }
    runs
}

fn read_values(presented: &PresentedSnapshot, local: &LocalPresentClient) -> Option<ZmHudValues> {
    let snap = presented.snapshot()?;
    let dvars = snap.meta.script_dvars(local.0);
    // The scripts hid his HUD (the game-over shot).
    if dvars.string("bo2zm_hud_hidden") == Some("1") {
        return None;
    }
    let round = dvars.string("bo2zm_round")?.to_owned();
    let meta = snap.meta.for_client(local.0)?;
    Some(ZmHudValues {
        round,
        points: meta.score,
        clip: meta.ammo_clip,
        // Both guns held (Black Ops II dual wield): the left one's clip.
        clip_left: presented
            .player(local.0)
            .filter(|ps| ps.last_weapon_hand == 1)
            .map(|ps| clip_of(&ps.ammoclip, ps.weapon as i32, 1)),
        stock: meta.ammo_stock,
        hint: dvars.string("bo2zm_hint").unwrap_or_default().to_owned(),
        offhand: dvars.string("bo2zm_offhand").unwrap_or_default().to_owned(),
    })
}

/// The HUD icon of a grenade or mine he carries.
fn offhand_icon(weapon: &str) -> Option<&'static str> {
    Some(match weapon {
        "frag_grenade_zm" => "hud_us_grenade",
        "sticky_grenade_zm" => "hud_icon_sticky_grenade",
        "cymbal_monkey_zm" => "hud_cymbal_monkey",
        "claymore_zm" => "hud_icon_claymore",
        _ => return None,
    })
}

/// (icon, count) for each grenade or mine in `bo2zm_offhand`.
fn offhand_rows(value: &str) -> Vec<(&'static str, u32)> {
    value
        .split(';')
        .filter_map(|e| {
            let (w, n) = e.split_once(':')?;
            Some((offhand_icon(w)?, n.parse().unwrap_or(0)))
        })
        .filter(|(_, n)| *n > 0)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn zm_hud(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    keys: Option<Res<HudInputView>>,
    font: Option<Res<GameUiFont>>,
    window: Query<&Window, With<PrimaryWindow>>,
    roots: Query<Entity, With<ZmHudRoot>>,
    mut texts: ParamSet<(
        Query<(&mut Text, &mut TextFont), With<ZmRound>>,
        Query<(&mut Text, &mut TextFont), With<ZmPoints>>,
        Query<(&mut Text, &mut TextFont), With<ZmAmmo>>,
    )>,
    hint: Query<Entity, With<ZmHint>>,
    mut shown: Local<Option<ZmHudValues>>,
    mut scale_shown: Local<f32>,
    (bo2, icons, mut images, mut chalk, lines, mut bo2_shown, mut offhand_imgs): (
        Option<Res<crate::bo2_font::Bo2Fonts>>,
        Option<Res<assets::T6HudIcons>>,
        ResMut<Assets<Image>>,
        Local<Vec<Handle<Image>>>,
        Query<(Entity, &ZmLine)>,
        Local<bool>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
) {
    let values = match (presented.as_deref(), local.as_deref()) {
        (Some(p), Some(l)) => read_values(p, l),
        _ => None,
    };
    // The round's chalk marks (rounds 1 to 5), once.
    if chalk.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        let found: Vec<Handle<Image>> = (1..=5)
            .filter_map(|n| {
                let name = format!("hud_chalk_{n}");
                icons
                    .0
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, img)| images.add((**img).clone()))
            })
            .collect();
        if found.len() == 5 {
            *chalk = found;
        }
    }
    // The grenade and mine icons, once.
    if offhand_imgs.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        for w in [
            "frag_grenade_zm",
            "sticky_grenade_zm",
            "cymbal_monkey_zm",
            "claymore_zm",
        ] {
            let name = offhand_icon(w).unwrap_or_default();
            if let Some((_, img)) = icons.0.iter().find(|(k, _)| *k == name) {
                offhand_imgs.push((name.to_owned(), images.add((**img).clone())));
            }
        }
    }
    let Some(values) = values else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *shown = None;
        return;
    };
    let Some(font) = font else { return };
    let scale = window
        .single()
        .map_or(1.0, |w| (w.height() / 1080.0).max(0.4));
    if roots.is_empty() {
        let f = |px: f32| game_text_font(&font.0, px * scale);
        commands
            .spawn((
                ZmHudRoot,
                UiLayer::Hud,
                UiLayerVisibility,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
            ))
            .with_children(|p| {
                p.spawn((
                    ZmRound,
                    Text::new(""),
                    f(84.0),
                    TextColor(ROUND_RED),
                    shadow(),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(3.5),
                        bottom: Val::Percent(4.0),
                        ..default()
                    },
                ));
                p.spawn((
                    ZmPoints,
                    Text::new(""),
                    f(34.0),
                    TextColor(POINTS_WHITE),
                    shadow(),
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Percent(3.5),
                        bottom: Val::Percent(14.0),
                        ..default()
                    },
                ));
                p.spawn((
                    ZmAmmo,
                    Text::new(""),
                    f(30.0),
                    TextColor(Color::WHITE),
                    shadow(),
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Percent(3.5),
                        bottom: Val::Percent(6.0),
                        ..default()
                    },
                ));
                p.spawn((
                    ZmHint,
                    Text::new(""),
                    f(24.0),
                    TextColor(Color::WHITE),
                    shadow(),
                    TextLayout::justify(Justify::Center),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        top: Val::Percent(62.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                ));
                // The same four in BO2's own font (filled when it loaded).
                let at = |left: Option<f32>, right: Option<f32>, bottom: Option<f32>| Node {
                    position_type: PositionType::Absolute,
                    left: left.map_or(Val::Auto, Val::Percent),
                    right: right.map_or(Val::Auto, Val::Percent),
                    bottom: bottom.map_or(Val::Auto, Val::Percent),
                    align_items: AlignItems::FlexEnd,
                    ..default()
                };
                p.spawn((ZmLine(0), at(Some(3.5), None, Some(4.0))));
                p.spawn((ZmLine(1), at(None, Some(3.5), Some(14.0))));
                p.spawn((ZmLine(2), at(None, Some(3.5), Some(6.0))));
                p.spawn((
                    ZmLine(3),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        top: Val::Percent(62.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                ));
            });
        *shown = None;
        return;
    }
    let bo2_now = bo2.is_some();
    if shown.as_ref() == Some(&values)
        && (*scale_shown - scale).abs() < 1e-3
        && *bo2_shown == bo2_now
    {
        return;
    }
    let rescale = (*scale_shown - scale).abs() >= 1e-3 || *bo2_shown != bo2_now;
    *scale_shown = scale;
    *bo2_shown = bo2_now;
    // bo2zm: Black Ops II's own fonts (`ui/t6/codbase.lua`'s names): the
    // round in Morris (rounds 1 to 5 as the chalk marks), the points in
    // Default, the ammo in Big, the hint in Default.
    if let Some(bo2) = bo2.as_deref() {
        for (mut t, _) in &mut texts.p0() {
            *t = Text::new("");
        }
        for (mut t, _) in &mut texts.p1() {
            *t = Text::new("");
        }
        for (mut t, _) in &mut texts.p2() {
            *t = Text::new("");
        }
        for e in &hint {
            commands.entity(e).despawn_related::<Children>();
            commands
                .entity(e)
                .with_child((TextSpan::new(""), game_text_font(&font.0, 24.0 * scale)));
        }
        let keys = keys.as_deref().cloned().unwrap_or_default();
        let ammo = match values.clip_left {
            Some(left) => format!("{left} | {} / {}", values.clip, values.stock),
            None => format!("{} / {}", values.clip, values.stock),
        };
        let round: u32 = values.round.parse().unwrap_or(0);
        // bo2zm M4: BO2's own HUD (unless BO2ZM_LUI=0) draws the round, the points
        // and the ammo.
        let lui = crate::lui_hud::lui_enabled();
        for (e, line) in &lines {
            commands.entity(e).despawn_related::<Children>();
            match line.0 {
                0 | 1 if lui => {}
                0 if (1..=5).contains(&round) && chalk.len() == 5 => {
                    let image = chalk[round as usize - 1].clone();
                    commands.entity(e).with_child((
                        ImageNode {
                            image,
                            color: ROUND_RED,
                            ..default()
                        },
                        Node {
                            width: Val::Px(150.0 * scale),
                            height: Val::Px(150.0 * scale),
                            ..default()
                        },
                    ));
                }
                0 => {
                    let text = values.round.clone();
                    commands.entity(e).with_children(|c| {
                        bo2.spawn_line(
                            c,
                            "Morris",
                            &[(text, ROUND_RED)],
                            110.0 * scale,
                            3.0 * scale,
                        );
                    });
                }
                1 => {
                    let text = values.points.to_string();
                    commands.entity(e).with_children(|c| {
                        bo2.spawn_line(
                            c,
                            "Default",
                            &[(text, POINTS_WHITE)],
                            46.0 * scale,
                            2.0 * scale,
                        );
                    });
                }
                2 => {
                    let text = ammo.clone();
                    // BO2's offhand icons left of the ammo: one per grenade,
                    // each one fainter (offhandicons.lua), a group per kind.
                    let rows: Vec<(Handle<Image>, u32)> = offhand_rows(if lui { "" } else { &values.offhand })
                        .into_iter()
                        .filter_map(|(icon, n)| {
                            offhand_imgs
                                .iter()
                                .find(|(k, _)| k == icon)
                                .map(|(_, h)| (h.clone(), n.min(4)))
                        })
                        .collect();
                    commands.entity(e).with_children(|c| {
                        for (h, n) in rows {
                            c.spawn(Node {
                                margin: UiRect::right(Val::Px(16.0 * scale)),
                                column_gap: Val::Px(2.0 * scale),
                                align_self: AlignSelf::Center,
                                ..default()
                            })
                            .with_children(|g| {
                                for i in 0..n {
                                    let size = Val::Px(34.0 * scale);
                                    g.spawn((
                                        ImageNode::new(h.clone()).with_color(Color::srgba(
                                            1.0,
                                            1.0,
                                            1.0,
                                            0.75f32.powi(i as i32),
                                        )),
                                        Node {
                                            width: size,
                                            height: size,
                                            ..default()
                                        },
                                    ));
                                }
                            });
                        }
                        if !lui {
                            bo2.spawn_line(
                                c,
                                "Big",
                                &[(text, Color::WHITE)],
                                44.0 * scale,
                                2.0 * scale,
                            );
                        }
                    });
                }
                _ => {
                    let runs = hint_runs(&values.hint, &keys);
                    commands.entity(e).with_children(|c| {
                        bo2.spawn_line(c, "Default", &runs, 40.0 * scale, 2.0 * scale);
                    });
                }
            }
        }
        *shown = Some(values);
        return;
    }
    for (e, _) in &lines {
        commands.entity(e).despawn_related::<Children>();
    }
    for (mut t, mut tf) in &mut texts.p0() {
        *t = Text::new(values.round.clone());
        if rescale {
            *tf = game_text_font(&font.0, 84.0 * scale);
        }
    }
    for (mut t, mut tf) in &mut texts.p1() {
        *t = Text::new(values.points.to_string());
        if rescale {
            *tf = game_text_font(&font.0, 34.0 * scale);
        }
    }
    for (mut t, mut tf) in &mut texts.p2() {
        *t = Text::new(match values.clip_left {
            Some(left) => format!("{left} | {} / {}", values.clip, values.stock),
            None => format!("{} / {}", values.clip, values.stock),
        });
        if rescale {
            *tf = game_text_font(&font.0, 30.0 * scale);
        }
    }
    let hint_changed = shown.as_ref().is_none_or(|s| s.hint != values.hint) || rescale;
    if hint_changed && std::env::var_os("IW4L_T6_HINTLOG").is_some() {
        diag::info!(World, "bo2zm hud hint shown: {:?}", values.hint);
    }
    if hint_changed {
        let keys = keys.as_deref().cloned().unwrap_or_default();
        for e in &hint {
            commands.entity(e).despawn_related::<Children>();
            let mut runs = hint_runs(&values.hint, &keys);
            // An empty hint still gets one (empty) span: with every span
            // removed and none added the text is not laid out again and the
            // old hint stays on screen (seen: the box's hint after it left).
            if runs.is_empty() {
                runs.push((String::new(), Color::WHITE));
            }
            for (run, color) in runs {
                commands.entity(e).with_child((
                    TextSpan::new(run),
                    game_text_font(&font.0, 24.0 * scale),
                    TextColor(color),
                ));
            }
        }
    }
    *shown = Some(values);
}

#[derive(Component)]
struct ZmScriptHudRoot;

/// One script HUD element as the server sends it (`bo2zm_hud`).
#[derive(Clone, Debug, PartialEq)]
struct ScriptElem {
    x: f32,
    y: f32,
    alignx: String,
    aligny: String,
    horzalign: String,
    vertalign: String,
    fontscale: f32,
    color: [f32; 4],
    sort: i32,
    text: String,
}

fn parse_script_hud(s: &str) -> Vec<ScriptElem> {
    let mut out: Vec<ScriptElem> = s
        .split('\u{1e}')
        .filter_map(|row| {
            let f: Vec<&str> = row.split('\u{1f}').collect();
            if f.len() < 13 {
                return None;
            }
            let n = |i: usize| f[i].parse::<f32>().unwrap_or(0.0);
            Some(ScriptElem {
                x: n(0),
                y: n(1),
                alignx: f[2].to_owned(),
                aligny: f[3].to_owned(),
                horzalign: f[4].to_owned(),
                vertalign: f[5].to_owned(),
                fontscale: n(6),
                color: [n(7), n(8), n(9), n(10)],
                sort: f[11].parse().unwrap_or(0),
                text: f[12..].join(" "),
            })
        })
        .collect();
    out.sort_by_key(|e| e.sort);
    out
}

/// The scripts' own HUD elements (Game Over, "you survived N rounds", Max
/// Ammo...) where BO2 puts them: a 640x480 screen, x/y from the element's
/// horizontal/vertical alignment base, the text anchored by alignx/aligny.
#[allow(clippy::too_many_arguments)]
fn zm_script_hud(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    keys: Option<Res<HudInputView>>,
    font: Option<Res<GameUiFont>>,
    window: Query<&Window, With<PrimaryWindow>>,
    roots: Query<Entity, With<ZmScriptHudRoot>>,
    mut shown: Local<(String, f32, f32, bool)>,
    bo2: Option<Res<crate::bo2_font::Bo2Fonts>>,
    (icons, mut images, mut pictures): (
        Option<Res<assets::T6HudIcons>>,
        ResMut<Assets<Image>>,
        Local<Vec<(String, Handle<Image>)>>,
    ),
) {
    // The game's own pictures the scripts put up (the red low-health
    // overlay), once.
    if pictures.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        for (name, image) in icons.0.iter().filter(|(n, _)| n.starts_with("overlay_")) {
            pictures.push((name.clone(), images.add((**image).clone())));
        }
    }
    let value = match (presented.as_deref(), local.as_deref()) {
        (Some(p), Some(l)) => p.snapshot().and_then(|snap| {
            snap.meta
                .script_dvars(l.0)
                .string("bo2zm_hud")
                .map(str::to_owned)
        }),
        _ => None,
    };
    let Some(value) = value else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *shown = (String::new(), 0.0, 0.0, false);
        return;
    };
    let Some(font) = font else { return };
    let (sw, sh) = window
        .single()
        .map_or((1920.0, 1080.0), |w| (w.width(), w.height()));
    if !roots.is_empty()
        && shown.0 == value
        && shown.1 == sw
        && shown.2 == sh
        && shown.3 == bo2.is_some()
    {
        return;
    }
    *shown = (value.clone(), sw, sh, bo2.is_some());
    for e in &roots {
        commands.entity(e).despawn();
    }
    let (kx, ky) = (sw / 640.0, sh / 480.0);
    let keys = keys.as_deref().cloned().unwrap_or_default();
    let elems = parse_script_hud(&value);
    commands
        .spawn((
            ZmScriptHudRoot,
            UiLayer::Hud,
            UiLayerVisibility,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ))
        .with_children(|p| {
            for e in elems {
                let base_x = match e.horzalign.as_str() {
                    "center" | "user_center" | "center_safearea" => 320.0,
                    "right" | "user_right" | "right_adjustable" => 640.0,
                    _ => 0.0,
                };
                let base_y = match e.vertalign.as_str() {
                    "middle" | "user_middle" | "center" | "center_safearea" => 240.0,
                    "bottom" | "user_bottom" | "bottom_adjustable" => 480.0,
                    _ => 0.0,
                };
                let px = 18.0 * e.fontscale.max(0.1) * ky;
                let x = (base_x + e.x) * kx;
                let y = (base_y + e.y) * ky;
                // A wide box the text is justified in, so alignx anchors it.
                let w = sw;
                let (left, justify) = match e.alignx.as_str() {
                    "center" => (x - w * 0.5, Justify::Center),
                    "right" => (x - w, Justify::Right),
                    _ => (x, Justify::Left),
                };
                let top = match e.aligny.as_str() {
                    "middle" => y - px * 0.6,
                    "bottom" => y - px * 1.2,
                    _ => y,
                };
                let color = Color::srgba(e.color[0], e.color[1], e.color[2], e.color[3]);
                // A picture: BO2's plain black/white fills (the game-over
                // fade); full screen when aligned to the full screen.
                if let Some(rest) = e.text.strip_prefix("#shader:") {
                    let mut it = rest.split(':');
                    let name = it.next().unwrap_or_default();
                    let w: f32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let h: f32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    // bo2zm M3 fix list 2: or one of the game's own pictures:
                    // the red overlay the health script fades in when he is
                    // hit ("my screen should get red"); it was skipped.
                    let fill = match name {
                        "black" => Some(Color::srgba(0.0, 0.0, 0.0, e.color[3])),
                        "white" => Some(color),
                        _ => None,
                    };
                    let picture = pictures
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, h)| h.clone());
                    if fill.is_none() && picture.is_none() {
                        continue;
                    }
                    let full = e.horzalign == "fullscreen" && e.vertalign == "fullscreen";
                    let (left, top, width, height) = if full {
                        (0.0, 0.0, sw, sh)
                    } else {
                        (x, y, w * kx, h * ky)
                    };
                    let node = Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(left),
                        top: Val::Px(top),
                        width: Val::Px(width),
                        height: Val::Px(height),
                        ..default()
                    };
                    match (fill, picture) {
                        (Some(fill), _) => {
                            p.spawn((BackgroundColor(fill), ZIndex(e.sort - 1000), node));
                        }
                        (None, Some(h)) => {
                            p.spawn((
                                ImageNode::new(h)
                                    .with_color(color)
                                    .with_mode(bevy::ui::widget::NodeImageMode::Stretch),
                                ZIndex(e.sort - 1000),
                                node,
                            ));
                        }
                        (None, None) => {}
                    }
                    continue;
                }
                // bo2zm: in BO2's own font when it loaded.
                if let Some(bo2) = bo2.as_deref() {
                    let runs: Vec<(String, Color)> = hint_runs(&e.text, &keys)
                        .into_iter()
                        .map(|(run, c)| {
                            let c = if c == Color::WHITE {
                                color
                            } else {
                                c.with_alpha(e.color[3])
                            };
                            (run, c)
                        })
                        .collect();
                    p.spawn((
                        ZIndex(e.sort),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(left),
                            top: Val::Px(top),
                            width: Val::Px(w),
                            justify_content: match justify {
                                Justify::Center => JustifyContent::Center,
                                Justify::Right => JustifyContent::FlexEnd,
                                _ => JustifyContent::FlexStart,
                            },
                            ..default()
                        },
                    ))
                    .with_children(|c| {
                        // His "a little bit too big": three quarters of the
                        // size this drew at before.
                        bo2.spawn_line(c, "Default", &runs, px * 0.94, px * 0.06);
                    });
                    continue;
                }
                p.spawn((
                    Text::new(""),
                    game_text_font(&font.0, px),
                    TextColor(color),
                    shadow(),
                    TextLayout::justify(justify),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(left),
                        top: Val::Px(top),
                        width: Val::Px(w),
                        ..default()
                    },
                ))
                .with_children(|t| {
                    for (run, c) in hint_runs(&e.text, &keys) {
                        let c = if c == Color::WHITE {
                            color
                        } else {
                            c.with_alpha(e.color[3])
                        };
                        t.spawn((
                            TextSpan::new(run),
                            game_text_font(&font.0, px),
                            TextColor(c),
                        ));
                    }
                });
            }
        });
}

#[derive(Component)]
struct ZmIconsRoot;

/// The game's icon for a player client field (`setclientfieldtoplayer`):
/// the perks he has, the power-ups running.
fn icon_material(field: &str) -> Option<&'static str> {
    Some(match field {
        "perk_juggernaut" => "specialty_juggernaut_zombies",
        "perk_quick_revive" => "specialty_quickrevive_zombies",
        "perk_sleight_of_hand" => "specialty_fastreload_zombies",
        "perk_double_tap" => "specialty_doubletap_zombies",
        "powerup_instant_kill" => "specialty_instakill_zombies",
        "powerup_double_points" => "specialty_doublepoints_zombies",
        "powerup_fire_sale" => "specialty_firesale_zombies",
        _ => return None,
    })
}

/// Perk icons in a row above the round, in the order he got them (a paused
/// perk dimmed); running power-ups centred at the bottom (a power-up about
/// to end blinks: the server's 2 is its dark beat).
#[allow(clippy::too_many_arguments)]
fn zm_icons(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    icons: Option<Res<assets::T6HudIcons>>,
    mut images: ResMut<Assets<Image>>,
    window: Query<&Window, With<PrimaryWindow>>,
    roots: Query<Entity, With<ZmIconsRoot>>,
    mut handles: Local<Vec<(String, Handle<Image>)>>,
    mut shown: Local<(String, f32)>,
) {
    let value = match (presented.as_deref(), local.as_deref()) {
        // BO2's own HUD (unless BO2ZM_LUI=0) draws the perks and power-ups.
        (Some(p), Some(l)) if !crate::lui_hud::lui_enabled() => p.snapshot().and_then(|snap| {
            let dvars = snap.meta.script_dvars(l.0);
            if dvars.string("bo2zm_hud_hidden") == Some("1") {
                return None;
            }
            dvars.string("bo2zm_icons").map(str::to_owned)
        }),
        _ => None,
    };
    let Some(value) = value else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *shown = (String::new(), 0.0);
        return;
    };
    if handles.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        for (name, image) in &icons.0 {
            handles.push((name.clone(), images.add((**image).clone())));
        }
    }
    let scale = window
        .single()
        .map_or(1.0, |w| (w.height() / 1080.0).max(0.4));
    if !roots.is_empty() && shown.0 == value && (shown.1 - scale).abs() < 1e-3 {
        return;
    }
    *shown = (value.clone(), scale);
    for e in &roots {
        commands.entity(e).despawn();
    }
    let handle = |m: &str| handles.iter().find(|(n, _)| n == m).map(|(_, h)| h.clone());
    let mut perks = Vec::new();
    let mut powerups = Vec::new();
    for entry in value.split(',').filter(|e| !e.is_empty()) {
        let Some((field, v)) = entry.split_once(':') else {
            continue;
        };
        let v: i32 = v.parse().unwrap_or(0);
        let Some(h) = icon_material(field).and_then(handle) else {
            continue;
        };
        if field.starts_with("perk_") {
            perks.push((h, if v == 2 { 0.4 } else { 1.0 }));
        } else if v != 2 {
            powerups.push(h);
        }
    }
    let row = |left: Option<Val>, bottom: Val| Node {
        position_type: PositionType::Absolute,
        left: left.unwrap_or(Val::Auto),
        width: if left.is_none() {
            Val::Percent(100.0)
        } else {
            Val::Auto
        },
        bottom,
        justify_content: if left.is_none() {
            JustifyContent::Center
        } else {
            JustifyContent::Start
        },
        column_gap: Val::Px(8.0 * scale),
        ..default()
    };
    commands
        .spawn((
            ZmIconsRoot,
            UiLayer::Hud,
            UiLayerVisibility,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn(row(Some(Val::Percent(3.5)), Val::Percent(15.0)))
                .with_children(|r| {
                    for (h, alpha) in perks {
                        let size = Val::Px(52.0 * scale);
                        r.spawn((
                            ImageNode::new(h).with_color(Color::srgba(1.0, 1.0, 1.0, alpha)),
                            Node {
                                width: size,
                                height: size,
                                ..default()
                            },
                        ));
                    }
                });
            p.spawn(row(None, Val::Percent(7.0))).with_children(|r| {
                for h in powerups {
                    let size = Val::Px(72.0 * scale);
                    r.spawn((
                        ImageNode::new(h),
                        Node {
                            width: size,
                            height: size,
                            ..default()
                        },
                    ));
                }
            });
        });
}

#[derive(Component)]
struct ZmPopupRoot;

/// A points popup flying off the score: born (s), how long it flies, how far
/// left and up it goes, and where it started (from the right and bottom).
#[derive(Component)]
struct ZmPopup {
    born: f32,
    dur: f32,
    dx: f32,
    dy: f32,
    right: f32,
    bottom: f32,
}

const LOSE_RED: Color = Color::srgb(0.85, 0.08, 0.05);

/// BO2's floating score (hudcompetitivescoreboardzombie.lua): each time his
/// points change, the change flies off the left of the score and fades:
/// "+50" as he earns, "-950" in red as he spends (its FlyingDuration /
/// FlyingLeftOffSet / FlyingTopOffSet ranges, measured here as numbers of
/// our own).
#[allow(clippy::too_many_arguments)]
fn zm_popups(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    bo2: Option<Res<crate::bo2_font::Bo2Fonts>>,
    window: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    roots: Query<Entity, With<ZmPopupRoot>>,
    mut popups: Query<(Entity, &ZmPopup, &mut Node)>,
    children: Query<&Children>,
    mut glyphs: Query<&mut ImageNode>,
    mut last: Local<Option<i32>>,
    mut seed: Local<u32>,
    mut since: Local<Option<f32>>,
) {
    let values = match (presented.as_deref(), local.as_deref()) {
        // BO2's own HUD (unless BO2ZM_LUI=0) flies its own points.
        (Some(p), Some(l)) if !crate::lui_hud::lui_enabled() => read_values(p, l),
        _ => None,
    };
    let Some(values) = values else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *last = None;
        *since = None;
        return;
    };
    let now = time.elapsed_secs();
    let shown_for = now - *since.get_or_insert(now);
    // Fly and fade.
    for (e, p, mut node) in &mut popups {
        let t = (now - p.born) / p.dur;
        if t >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        let ease = 1.0 - (1.0 - t) * (1.0 - t);
        node.right = Val::Px(p.right + p.dx * ease);
        node.bottom = Val::Px(p.bottom + p.dy * ease);
        let alpha = if t < 0.35 {
            1.0
        } else {
            1.0 - (t - 0.35) / 0.65
        };
        let mut stack: Vec<Entity> = children.get(e).map(|c| c.to_vec()).unwrap_or_default();
        while let Some(c) = stack.pop() {
            if let Ok(mut img) = glyphs.get_mut(c) {
                // The dark copy under each letter keeps its own share.
                let shadow = img.color.to_srgba().red == 0.0;
                img.color
                    .set_alpha(if shadow { alpha * 0.75 } else { alpha });
            }
            if let Ok(more) = children.get(c) {
                stack.extend(more.iter());
            }
        }
    }
    let Some(bo2) = bo2.as_deref() else {
        *last = Some(values.points);
        return;
    };
    let root = match roots.iter().next() {
        Some(r) => r,
        None => commands
            .spawn((
                ZmPopupRoot,
                UiLayer::Hud,
                UiLayerVisibility,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
            ))
            .id(),
    };
    let before = last.replace(values.points);
    let Some(before) = before else { return };
    let change = values.points - before;
    // The game's starting points arrive in its first moments: no popup.
    if change == 0 || shown_for < 3.0 {
        return;
    }
    let (sw, sh) = window
        .single()
        .map_or((1920.0, 1080.0), |w| (w.width(), w.height()));
    let scale = (sh / 1080.0).max(0.4);
    // The score's left edge (it is drawn right-aligned 3.5% from the right,
    // 14% up, 46 px Default).
    let score_w = bo2.line_width("Default", &values.points.to_string(), 46.0 * scale);
    let right = sw * 0.035 + score_w + 6.0 * scale;
    let bottom = sh * 0.14 + 4.0 * scale;
    let mut rand = || {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) ^ (now.to_bits());
        (*seed >> 8) as f32 / 16_777_216.0
    };
    let (text, color) = if change > 0 {
        (format!("+{change}"), POINTS_WHITE)
    } else {
        (format!("-{}", -change), LOSE_RED)
    };
    if std::env::var_os("IW4L_T6_HUDLOG").is_some() {
        diag::info!(World, "bo2zm hud popup {text} (points {})", values.points);
    }
    let popup = ZmPopup {
        born: now,
        dur: 0.6 + 0.4 * rand(),
        dx: (50.0 + 90.0 * rand()) * scale,
        dy: (-30.0 + 70.0 * rand()) * scale,
        right,
        bottom,
    };
    commands.entity(root).with_children(|p| {
        p.spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(right),
                bottom: Val::Px(bottom),
                ..default()
            },
            popup,
        ))
        .with_children(|c| {
            bo2.spawn_line(c, "Default", &[(text, color)], 34.0 * scale, 2.0 * scale);
        });
    });
}

#[derive(Component)]
struct ZmHurtRoot;

/// bo2zm M3 fix list 2: the red screen edges when he is hurt, BO2's own
/// `overlay_low_health` over the whole screen: as strong as the health he
/// has lost the moment he is hit, fading as it comes back (IW4's blood
/// overlay curve, 0.3 a second: BO2's engine keeps its own in the exe, so
/// the fade speed is OUR CHOICE). The health script's red flashing at a
/// fifth of his health still pulses on top. His "when I get hit, I have no
/// effect like I got hit. My screen should get red."
#[allow(clippy::too_many_arguments)]
fn zm_hurt_overlay(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    icons: Option<Res<assets::T6HudIcons>>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time>,
    mut roots: Query<(Entity, &mut ImageNode), With<ZmHurtRoot>>,
    (mut picture, mut intensity): (Local<Option<Handle<Image>>>, Local<f32>),
) {
    // Below the HUD's text; the dead and spectators get none.
    const RATE: f32 = 0.3;
    if picture.is_none()
        && let Some(icons) = icons.as_deref()
        && let Some((_, image)) = icons.0.iter().find(|(n, _)| n == "overlay_low_health")
    {
        *picture = Some(images.add((**image).clone()));
    }
    let lost = match (presented.as_deref(), local.as_deref()) {
        (Some(p), Some(l)) => p
            .player(l.0)
            .filter(|ps| ps.health > 0 && ps.max_health > 0 && ps.pm_type < 2)
            .map(|ps| 1.0 - (ps.health as f32 / ps.max_health as f32).clamp(0.0, 1.0)),
        _ => None,
    };
    let Some(lost) = lost else {
        *intensity = 0.0;
        for (e, _) in &roots {
            commands.entity(e).despawn();
        }
        return;
    };
    *intensity = if lost > *intensity {
        lost
    } else {
        (*intensity - time.delta_secs() * RATE).max(lost)
    };
    let Some(handle) = picture.as_ref() else {
        return;
    };
    let color = Color::srgba(1.0, 1.0, 1.0, *intensity);
    if let Ok((_, mut node)) = roots.single_mut() {
        node.color = color;
        return;
    }
    if *intensity <= 0.0 {
        return;
    }
    commands.spawn((
        ZmHurtRoot,
        UiLayer::Hud,
        UiLayerVisibility,
        // Stretched over the screen, as BO2 draws it (not kept square).
        ImageNode::new(handle.clone())
            .with_color(color)
            .with_mode(bevy::ui::widget::NodeImageMode::Stretch),
        ZIndex(-2000),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
    ));
}

pub(crate) fn register_zm_hud_systems(app: &mut App) {
    app.add_systems(
        Update,
        (
            crate::bo2_font::load_bo2_fonts,
            (zm_hud, zm_script_hud, zm_icons, zm_popups, zm_hurt_overlay),
        )
            .chain()
            .in_set(ClientSet::Ui),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_runs_fill_the_key_and_split_colours() {
        let keys = HudInputView {
            use_key: Some("F".into()),
            ..Default::default()
        };
        let runs = hint_runs("Hold ^3[{+activate}]^7 for M14 [Cost: 500]", &keys);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].0, "Hold ");
        assert_eq!(runs[1].0, "[F]");
        assert_eq!(runs[2].0, " for M14 [Cost: 500]");
    }
}
