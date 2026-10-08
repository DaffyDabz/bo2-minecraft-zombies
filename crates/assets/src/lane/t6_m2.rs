//! bo2zm M2: a Black Ops II map's guns, arms, animations, effects and
//! sound, from every zone the map loads.
//!
//! A Zombies map loads shared zones before its own (`code_post_gfx_zm`,
//! `common_zm`, `patch_zm`, ...) and its patch after. Each is walked once
//! and captured; later zones win a name they share with an earlier one
//! (the patch's weapons replace the map's). Comma-named assets are
//! references to another zone's and are skipped.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use asset_anim::XAnimBuild;
use asset_game::{FxCatalog, OwnedFxImpactTable, WeaponBuild, WeaponCatalog};
use asset_material::MaterialCatalog;
use asset_model::{FpvMeshBuild, WorldWeaponBuild};
use asset_t6::{PackSet, ZoneCapture};

/// The arms a Zombies map gives its players, by map (from the map's own
/// script: `zm_nuked.gsc` sets `c_zom_suit_viewhands` for the CIA player and
/// `c_zom_hazmat_viewhands_light` for CDC).
pub const T6_PLAYER_ARMS: &str = "c_zom_suit_viewhands";
const T6_OTHER_ARMS: [&str; 1] = ["c_zom_hazmat_viewhands_light"];

/// Zones a Zombies map loads before its own, in order.
const T6_ZM_PRE_ZONES: [&str; 4] = ["code_post_gfx_zm", "common_zm", "patch_zm", "patch_ui_zm"];

pub(crate) struct T6Combat {
    pub weapons: WeaponBuild,
    pub fpv: FpvMeshBuild,
    pub world_weapons: WorldWeaponBuild,
    pub xanims: XAnimBuild,
    pub fx: FxCatalog,
    /// The models effects draw (shell casings, debris).
    pub fx_models: asset_game::FxModelCatalog,
    /// The tracers every zone defines (each weapon names its own).
    pub tracers: asset_game::TracerCatalog,
    pub impact_fx: Option<OwnedFxImpactTable>,
    pub sound: Result<asset_audio::SoundCatalog, String>,
    /// The map's footstep tables (first person first), alias ids per
    /// surface and step kind.
    pub footsteps: Vec<asset_t6::FootstepTableRef>,
    /// The default ambient room's tone, from the map's ambience script.
    pub room_tone: Option<String>,
    /// bo2zm M3: every zone's compiled scripts, string tables and entity
    /// strings, in load order.
    pub scripts: crate::T6ScriptSet,
    /// bo2zm M3: the Zombies HUD's icons.
    pub hud_icons: crate::T6HudIcons,
    /// bo2zm M3: the Zombies HUD's fonts.
    pub hud_fonts: crate::T6HudFonts,
    /// bo2zm M4: Black Ops II's UI scripts and what they read.
    pub ui: crate::T6Ui,
    /// bo2zm: the zones' bullet penetration table.
    pub pen_table: Option<weapon_iw4::PenetrationDepthTable>,
    pub report: Vec<String>,
}

/// The Zombies HUD's icons: the perks a Nuketown machine sells and the
/// power-ups that last a while (their materials' colour maps).
const HUD_ICONS: [&str; 47] = [
    "specialty_juggernaut_zombies",
    "specialty_quickrevive_zombies",
    "specialty_fastreload_zombies",
    "specialty_doubletap_zombies",
    "specialty_instakill_zombies",
    "specialty_doublepoints_zombies",
    "specialty_firesale_zombies",
    // The round's chalk marks, rounds 1 to 5 (hudroundstatuszombie.lua).
    "hud_chalk_1",
    "hud_chalk_2",
    "hud_chalk_3",
    "hud_chalk_4",
    "hud_chalk_5",
    // The grenades and mines he carries, one icon each (offhandicons.lua):
    // frag, Semtex, monkey, Claymore.
    "hud_us_grenade",
    "hud_icon_sticky_grenade",
    "hud_cymbal_monkey",
    "hud_icon_claymore",
    // The red screen-edge overlay the health script fades in when he is hit
    // (_zm_playerhealth's healthoverlay).
    "overlay_low_health",
    // The guns' hip-fire crosshair: its four lines and the centre pieces
    // some guns name (weapon defs' reticleSide / reticleCenter).
    "reticle_side_small",
    "reticle_center_cross",
    "reticle_flechette",
    // The scoreboard's team badges ("faction_" .. GetFactionForTeam): the
    // CDC and CIA (Nuketown's teamset).
    "faction_cdc",
    "faction_cia",
    // Nuketown's desert map behind its start locations: GameMapZombie
    // builds the name ("menu_" .. GetUIMapName() .. "_map", and "_blur"),
    // so no script names it whole.
    "menu_zm_nuked_map",
    "menu_zm_nuked_map_blur",
    // BO2's Xbox button pictures (code_post_gfx_zm): the pad's prompts in
    // its menus and hints (frame::BO2_XBOX_GLYPHS).
    "xenonbutton_a",
    "xenonbutton_b",
    "xenonbutton_x",
    "xenonbutton_y",
    "xenonbutton_lb",
    "xenonbutton_rb",
    "xenonbutton_lt",
    "xenonbutton_rt",
    "xenonbutton_back",
    "xenonbutton_start",
    "xenonbutton_ls",
    "xenonbutton_rs",
    "xenonbutton_dpad_all",
    "xenonbutton_dpad_ud",
    "xenonbutton_dpad_rl",
    "xenonbutton_dpad_up",
    "xenonbutton_dpad_down",
    "xenonbutton_dpad_left",
    "xenonbutton_dpad_right",
    // BO2 on PC: its menu tabs' arrows (frame::bo2_cycle_glyph).
    "ui_arrow_left",
    "ui_arrow_right",
    // The lobby rows' speaker (the engine's VoipImage, code_post_gfx_zm):
    // muted, and talking (its left half is the quiet one).
    "voice_off",
    "voice_on",
];

/// The HUD's fonts by the Lua scripts' names and their files, as
/// `ui/t6/codbase.lua` registers them for English.
const HUD_FONTS: [(&str, &str); 7] = [
    ("Default", "fonts/720/normalFont"),
    ("Condensed", "fonts/720/smallFont"),
    ("Big", "fonts/720/bigFont"),
    ("Morris", "fonts/720/extraBigFont"),
    ("ExtraSmall", "fonts/720/extraSmallFont"),
    ("Italic", "fonts/720/italicFont"),
    ("SmallItalic", "fonts/720/smallItalicFont"),
];

fn hud_fonts(
    captures: &[ZoneCapture],
    packs: Option<&PackSet>,
    report: &mut Vec<String>,
) -> crate::T6HudFonts {
    let mut out = crate::T6HudFonts::default();
    let mut sheet_material = None;
    for (name, file) in HUD_FONTS {
        let Some(font) = captures
            .iter()
            .rev()
            .flat_map(|c| c.fonts.iter())
            .find(|f| f.name.eq_ignore_ascii_case(file))
        else {
            report.push(format!("t6 hud font {name}: {file} missing"));
            continue;
        };
        sheet_material.get_or_insert_with(|| font.material.clone());
        out.fonts.push(crate::T6HudFont {
            name: name.to_owned(),
            pixel_height: font.pixel_height as f32,
            glyphs: font
                .glyphs
                .iter()
                .map(|g| {
                    (
                        g.letter,
                        [
                            i16::from(g.x0),
                            i16::from(g.y0),
                            i16::from(g.dx),
                            i16::from(g.pixel_width),
                            i16::from(g.pixel_height),
                        ],
                        g.st,
                    )
                })
                .collect(),
        });
    }
    // Every one of them draws from the same sheet (`gamefonts_pc`).
    if let (Some(material), Some(packs)) = (sheet_material, packs) {
        let image = captures.iter().find_map(|c| {
            let m = c.materials.iter().find(|m| m.name == material)?;
            let t = m.textures.first()?;
            c.images.get(t.image?.index)
        });
        match image.map(|i| super::t6_materials::decode(packs, i)) {
            Some(Ok(img)) => out.sheet = Some(std::sync::Arc::new(img)),
            Some(Err(e)) => report.push(format!("t6 hud font sheet {material}: {e}")),
            None => report.push(format!("t6 hud font sheet {material}: no image")),
        }
    }
    report.push(format!(
        "t6 hud fonts: {} ({})",
        out.fonts.len(),
        out.fonts
            .iter()
            .map(|f| format!("{} {}px {} glyphs", f.name, f.pixel_height, f.glyphs.len()))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out
}

/// bo2zm M4: the zones Black Ops II's Zombies front end reads (main menu,
/// lobby, globe and map picker), in load order; Nuketown's load zone adds
/// its map pictures and loading screen.
const T6_ZM_FRONTEND_ZONES: [&str; 6] = [
    "code_post_gfx_zm",
    "common_zm",
    "patch_zm",
    "ui_zm",
    "patch_ui_zm",
    "dlczm0_load_zm",
];

/// bo2zm M4: Black Ops II's Zombies front end without a map: its UI
/// scripts, the pictures they name, the fonts, English strings and string
/// tables. Kept for the whole run: the menu comes back after each game.
#[derive(Clone, bevy::prelude::Resource)]
pub struct T6Frontend {
    pub ui: crate::T6Ui,
    pub icons: crate::T6HudIcons,
    pub fonts: crate::T6HudFonts,
    /// Its sounds: the menus' clicks and music (`zmb_code_post_gfx`).
    pub sound: Option<std::sync::Arc<asset_audio::SoundCatalog>>,
    pub report: Vec<String>,
}

/// bo2zm M4: read the front end from the zones in `zone_dir`
/// (`zone/all`; the English zones are in `zone/english` beside it).
pub fn load_t6_frontend(zone_dir: &Path) -> T6Frontend {
    let started = std::time::Instant::now();
    let mut report = Vec::new();
    let capture_all = |dir: &Path, prefix: &str, report: &mut Vec<String>| -> Vec<ZoneCapture> {
        T6_ZM_FRONTEND_ZONES
            .iter()
            .filter_map(|z| {
                let path = dir.join(format!("{prefix}{z}.ff"));
                if !path.is_file() {
                    return None;
                }
                match asset_t6::capture_zone(&path) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        report.push(format!("t6 front end zone gap: {e}"));
                        None
                    }
                }
            })
            .collect()
    };
    let caps = capture_all(zone_dir, "", &mut report);
    let lang_dir = zone_dir.parent().map(|d| d.join("english"));
    let lang_caps = lang_dir
        .as_deref()
        .map(|d| capture_all(d, "en_", &mut report))
        .unwrap_or_default();
    let captures: Vec<&ZoneCapture> = caps.iter().collect();
    report.push(format!(
        "t6 front end zones: {}; English: {}",
        captures
            .iter()
            .map(|c| c.zone.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        lang_caps
            .iter()
            .map(|c| c.zone.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let packs = match PackSet::open_dir(zone_dir) {
        Ok(p) => Some(p),
        Err(e) => {
            report.push(format!("t6 front end image packs: {e}"));
            None
        }
    };
    let mut strings: HashMap<String, String> = HashMap::new();
    for c in &lang_caps {
        for (k, v) in &c.localize {
            strings.insert(k.clone(), v.clone());
        }
    }
    let mut tables = Vec::new();
    for c in &captures {
        for t in &c.string_tables {
            if real(&t.name) {
                tables.push((t.name.clone(), t.columns, t.rows, t.cells.clone()));
            }
        }
    }
    let read_ms = started.elapsed().as_millis();
    let scripts = ui_scripts(&captures);
    let mut names = ui_strings(&scripts);
    // Every picture the front-end zones hold: the menus also name them
    // from tables and joined strings (`menu_` .. map .. `_map`, the maps
    // table's signposts), which no script constant spells out.
    for c in captures
        .iter()
        .filter(|c| matches!(c.zone.as_str(), "ui_zm" | "patch_ui_zm" | "dlczm0_load_zm"))
    {
        names.extend(
            c.materials
                .iter()
                .filter(|m| real(&m.name))
                .map(|m| m.name.clone()),
        );
    }
    // And the pictures string tables name (rank icons, map signposts).
    for (_, _, _, cells) in &tables {
        names.extend(
            cells
                .iter()
                .filter(|c| !c.is_empty() && c.len() < 64)
                .cloned(),
        );
    }
    let icons = hud_icons(&captures, packs.as_ref(), &names, &mut report);
    let icons_ms = started.elapsed().as_millis();
    let lang_packs = lang_dir.as_deref().and_then(|d| PackSet::open_dir(d).ok());
    let fonts = hud_fonts(
        &lang_caps,
        lang_packs.as_ref().or(packs.as_ref()),
        &mut report,
    );
    // The menus' sounds, from the same banks a map reads them from.
    let sound_dir = zone_dir
        .parent()
        .and_then(Path::parent)
        .map(|install| install.join("sound"));
    let sound = sound_dir.filter(|d| d.is_dir()).map(|dir| {
        let bases = bank_bases(&captures, "");
        let base_refs: Vec<&str> = bases.iter().map(String::as_str).collect();
        let (index, bank_report) = asset_audio::T6BankIndex::open(&dir, &base_refs);
        report.extend(bank_report);
        let banks: Vec<&asset_t6::SndBankRef> = captures
            .iter()
            .copied()
            .chain(lang_caps.iter())
            .flat_map(|c| c.sound_banks.iter())
            .collect();
        let globals = captures.iter().find_map(|c| c.snd_globals.as_ref());
        let (catalog, census) = asset_audio::build_t6_sound_catalog(
            &banks,
            globals,
            &index,
            asset_core::ZoneOwner::intern("frontend"),
        );
        report.push(format!(
            "t6 front end sound: {} aliases, {} with a clip",
            census.aliases, census.with_clip
        ));
        std::sync::Arc::new(catalog)
    });
    report.push(format!(
        "t6 front end: {} scripts, {} strings, {} string tables; zones read in {read_ms} ms, pictures by {icons_ms} ms, all in {} ms",
        scripts.len(),
        strings.len(),
        tables.len(),
        started.elapsed().as_millis()
    ));
    let mut out = T6Frontend {
        ui: crate::T6Ui {
            scripts,
            strings: std::sync::Arc::new(strings.into_iter().collect()),
            tables: std::sync::Arc::new(tables),
            game_settings: std::sync::Arc::new(zm_game_settings(&captures)),
        },
        icons,
        fonts,
        sound,
        report,
    };
    // bo2mc: MINECRAFT next to NUKETOWN in the map pick.
    crate::bo2mc_frontend::add_minecraft(&mut out);
    out
}

/// bo2zm M4: Zombies' game settings defaults, from the zones' rawfiles
/// (`gametype_setting <name> <value>` in the default settings file, then
/// the standard mode's).
fn zm_game_settings(captures: &[&ZoneCapture]) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = Vec::new();
    for file in ["gamesettings_default.cfg", "gamesettings_zstandard.cfg"] {
        let Some(bytes) = captures
            .iter()
            .rev()
            .flat_map(|c| c.raw_files.iter())
            .find(|(n, _)| {
                n.to_ascii_lowercase().ends_with(file) && n.to_ascii_lowercase().contains("zm")
            })
            .map(|(_, b)| b.clone())
        else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let mut w = line.split_whitespace();
            if w.next() != Some("gametype_setting") {
                continue;
            }
            let (Some(name), Some(value)) =
                (w.next(), w.next().and_then(|v| v.parse::<f32>().ok()))
            else {
                continue;
            };
            out.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            out.push((name.to_owned(), value));
        }
    }
    out
}

/// bo2zm M4: every UI script (`.lua` rawfile) the zones hold, in load
/// order.
fn ui_scripts(captures: &[&ZoneCapture]) -> Vec<(String, std::sync::Arc<[u8]>)> {
    captures
        .iter()
        .flat_map(|c| c.raw_files.iter())
        .filter(|(name, _)| name.to_ascii_lowercase().ends_with(".lua"))
        .map(|(name, bytes)| (name.clone(), std::sync::Arc::from(bytes.as_slice())))
        .collect()
}

/// bo2zm M4: the material names the UI scripts may draw: every string
/// constant in them (`RegisterMaterial("hud_chalk_1")`).
fn ui_strings(scripts: &[(String, std::sync::Arc<[u8]>)]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for (_, bytes) in scripts {
        let Ok(chunk) = hks_t6::parse(bytes) else {
            continue;
        };
        let mut stack = vec![&chunk.main];
        while let Some(p) = stack.pop() {
            for k in &p.consts {
                if let hks_t6::Const::String(s) = k
                    && s.len() < 64
                {
                    out.insert(String::from_utf8_lossy(s).into_owned());
                }
            }
            stack.extend(p.protos.iter().map(|c| &**c));
        }
    }
    out
}

fn hud_icons(
    captures: &[&ZoneCapture],
    packs: Option<&PackSet>,
    ui_names: &std::collections::BTreeSet<String>,
    report: &mut Vec<String>,
) -> crate::T6HudIcons {
    let mut icons = Vec::new();
    let Some(packs) = packs else {
        return crate::T6HudIcons(icons);
    };
    // The materials the UI scripts name, beside the hand-picked ones.
    let mut wanted: Vec<String> = HUD_ICONS
        .iter()
        .chain(super::t6_perks::HUD_ICONS.iter())
        .map(|s| (*s).to_owned())
        .collect();
    let mut seen: std::collections::BTreeSet<String> = wanted.iter().cloned().collect();
    // bo2mc: every weapon's kill icon and HUD icon (its picture as an item).
    for c in captures {
        for w in &c.weapons {
            for (off, name) in &w.materials {
                if (*off == 1632 || *off == 940) && seen.insert(name.clone()) {
                    wanted.push(name.clone());
                }
            }
        }
    }
    for c in captures {
        for m in &c.materials {
            if ui_names.contains(&m.name) && seen.insert(m.name.clone()) {
                wanted.push(m.name.clone());
            }
        }
    }
    let mut failed = 0usize;
    let mut failed_names: Vec<String> = Vec::new();
    let mut additive = 0usize;
    for name in &wanted {
        let name = name.as_str();
        let mut add = false;
        // A patch zone's copy replaces the one it patches (`patch_ui_zm`'s
        // Theater frame is silver with light cracks, `ui_zm`'s dull).
        let patched = captures.iter().filter(|c| c.zone.starts_with("patch_"));
        let rest = captures.iter().filter(|c| !c.zone.starts_with("patch_"));
        let found = patched.chain(rest).find_map(|c| {
            let m = c.materials.iter().find(|m| m.name == name)?;
            add = super::t6_materials::is_additive(m);
            let t = super::t6_materials::colour_texture(m, c)?;
            c.images.get(t.image?.index)
        });
        let Some(image) = found else {
            if HUD_ICONS.contains(&name) {
                report.push(format!("t6 hud icon {name}: no material"));
            }
            failed_names.push(format!("{name} (no colour map)"));
            failed += 1;
            continue;
        };
        match super::t6_materials::decode(packs, image) {
            Ok(img) => {
                // Additive pictures (menu brackets, glows) get their
                // brightness as alpha: the 2D layer only alpha-blends.
                let img = match add
                    .then(|| super::t6_materials::additive_to_alpha(&img))
                    .flatten()
                {
                    Some(a) => {
                        additive += 1;
                        a
                    }
                    None => img,
                };
                // The lobby's quiet speaker: BO2 draws `voice_on` without
                // its sound waves while nobody talks.
                if name == "voice_on" {
                    if let Some(quiet) = super::t6_materials::left_part(&img, 0.5) {
                        icons.push(("voice_quiet".to_owned(), std::sync::Arc::new(quiet)));
                    }
                }
                icons.push((name.to_owned(), std::sync::Arc::new(img)));
            }
            Err(e) => {
                if HUD_ICONS.contains(&name) {
                    report.push(format!("t6 hud icon {name}: {e}"));
                }
                failed_names.push(format!("{name} ({e})"));
                failed += 1;
            }
        }
    }
    if !failed_names.is_empty() {
        report.push(format!(
            "t6 hud icons without a picture: {}",
            failed_names.join(", ")
        ));
    }
    report.push(format!(
        "t6 hud icons: {} of {} ({} hand-picked, the rest the UI scripts name; {failed} without a picture; {additive} additive made see-through)",
        icons.len(),
        wanted.len(),
        HUD_ICONS.len()
    ));
    crate::T6HudIcons(icons)
}

/// Nuketown's weapons: every base gun the box and walls give, the starting
/// pistol, knives and equipment.
const NUKETOWN_WEAPONS: [&str; 40] = [
    "m1911_zm",
    "beretta93r_zm",
    "fiveseven_zm",
    "fivesevendw_zm",
    "kard_zm",
    "python_zm",
    "judge_zm",
    "ak74u_zm",
    "mp5k_zm",
    "qcw05_zm",
    "870mcs_zm",
    "rottweil72_zm",
    "saiga12_zm",
    "srm1216_zm",
    "m14_zm",
    "m16_zm",
    "fnfal_zm",
    "galil_zm",
    "hk416_zm",
    "saritch_zm",
    "tar21_zm",
    "type95_zm",
    "xm8_zm",
    "hamr_zm",
    "lsat_zm",
    "rpd_zm",
    "barretm82_zm",
    "dsr50_zm",
    "ray_gun_zm",
    "raygun_mark2_zm",
    "knife_ballistic_zm",
    "m32_zm",
    "usrpg_zm",
    "knife_zm",
    "bowie_knife_zm",
    "tazer_knuckles_zm",
    "frag_grenade_zm",
    "sticky_grenade_zm",
    "claymore_zm",
    "cymbal_monkey_zm",
];

/// The M2 census: every Nuketown weapon complete (first-person and world
/// models, launcher and melee models, magazine and scope attachments,
/// animations), and nothing the weapons,
/// their animations, the impact table or the map scripts name missing from
/// the effects or the sound bank. A sound BO2's own banks lack (an
/// animation's cue Nuketown never shipped) is silent in BO2 too: counted
/// apart, as is an alias BO2 ships without audio.
#[allow(clippy::too_many_arguments)]
fn census(
    captures: &[&ZoneCapture],
    weapons: &[&str],
    weapon_refs: &HashMap<String, &asset_t6::WeaponRef>,
    models_missing: &[String],
    xanims: &XAnimBuild,
    fx: &FxCatalog,
    impact: Option<&OwnedFxImpactTable>,
    map_fx: &asset_audio::ScriptedMapFx,
    room_tone: Option<&str>,
    sound: Option<&asset_audio::SoundCatalog>,
) -> Vec<String> {
    use fastfile_t6::layout::WeaponDef as d;
    let ns = asset_core::AssetNamespace::T6;
    let mut missing_weapons: Vec<String> = Vec::new();
    let mut missing_fx: Vec<String> = Vec::new();
    let mut missing_sounds: Vec<String> = Vec::new();
    let mut absent_in_bo2: Vec<String> = Vec::new();
    let mut silent: usize = 0;
    let (mut fx_n, mut snd_n) = (0usize, 0usize);
    let anim_notifies: HashMap<String, &asset_t6::XAnimRef> = captures
        .iter()
        .flat_map(|c| c.xanims.iter())
        .filter(|a| !a.name.starts_with(','))
        .map(|a| (a.name.to_ascii_lowercase(), a))
        .collect();
    let aliases: HashMap<String, &asset_audio::CapturedSound> = sound
        .map(|cat| {
            cat.sounds
                .iter()
                .map(|s| (s.name.to_ascii_lowercase(), s))
                .collect()
        })
        .unwrap_or_default();
    let mut check_fx = |name: &str, what: &str, missing: &mut Vec<String>| {
        if name.is_empty() {
            return;
        }
        fx_n += 1;
        if fx.get_in(ns, name.trim_start_matches(',')).is_none() {
            missing.push(format!("{what}: {name}"));
        }
    };
    // A sound named by the data: in the bank with audio (or shipped silent),
    // else absent from BO2's own banks (silent in BO2 too). Missing means
    // in the bank but no audio found: a failure here.
    let mut check_sound = |name: &str, _from_anim: bool, what: &str| {
        if name.is_empty() {
            return;
        }
        snd_n += 1;
        match aliases.get(&name.to_ascii_lowercase()) {
            None => absent_in_bo2.push(name.to_owned()),
            Some(sound) => {
                let audible = sound.aliases.iter().any(|row| row.streamed.is_some());
                let shipped_silent = sound.aliases.iter().all(|row| {
                    row.streamed.is_none() && row.file_name.as_deref().unwrap_or("").is_empty()
                });
                if !audible {
                    if shipped_silent {
                        silent += 1;
                    } else {
                        missing_sounds.push(format!("{what}: {name} (no audio)"));
                    }
                }
            }
        }
    };
    for &name in weapons {
        let Some(w) = weapon_refs.get(name) else {
            missing_weapons.push(format!("{name}: not in the zones"));
            continue;
        };
        let mut gaps = Vec::new();
        for model in w
            .gun_models
            .first()
            .into_iter()
            .chain(w.world_models.first())
            .map(String::as_str)
            .chain(
                [d::rocketModel, d::projectileModel, d::additionalMeleeModel]
                    .map(|f| w.def_model(f)),
            )
            .chain(w.attach_view_models.iter().map(|a| a.1.as_str()))
            .filter(|m| !m.is_empty() && !m.starts_with(','))
        {
            if models_missing.iter().any(|m| m == model) {
                gaps.push(format!("model {model}"));
            }
        }
        for anim in w.xanims.iter().filter(|a| !a.is_empty()) {
            let key = anim.trim_start_matches(',').to_ascii_lowercase();
            if xanims.get(ns, &key).is_none() {
                gaps.push(format!("anim {anim}"));
            }
            if let Some(a) = anim_notifies.get(&key) {
                for (note, _) in &a.notifies {
                    if let Some(alias) = note.strip_prefix("sndnt#") {
                        check_sound(alias, true, name);
                    }
                }
            }
        }
        if !gaps.is_empty() {
            missing_weapons.push(format!("{name}: {}", gaps.join(", ")));
        }
        for field in [
            d::viewFlashEffect,
            d::worldFlashEffect,
            d::viewShellEjectEffect,
            d::worldShellEjectEffect,
            d::viewLastShotEjectEffect,
            d::worldLastShotEjectEffect,
            d::projExplosionEffect,
            d::projTrailEffect,
            d::projIgnitionEffect,
        ] {
            check_fx(w.def_fx(field), name, &mut missing_fx);
        }
        for field in [
            d::fireSound,
            d::fireSoundPlayer,
            d::emptyFireSound,
            d::emptyFireSoundPlayer,
            d::reloadSound,
            d::reloadSoundPlayer,
            d::reloadEmptySound,
            d::reloadEmptySoundPlayer,
            d::raiseSound,
            d::raiseSoundPlayer,
            d::putawaySound,
            d::putawaySoundPlayer,
            d::projExplosionSound,
            d::meleeSwipeSoundPlayer,
            d::meleeSwipeSound,
            d::meleeHitSound,
            d::meleeMissSound,
            d::pullbackSoundPlayer,
        ] {
            check_sound(w.def_string(field), false, name);
        }
        for alias in &w.bounce_sounds {
            check_sound(alias, false, name);
        }
        // A map value may name the player's alias then the others'.
        for (_, value) in &w.notetrack_sounds {
            for alias in value.split_whitespace() {
                check_sound(alias, false, name);
            }
        }
    }
    if let Some(table) = impact {
        for row in 0..table.row_count() {
            for surf in 0..40 {
                if let Some(n) = table.effect_name(row, surf, Some(0)) {
                    check_fx(n.name, "impact table", &mut missing_fx);
                }
            }
            for flesh in 0..8 {
                if let Some(n) = table.effect_name(row, 7, Some(flesh)) {
                    check_fx(n.name, "impact table", &mut missing_fx);
                }
            }
        }
    }
    for shot in &map_fx.oneshots {
        check_fx(&shot.fxid, "map", &mut missing_fx);
    }
    for l in &map_fx.loop_sounds {
        check_sound(&l.soundalias, false, "map loop");
    }
    for r in &map_fx.random_sounds {
        check_sound(&r.soundalias, false, "map");
    }
    if let Some(tone) = room_tone {
        check_sound(tone, false, "room tone");
    }
    for gait in [
        "sprint",
        "run",
        "walk",
        "prone",
        "crouch_run",
        "crouch_walk",
    ] {
        for surf in [
            "default", "asphalt", "concrete", "dirt", "metal", "wood", "grass", "gravel",
        ] {
            check_sound(&format!("fly_step_{gait}_plr_{surf}"), false, "footsteps");
        }
    }
    for surf in [
        "default", "asphalt", "concrete", "dirt", "metal", "wood", "grass", "gravel",
    ] {
        check_sound(&format!("fly_land_plr_{surf}"), false, "landings");
    }
    missing_fx.sort();
    missing_fx.dedup();
    missing_sounds.sort();
    missing_sounds.dedup();
    absent_in_bo2.sort();
    absent_in_bo2.dedup();
    vec![
        format!(
            "t6 census: {} missing weapons of {} (models, first person, animations){}",
            missing_weapons.len(),
            weapons.len(),
            if missing_weapons.is_empty() {
                String::new()
            } else {
                format!(": {missing_weapons:?}")
            }
        ),
        format!(
            "t6 census: {} missing FX of {fx_n} named (weapons, impacts, map){}",
            missing_fx.len(),
            if missing_fx.is_empty() {
                String::new()
            } else {
                format!(": {:?}", &missing_fx[..missing_fx.len().min(12)])
            }
        ),
        format!(
            "t6 census: {} missing sounds of {snd_n} named (weapons, animations, footsteps, landings, map){}; {} names BO2's own banks lack (silent in BO2 too) {:?}; {silent} shipped silent",
            missing_sounds.len(),
            if missing_sounds.is_empty() {
                String::new()
            } else {
                format!(": {:?}", &missing_sounds[..missing_sounds.len().min(12)])
            },
            absent_in_bo2.len(),
            &absent_in_bo2[..absent_in_bo2.len().min(8)],
        ),
    ]
}

/// The map's compiled client scripts, read for its placed effects and
/// ambience (later zones' copies win: the patch zone fixes the map's).
fn map_scripts(
    captures: &[&ZoneCapture],
    map: &str,
    report: &mut Vec<String>,
) -> (asset_audio::ScriptedMapFx, Option<String>) {
    let facts = asset_t6::MapScriptFacts::read(map, |name| {
        let want = format!("script:{name}");
        captures.iter().rev().find_map(|c| {
            c.raw_files
                .iter()
                .find(|(n, _)| *n == want)
                .map(|(_, bytes)| bytes.clone())
        })
    });
    let shown: Vec<&asset_t6::PlacedFx> = facts
        .placed
        .iter()
        .filter(|p| p.exploder.is_none() && p.kind != "exploder")
        .collect();
    // IW4L_T6_PLACEDLOG=1: each shown placed effect (test aid).
    if std::env::var_os("IW4L_T6_PLACEDLOG").is_some() {
        for p in &shown {
            report.push(format!(
                "t6 placed fx {} = {} {} at {:?} angles {:?} delay {}",
                p.fxid,
                facts.effects.get(&p.fxid).map_or("?", String::as_str),
                p.kind,
                p.origin,
                p.angles,
                p.delay
            ));
        }
    }
    let oneshots: Vec<asset_audio::CreateFxOneshot> = shown
        .iter()
        .map(|p| asset_audio::CreateFxOneshot {
            fxid: facts
                .effects
                .get(&p.fxid)
                .cloned()
                .unwrap_or_else(|| p.fxid.clone()),
            origin_inches: p.origin,
            angles_deg: p.angles,
            // A loop effect's delay is its repeat time, not a pre-roll.
            delay: if p.kind == "loopfx" { 0.0 } else { p.delay },
        })
        .collect();
    let mut loop_sounds: Vec<asset_audio::CreateFxLoopSound> = Vec::new();
    for (fxid, alias, off) in &facts.sounds_on_fx {
        for p in shown.iter().filter(|p| &p.fxid == fxid) {
            loop_sounds.push(asset_audio::CreateFxLoopSound {
                soundalias: alias.clone(),
                origin_inches: [0, 1, 2].map(|a| p.origin[a] + off[a]),
                line_end_inches: None,
            });
        }
    }
    for (alias, origin) in &facts.loops_at {
        loop_sounds.push(asset_audio::CreateFxLoopSound {
            soundalias: alias.clone(),
            origin_inches: *origin,
            line_end_inches: None,
        });
    }
    let mut random_sounds: Vec<asset_audio::RandomPointSound> = facts
        .random_sounds
        .iter()
        .map(|(alias, wait, places)| asset_audio::RandomPointSound {
            soundalias: alias.clone(),
            wait_secs: *wait,
            places_inches: places.clone(),
        })
        .collect();
    let (structs_random, structs_loop) =
        map_sound_structs(captures, &mut random_sounds, &mut loop_sounds);
    let unresolved = shown
        .iter()
        .filter(|p| !facts.effects.contains_key(&p.fxid))
        .count();
    report.push(format!(
        "t6 map sound structs: {structs_random} now-and-then (wind gusts, creaks), {structs_loop} loops (power lines, wind lines, arcs)"
    ));
    report.push(format!(
        "t6 map scripts: {} placed effects ({} shown at load, {} exploders wait for the game, {unresolved} without an effect table entry); {} loop sounds ({} on effects, {} at points); {} now-and-then sounds; room tone {}",
        facts.placed.len(),
        oneshots.len(),
        facts.placed.len() - shown.len(),
        loop_sounds.len(),
        loop_sounds.len() - facts.loops_at.len(),
        facts.loops_at.len(),
        random_sounds.len(),
        facts.room_tone.as_deref().unwrap_or("none"),
    ));
    report.push(format!("t6 map rooms (room, echo, dry, wet): {:?}", facts.room_echoes));
    (
        asset_audio::ScriptedMapFx {
            oneshots,
            loop_sounds,
            random_sounds,
            rooms: facts.room_echoes.clone(),
        },
        facts.room_tone,
    )
}

/// The map's sound structs, which every map's client script
/// (clientscripts/mp/_audio.csc) starts: a "random" one plays its sound at
/// its place every `randomfloatrange(script_wait_min, script_wait_max)` s
/// (1 to 3 by default), a "looper" loops it there, a "line_emitter" loops
/// it along the line to its target. Returns (random, loops) added.
fn map_sound_structs(
    captures: &[&ZoneCapture],
    random: &mut Vec<asset_audio::RandomPointSound>,
    loops: &mut Vec<asset_audio::CreateFxLoopSound>,
) -> (usize, usize) {
    let Some(ents) = captures.iter().rev().find(|c| !c.map_ents.is_empty()) else {
        return (0, 0);
    };
    let ents: Vec<_> = ents
        .map_ents
        .iter()
        .flat_map(|e| asset_t6::parse_entities(e))
        .collect();
    let (mut nr, mut nl) = (0, 0);
    for e in &ents {
        if e.classname() != "script_struct" {
            continue;
        }
        let (Some(alias), Some(origin)) = (e.get("script_sound"), e.vec3("origin")) else {
            continue;
        };
        let alias = alias.to_owned();
        match e.get("script_label") {
            Some("random") => {
                let f = |k: &str, d: f32| e.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(d);
                random.push(asset_audio::RandomPointSound {
                    soundalias: alias,
                    wait_secs: [f("script_wait_min", 1.0), f("script_wait_max", 3.0)],
                    places_inches: vec![origin],
                });
                nr += 1;
            }
            Some("looper") => {
                loops.push(asset_audio::CreateFxLoopSound {
                    soundalias: alias,
                    origin_inches: origin,
                    line_end_inches: None,
                });
                nl += 1;
            }
            Some("line_emitter") => {
                let end = e.get("target").and_then(|t| {
                    ents.iter()
                        .find(|o| o.get("targetname") == Some(t))
                        .and_then(|o| o.vec3("origin"))
                });
                loops.push(asset_audio::CreateFxLoopSound {
                    soundalias: alias,
                    origin_inches: origin,
                    line_end_inches: end,
                });
                nl += 1;
            }
            _ => {}
        }
    }
    (nr, nl)
}

/// bo2zm M3: every XModel the zones carry whose name a server script
/// (as a string constant) or a map entity (`"model"`) names.
/// What a bullet shows on a model's bone boxes: its paint's surface (a
/// mannequin's is plastic), the one most of its surfaces use; 0 (blood) when
/// none has one.
fn model_surface_flags(capture: &ZoneCapture, model: &asset_t6::XModelRef) -> u32 {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for srf in &model.lod0 {
        let flags = srf
            .material
            .and_then(|k| capture.material(k))
            .map_or(0, |m| m.surface_flags);
        if flags != 0 {
            *counts.entry(flags).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map_or(0, |(flags, _)| flags)
}

fn script_model_names(captures: &[&ZoneCapture]) -> BTreeSet<String> {
    let models: BTreeSet<&str> = captures
        .iter()
        .flat_map(|c| c.xmodels.iter().map(|m| m.name.as_str()))
        .filter(|n| real(n))
        .collect();
    let mut named = BTreeSet::new();
    let mut take = |s: &str| {
        if models.contains(s) {
            named.insert(s.to_owned());
        }
    };
    for c in captures {
        for (name, bytes) in &c.raw_files {
            if !name.starts_with("script:") || !name.ends_with(".gsc") {
                continue;
            }
            for token in bytes.split(|&b| b == 0) {
                if (3..96).contains(&token.len()) && token.iter().all(|b| b.is_ascii_graphic()) {
                    take(&String::from_utf8_lossy(token));
                }
            }
        }
        // Every gun's world model (the box's floating gun, bought wall guns).
        for w in &c.weapons {
            for m in &w.world_models {
                take(m);
            }
        }
        // The pieces a breakable prop throws (a mannequin's head and arms).
        for d in &c.destructibles {
            for p in &d.pieces {
                for st in &p.stages {
                    for m in st.spawn_models.iter().flatten() {
                        take(m);
                    }
                }
            }
        }
        for ents in &c.map_ents {
            for e in asset_t6::parse_entities(ents) {
                if let Some(m) = e.get("model").filter(|m| !m.starts_with('*')) {
                    take(m);
                }
                // A zbarrier's pieces (the magic box) name their models.
                for i in 1..=8 {
                    if let Some(m) = e.get(&format!("zbarrierboardmodel{i}")) {
                        take(m);
                    }
                }
            }
        }
    }
    named
}

/// The zones around the map zone, in load order, that exist beside it.
fn zone_paths(map_path: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    // bo2zm: a map from an extra folder (`IW4L_T6_EXTRA`) still loads the
    // install's common zones.
    let dir = asset_transport::t6_extra::install_zone_dir(map_path);
    let dir = dir.as_path();
    let stem = map_path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let mut pre: Vec<PathBuf> = T6_ZM_PRE_ZONES
        .iter()
        .map(|z| dir.join(format!("{z}.ff")))
        .collect();
    // Nuketown is the first Zombies DLC map: its load zone carries the
    // intro's sound bank.
    if stem == "zm_nuked" {
        pre.push(dir.join("dlczm0_load_zm.ff"));
    }
    // A map from an extra folder (Zombies Declassified) loads its mod's
    // zones (weapons, string tables, menus) before the map, as its game does.
    if map_path.parent() != Some(dir) {
        pre.extend(
            ["mod.ff", "mod_load.ff", "mod_patch.ff"]
                .iter()
                .filter_map(|z| asset_transport::t6_extra::extra_file(z)),
        );
    }
    // The mode zone (`so_zclassic_zm_buried.ff`: the map's zombie types,
    // their characters and animstatedefs, the mode's extra entities), then
    // the patch. Nuketown plays zstandard and has none.
    let mode = if stem == "zm_nuked" {
        "zstandard"
    } else {
        "zclassic"
    };
    let post = vec![
        asset_transport::t6_extra::file_in(dir, &format!("so_{mode}_{stem}.ff")),
        asset_transport::t6_extra::file_in(dir, &format!("{stem}_patch.ff")),
    ];
    let exists = |v: Vec<PathBuf>| v.into_iter().filter(|p| p.is_file()).collect();
    (exists(pre), exists(post))
}

fn real(name: &str) -> bool {
    !name.is_empty() && !name.starts_with(',')
}

/// Where each material is really defined: name -> (capture, index), the
/// last zone that defines it (not a comma-named reference) winning.
fn material_homes(captures: &[&ZoneCapture]) -> HashMap<String, (usize, usize)> {
    let mut homes = HashMap::new();
    for (ci, c) in captures.iter().enumerate() {
        for (mi, m) in c.materials.iter().enumerate() {
            if real(&m.name) {
                homes.insert(m.name.clone(), (ci, mi));
            }
        }
    }
    homes
}

/// For every capture, the material indices (into that capture) the wanted
/// models and effects draw with, each taken from the zone that defines it.
/// bo2zm M3: Pack-a-Punch's look. The scripts give an upgraded gun camo
/// 39 on this map (`get_pack_a_punch_weapon_options`; 40 on Mob of the
/// Dead, 45 on Origins); `mp/weaponoptions_zm.csv`'s row for it says it
/// swaps materials (a 1 in column 3) and which of the gun's camo material
/// sets (column 4, counted from 1: the M1911's camo1 -> the zombies camo).
/// Each upgraded gun's view model gets a copy with those swaps,
/// `<model>+camo<n>`.
#[derive(Default)]
struct PapCamo {
    /// Copy -> (the gun model, swaps base material -> camo material, and
    /// the camo's texture tiling: the swap's constants 0 and 1, which BO2's
    /// 3D camo shader multiplies the texture coordinates by, 2 x 2 for the
    /// zombies camo).
    models: BTreeMap<String, (String, Vec<(String, String, [f32; 2])>)>,
    /// Upgraded weapon -> its gun model's copy.
    weapon_models: HashMap<String, String>,
}

/// A float as a half (IEEE binary16), rounded down; texture coordinates.
fn f32_to_half(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x007f_ffff;
    if exp <= 0 {
        return sign;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    sign | ((exp as u16) << 10) | ((mant >> 13) as u16)
}

fn pap_camo(
    captures: &[&ZoneCapture],
    weapon_refs: &HashMap<String, &asset_t6::WeaponRef>,
    camo_index: u32,
) -> PapCamo {
    let mut out = PapCamo::default();
    let want = camo_index.to_string();
    let set = captures
        .iter()
        .rev()
        .flat_map(|c| c.string_tables.iter())
        .find(|t| t.name == "mp/weaponoptions_zm.csv")
        .and_then(|t| {
            (0..t.rows).find_map(|r| {
                let cell = |k: usize| t.cells.get(r * t.columns + k).map_or("", String::as_str);
                (cell(0) == want && cell(1) == "camo" && cell(3) == "1")
                    .then(|| cell(4).parse::<usize>().ok())
                    .flatten()
            })
        })
        .filter(|set| *set > 0);
    let Some(set) = set else {
        return out;
    };
    let camos: HashMap<&str, &asset_t6::WeaponCamoRef> = captures
        .iter()
        .flat_map(|c| c.weapon_camos.iter())
        .map(|c| (c.name.as_str(), c))
        .collect();
    for (name, w) in weapon_refs {
        if !name.contains("_upgraded") {
            continue;
        }
        let Some(list) = camos
            .get(w.camo.as_str())
            .and_then(|camo| camo.material_sets.get(set - 1))
        else {
            continue;
        };
        let swaps: Vec<(String, String, [f32; 2])> = list
            .iter()
            .flat_map(|m| {
                let tile = [m.consts[0], m.consts[1]].map(|t| if t > 0.0 { t } else { 1.0 });
                m.base
                    .iter()
                    .cloned()
                    .zip(m.camo.iter().cloned())
                    .map(move |(b, c)| (b, c, tile))
            })
            .filter(|(base, camo, _)| real(base) && real(camo))
            .collect();
        let Some(gun) = w.gun_models.first().filter(|n| real(n)) else {
            continue;
        };
        if swaps.is_empty() {
            continue;
        }
        let copy = format!("{gun}+camo{camo_index}");
        out.models
            .entry(copy.clone())
            .or_insert((gun.clone(), swaps));
        out.weapon_models.insert(name.clone(), copy);
    }
    out
}

fn wanted_materials(
    captures: &[&ZoneCapture],
    model_names: &BTreeSet<String>,
    homes: &HashMap<String, (usize, usize)>,
) -> Vec<BTreeSet<usize>> {
    let mut wanted: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); captures.len()];
    let mut want = |name: &str| {
        if let Some(&(ci, mi)) = homes.get(name.trim_start_matches(',')) {
            wanted[ci].insert(mi);
        }
    };
    // Models: from the last capture that defines each.
    for name in model_names {
        if let Some((ci, model)) = captures
            .iter()
            .enumerate()
            .rev()
            .find_map(|(ci, c)| c.xmodels.iter().find(|m| &m.name == name).map(|m| (ci, m)))
        {
            for srf in &model.lod0 {
                if let Some(k) = srf.material
                    && let Some(m) = captures[ci].materials.get(k.index)
                {
                    want(&m.name);
                }
            }
        }
    }
    // Effects and tracers: their material names.
    for c in captures {
        for e in c.fx.iter().filter(|e| real(&e.name)) {
            for el in &e.elems {
                for v in &el.visuals {
                    match v {
                        asset_t6::FxVisualRef::Material(n) => want(n),
                        asset_t6::FxVisualRef::Mark([a, b]) => {
                            want(a);
                            want(b);
                        }
                        _ => {}
                    }
                }
            }
        }
        for t in c.tracers.iter().filter(|t| real(&t.name)) {
            want(&t.material);
        }
    }
    wanted
}

/// The sound bank files a map's banks read: each bank asset's own files,
/// the map's location banks (`zmb_<location>*`), and every file those
/// name as a dependency.
fn bank_bases(captures: &[&ZoneCapture], map: &str) -> Vec<String> {
    let mut bases: Vec<String> = Vec::new();
    let mut push = |b: String| {
        if !b.is_empty() && !bases.contains(&b) {
            bases.push(b);
        }
    };
    if let Some(location) = map.strip_prefix("zm_") {
        push(format!("zmb_{location}"));
        push(format!("zmb_{location}_real"));
        push(format!("zmb_{location}_real_intro"));
    }
    for c in captures {
        for b in &c.sound_banks {
            let base = b
                .name
                .rsplit_once('.')
                .map_or(b.name.as_str(), |(base, _)| base);
            push(base.to_owned());
        }
    }
    for dep in ["zmb_common", "zmb_code_post_gfx", "cmn_root"] {
        push(dep.to_owned());
    }
    bases
}

pub(crate) fn load_t6_combat(
    map_path: &Path,
    map_capture: &ZoneCapture,
    materials: &mut MaterialCatalog,
    packs: Option<&PackSet>,
) -> T6Combat {
    let mut report = Vec::new();
    let (pre, post) = zone_paths(map_path);
    let zone_dir = asset_transport::t6_extra::install_zone_dir(map_path);
    let capture = |path: &PathBuf, report: &mut Vec<String>| match asset_t6::capture_zone(path) {
        Ok(c) => Some(c),
        Err(e) => {
            report.push(format!("t6 zone gap: {e}"));
            None
        }
    };
    let pre_caps: Vec<ZoneCapture> = pre.iter().filter_map(|p| capture(p, &mut report)).collect();
    let post_caps: Vec<ZoneCapture> = post
        .iter()
        .filter_map(|p| capture(p, &mut report))
        .collect();
    // bo2mc: the other perks' assets from the other maps' zones, first
    // (the map's own zones win a name they share).
    let perk_caps: Vec<ZoneCapture> = if super::t6_perks::wanted(
        &map_path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
    ) {
        let base: Vec<&ZoneCapture> = pre_caps
            .iter()
            .chain(std::iter::once(map_capture))
            .chain(post_caps.iter())
            .collect();
        super::t6_perks::extra_captures(map_path, &base, &mut report)
    } else {
        Vec::new()
    };
    let captures: Vec<&ZoneCapture> = perk_caps
        .iter()
        .chain(pre_caps.iter())
        .chain(std::iter::once(map_capture))
        .chain(post_caps.iter())
        .collect();
    report.push(format!(
        "t6 combat zones: {}",
        captures
            .iter()
            .map(|c| c.zone.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));

    // Weapons: later zones replace earlier ones of the same name.
    let mut weapon_refs: HashMap<String, &asset_t6::WeaponRef> = HashMap::new();
    let mut weapon_order: Vec<String> = Vec::new();
    for c in &captures {
        for w in c.weapons.iter().filter(|w| real(&w.name)) {
            if weapon_refs.insert(w.name.clone(), w).is_none() {
                weapon_order.push(w.name.clone());
            }
        }
    }
    let pap = pap_camo(&captures, &weapon_refs, 39);
    report.push(format!(
        "t6 Pack-a-Punch camo: {} gun models, {} upgraded weapons",
        pap.models.len(),
        pap.weapon_models.len()
    ));
    let mut model_names: BTreeSet<String> = BTreeSet::new();
    for w in weapon_refs.values() {
        if let Some(gun) = w.gun_models.first().filter(|n| real(n)) {
            model_names.insert(gun.clone());
        }
        let hands = w.def_model(fastfile_t6::layout::WeaponDef::handXModel);
        if real(hands) {
            model_names.insert(hands.to_owned());
        }
        // The magazine and (snipers) the scope, attached to the gun.
        for (_, model, ..) in &w.attach_view_models {
            if real(model) {
                model_names.insert(model.clone());
            }
        }
        // What a launcher shows loaded and fires: the RPG's rocket, the
        // ballistic knife's blade, the knife a gun's melee swings.
        for field in [
            fastfile_t6::layout::WeaponDef::rocketModel,
            fastfile_t6::layout::WeaponDef::projectileModel,
            fastfile_t6::layout::WeaponDef::additionalMeleeModel,
        ] {
            let model = w.def_model(field);
            if real(model) {
                model_names.insert(model.to_owned());
            }
        }
    }
    model_names.insert(T6_PLAYER_ARMS.to_owned());
    for arms in T6_OTHER_ARMS {
        model_names.insert(arms.to_owned());
    }
    // Every other map's player arms (`c_zom_reporter_viewhands` in
    // Buried's mode zone): whatever viewhands its zones carry.
    for c in &captures {
        for m in c.xmodels.iter() {
            if real(&m.name) && m.name.contains("_viewhands") {
                model_names.insert(m.name.clone());
            }
        }
    }
    // bo2zm M3: models the map's own scripts and entities name (perk
    // machines, the box, power-ups, zombie bodies and heads...), drawn as
    // script models and actors from the same catalog.
    let script_models = script_model_names(&captures);
    report.push(format!(
        "t6 script models: {} named by the map's scripts and entities",
        script_models.len()
    ));
    model_names.extend(script_models);
    // bo2mc: the perk machines only the engine names (Deadshot's, PhD's).
    if !perk_caps.is_empty() {
        model_names.extend(super::t6_perks::MODELS.iter().map(|m| (*m).to_owned()));
    }

    // Third-person gun models too (a weapon cannot be given without one).
    let mut world_names: BTreeSet<String> = BTreeSet::new();
    for w in weapon_refs.values() {
        if let Some(world) = w.world_models.first().filter(|n| real(n)) {
            world_names.insert(world.clone());
        }
    }
    // Effect models: shell casings, debris.
    let mut fx_model_names: BTreeSet<String> = BTreeSet::new();
    for c in &captures {
        for e in c.fx.iter().filter(|e| real(&e.name)) {
            for el in &e.elems {
                for v in &el.visuals {
                    if let asset_t6::FxVisualRef::Model(n) = v
                        && real(n)
                    {
                        fx_model_names.insert(n.clone());
                    }
                }
            }
        }
    }
    let all_models: BTreeSet<String> = model_names
        .union(&world_names)
        .cloned()
        .chain(fx_model_names.iter().cloned())
        .collect();

    // Materials the guns, arms and effects draw with, linked per zone.
    let homes = material_homes(&captures);
    let mut wanted = wanted_materials(&captures, &all_models, &homes);
    for (_, swaps) in pap.models.values() {
        for (_, camo, _) in swaps {
            if let Some(&(ci, mi)) = homes.get(camo.trim_start_matches(',')) {
                wanted[ci].insert(mi);
            }
        }
    }
    // Images by name, from the zone that holds their pixels.
    let mut image_homes: HashMap<String, asset_t6::ImageRef> = HashMap::new();
    for c in &captures {
        for img in &c.images {
            if real(&img.name) {
                image_homes.insert(img.name.clone(), img.clone());
            }
        }
    }
    let mut local: Vec<HashMap<usize, usize>> = Vec::with_capacity(captures.len());
    for (c, want) in captures.iter().zip(&wanted) {
        let linked = super::t6_materials::link_materials_with(
            materials,
            c,
            packs,
            want.iter().copied(),
            &c.zone,
            Some(&image_homes),
        );
        report.extend(
            linked
                .report
                .into_iter()
                .map(|line| format!("{} ({}): {line}", "t6 combat", c.zone)),
        );
        local.push(linked.local);
    }

    // A model surface's material, in the catalog: the capture's own row
    // when it linked it, else the row of the zone that defines it.
    let catalog_material = |ci: usize, key: asset_t6::AssetKey| -> Option<usize> {
        if let Some(&i) = local[ci].get(&key.index) {
            return Some(i);
        }
        let name = captures[ci]
            .materials
            .get(key.index)?
            .name
            .trim_start_matches(',');
        let &(hc, hm) = homes.get(name)?;
        local[hc].get(&hm).copied()
    };
    let find_model = |name: &str| {
        captures
            .iter()
            .enumerate()
            .rev()
            .find_map(|(ci, c)| c.xmodels.iter().find(|m| m.name == name).map(|m| (ci, m)))
    };

    // First-person meshes: every gun and the players' arms.
    let mut fpv = FpvMeshBuild::default();
    fpv.set_capture_ns(asset_core::AssetNamespace::T6);
    let mut fpv_missing = Vec::new();
    for name in &model_names {
        let Some((ci, model)) = find_model(name) else {
            fpv_missing.push(name.clone());
            continue;
        };
        match asset_model::capture_model_skel_t6(model, |k| catalog_material(ci, k)) {
            Some(mut skel) => {
                skel.surface_flags = model_surface_flags(captures[ci], model);
                fpv.insert_in(asset_core::AssetNamespace::T6, skel, Some(materials))
            }
            None => fpv_missing.push(format!("{name} (unreadable)")),
        }
    }
    // The upgraded guns' copies, their camo surfaces swapped.
    for (copy, (gun, swaps)) in &pap.models {
        let Some((ci, model)) = find_model(gun) else {
            continue;
        };
        // Each surface's camo tiling, in the order the surfaces are read.
        let tiles: std::cell::RefCell<Vec<[f32; 2]>> = std::cell::RefCell::new(Vec::new());
        let swapped = |k: asset_t6::AssetKey| -> Option<usize> {
            let name = captures[ci]
                .materials
                .get(k.index)
                .map_or("", |m| m.name.trim_start_matches(','));
            match swaps
                .iter()
                .find(|(base, _, _)| base.trim_start_matches(',') == name)
            {
                Some((_, camo, tile)) => {
                    tiles.borrow_mut().push(*tile);
                    let &(hc, hm) = homes.get(camo.trim_start_matches(','))?;
                    local[hc].get(&hm).copied()
                }
                None => {
                    tiles.borrow_mut().push([1.0, 1.0]);
                    catalog_material(ci, k)
                }
            }
        };
        if let Some(mut skel) = asset_model::capture_model_skel_t6(model, swapped) {
            skel.name = copy.clone();
            // The camo surfaces' texture coordinates times the camo's
            // tiling (BO2's camo shader scales them; ours samples as given).
            let tiles = tiles.into_inner();
            if tiles.len() == skel.surface_vertex_ranges.len() {
                for (&(start, count), tile) in skel.surface_vertex_ranges.iter().zip(&tiles) {
                    if *tile == [1.0, 1.0] {
                        continue;
                    }
                    for uv in skel.uvs.iter_mut().skip(start).take(count) {
                        uv[0] *= tile[0];
                        uv[1] *= tile[1];
                    }
                    // The packed rows the gun draws from: two halves, v low
                    // and u high (T6's u-low word turned a half round).
                    for row in skel.packed_vertices.iter_mut().skip(start).take(count) {
                        let tex = u32::from_le_bytes([row[20], row[21], row[22], row[23]]);
                        let v = asset_model::half_to_f32(tex as u16) * tile[1];
                        let u = asset_model::half_to_f32((tex >> 16) as u16) * tile[0];
                        let tex = u32::from(f32_to_half(v)) | (u32::from(f32_to_half(u)) << 16);
                        row[20..24].copy_from_slice(&tex.to_le_bytes());
                    }
                }
            }
            fpv.insert_in(asset_core::AssetNamespace::T6, skel, Some(materials));
        }
    }
    // Third-person gun models.
    let mut world_weapons = WorldWeaponBuild::default();
    world_weapons.set_capture_ns(asset_core::AssetNamespace::T6);
    let mut world_missing = Vec::new();
    for name in &world_names {
        let Some((ci, model)) = find_model(name) else {
            world_missing.push(name.clone());
            continue;
        };
        match asset_model::capture_model_skel_t6(model, |k| catalog_material(ci, k)) {
            Some(skel) => {
                world_weapons.insert_in(asset_core::AssetNamespace::T6, skel, Some(materials))
            }
            None => world_missing.push(format!("{name} (unreadable)")),
        }
    }
    report.push(format!(
        "t6 world gun models: {} built, {} missing {:?}",
        world_weapons.len(),
        world_missing.len(),
        world_missing.iter().take(8).collect::<Vec<_>>()
    ));
    report.push(format!(
        "t6 first-person meshes: {} built, {} missing {:?}",
        fpv.len(),
        fpv_missing.len(),
        fpv_missing.iter().take(8).collect::<Vec<_>>()
    ));

    // Animations: every one, later zones win.
    let mut xanims = XAnimBuild::default();
    xanims.set_capture_ns(asset_core::AssetNamespace::T6);
    let mut anim_n = 0usize;
    for c in &captures {
        for a in &c.xanims {
            anim_n += usize::from(xanims.insert_t6(a));
        }
    }
    report.push(format!(
        "t6 animations: {anim_n} captured, {} kept, {} gaps",
        xanims.len(),
        xanims.capture_gaps
    ));

    // The weapon catalog.
    let mut catalog = WeaponCatalog::default();
    catalog.set_capture_ns(asset_core::AssetNamespace::T6);
    for name in &weapon_order {
        let w = weapon_refs[name];
        // A dual-wield weapon's left-hand weapon (its gun and dw_left_*
        // animations join the right one's viewmodel).
        let left = weapon_refs
            .get(w.def_string(fastfile_t6::layout::WeaponDef::szDualWieldWeaponName))
            .copied();
        catalog.capture_t6(w, left, &pap.weapon_models);
    }
    // bo2zm fix list 2: a rolling grenade rests and rolls on its body: the
    // nearest face of its projectile model's bounds to the model's origin
    // (the frag: a ball of about 1.4 units around it, the fuse above).
    let mut rolling = Vec::new();
    for name in &weapon_order {
        let w = weapon_refs[name];
        if w.def_i32(fastfile_t6::layout::WeaponDef::isRollingGrenade) == 0 {
            continue;
        }
        let model = w.def_model(fastfile_t6::layout::WeaponDef::projectileModel);
        if let Some((_, m)) = find_model(model) {
            let radius = (0..3)
                .flat_map(|a| [m.mins[a].abs(), m.maxs[a].abs()])
                .fold(f32::MAX, f32::min);
            if radius.is_finite() && radius > 0.1 {
                catalog.set_t6_rolling_radius(name, radius);
                rolling.push(format!("{name} {radius:.2}"));
            }
        }
    }
    report.push(format!("t6 rolling grenades: {}", rolling.join(", ")));
    let captured = catalog.len();
    let mut weapons = catalog.into_build();
    weapons.stamp_namespace(asset_core::AssetNamespace::T6);
    report.push(format!(
        "t6 weapons: {captured} captured, {} in the catalog",
        weapons.len()
    ));

    // Effects: every one, later zones win.
    let mut fx = FxCatalog::default();
    fx.set_capture_ns(asset_core::AssetNamespace::T6);
    let mut fx_n = 0usize;
    for c in &captures {
        for e in &c.fx {
            fx_n += usize::from(fx.capture_t6(e));
        }
    }
    report.push(format!("t6 effects: {fx_n} captured, {} kept", fx.len()));
    // bo2zm fix list 3: a gun whose first-person flash is the flash other
    // players see (it names the same effect for both) gets a first-person
    // copy at `T6_FIRST_PERSON_FLASH_SCALE` (our choice; see there). Bullet
    // guns only: the launchers' flashes are launch smoke or already small.
    // A flash made for the first person keeps its own size even without
    // parts drawn with the gun (the Ray Gun's).
    let mut first_person = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for name in &weapon_order {
        let w = weapon_refs[name];
        let view = w.def_fx(fastfile_t6::layout::WeaponDef::viewFlashEffect);
        let world = w.def_fx(fastfile_t6::layout::WeaponDef::worldFlashEffect);
        let bullet = w.def_i32(fastfile_t6::layout::WeaponDef::weapType) == 0;
        if !bullet || view.is_empty() || view != world || !seen.insert(view) {
            continue;
        }
        if let Some((before, after)) = fx.add_t6_first_person_flash(view) {
            first_person.push(format!("{view} {before:.1}->{after:.1}"));
        }
    }
    report.push(format!(
        "t6 first-person flashes made from far-view ones (x{}): {}",
        asset_game::T6_FIRST_PERSON_FLASH_SCALE,
        first_person.join(", ")
    ));
    let mut fx_models = asset_game::FxModelCatalog::default();
    fx_models.set_capture_ns(asset_core::AssetNamespace::T6);
    let mut fx_model_missing = Vec::new();
    for name in &fx_model_names {
        match find_model(name)
            .and_then(|(ci, m)| asset_model::capture_model_skel_t6(m, |k| catalog_material(ci, k)))
        {
            Some(skel) => fx_models.insert_skel(skel, materials),
            None => fx_model_missing.push(name.clone()),
        }
    }
    report.push(format!(
        "t6 effect models: {} built, {} missing {:?}",
        fx_models.len(),
        fx_model_missing.len(),
        &fx_model_missing[..fx_model_missing.len().min(8)]
    ));
    // Tracers: every one, later zones win.
    let mut tracers = asset_game::TracerCatalog::default();
    tracers.set_capture_ns(asset_core::AssetNamespace::T6);
    let mut tracer_n = 0usize;
    for c in &captures {
        for t in &c.tracers {
            tracer_n += usize::from(tracers.capture_t6(t));
        }
    }
    report.push(format!(
        "t6 tracers: {tracer_n} captured, {} kept",
        tracers.len()
    ));
    let impact_fx = captures
        .iter()
        .rev()
        .find_map(|c| c.impact_tables.first())
        .map(OwnedFxImpactTable::from_t6);
    report.push(format!(
        "t6 impact table: {}",
        impact_fx.as_ref().map_or_else(
            || "none".to_owned(),
            |t| format!("{} ({} rows)", t.name, t.row_count())
        )
    ));

    // Sound: every bank's aliases, the clips in the bank files beside the
    // zones (`<install>/sound`).
    let map = map_path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let sound_dir = Some(zone_dir.as_path())
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(|install| install.join("sound"));
    let (map_fx, room_tone) = map_scripts(&captures, &map, &mut report);
    // The language zones beside zone/all (zone/english/en_*.ff): the
    // English strings (hints, perk and power-up names) and the voices'
    // aliases (the announcer's, the players').
    let lang_dir = Some(zone_dir.as_path())
        .and_then(Path::parent)
        .map(|d| d.join("english"));
    let lang_caps: Vec<ZoneCapture> = [
        "code_post_gfx_zm",
        "common_zm",
        "patch_zm",
        "ui_zm",
        "patch_ui_zm",
        map.as_str(),
    ]
    .iter()
    .filter_map(|z| {
        let path = asset_transport::t6_extra::file_in(lang_dir.as_ref()?, &format!("en_{z}.ff"));
        if !path.is_file() {
            return None;
        }
        match asset_t6::capture_zone(&path) {
            Ok(c) => Some(c),
            Err(e) => {
                report.push(format!("t6 language zone gap: {e}"));
                None
            }
        }
    })
    .collect();
    let sound = match sound_dir {
        Some(dir) if dir.is_dir() => {
            let bases = bank_bases(&captures, &map);
            let base_refs: Vec<&str> = bases.iter().map(String::as_str).collect();
            let (index, bank_report) = asset_audio::T6BankIndex::open(&dir, &base_refs);
            report.extend(bank_report);
            let banks: Vec<&asset_t6::SndBankRef> = captures
                .iter()
                .copied()
                .chain(lang_caps.iter())
                .flat_map(|c| c.sound_banks.iter())
                .collect();
            let globals = captures.iter().find_map(|c| c.snd_globals.as_ref());
            let (mut catalog, census) = asset_audio::build_t6_sound_catalog(
                &banks,
                globals,
                &index,
                asset_core::ZoneOwner::intern(&map),
            );
            report.push(format!(
                "t6 sound: {} aliases, {} variants ({} with a clip, {} without); {} falloff curves",
                census.aliases,
                census.variants,
                census.with_clip,
                census.without_clip,
                globals.map_or(0, |g| g.curves.len())
            ));
            catalog.scripted_map_fx = Some(map_fx);
            let mut echoes: Vec<String> = catalog
                .radverbs
                .iter()
                .map(|(name, v)| format!("{name} {v:?}"))
                .collect();
            echoes.sort();
            report.push(format!("t6 sound: {} room echoes: {}", echoes.len(), echoes.join("; ")));
            Ok(catalog)
        }
        _ => Err("Black Ops II sound folder not found beside the zones".to_owned()),
    };
    let models_missing: Vec<String> = fpv_missing
        .iter()
        .chain(world_missing.iter())
        .cloned()
        .collect();
    // bo2mc: the other maps' weapons it loads whole are counted too.
    let census_weapons: Vec<&str> = NUKETOWN_WEAPONS
        .iter()
        .copied()
        .chain(super::t6_perks::WEAPONS.iter().copied().filter(|_| !perk_caps.is_empty()))
        .collect();
    let census_lines = census(
        &captures,
        &census_weapons,
        &weapon_refs,
        &models_missing,
        &xanims,
        &fx,
        impact_fx.as_ref(),
        sound
            .as_ref()
            .ok()
            .and_then(|c| c.scripted_map_fx.as_ref())
            .unwrap_or(&asset_audio::ScriptedMapFx::default()),
        room_tone.as_deref(),
        sound.as_ref().ok(),
    );
    for line in &census_lines {
        diag::info!(World, "{line}");
    }
    report.extend(census_lines);
    let footsteps = captures
        .iter()
        .rev()
        .find(|c| !c.footstep_tables.is_empty())
        .map(|c| c.footstep_tables.clone())
        .unwrap_or_default();
    let mut scripts = crate::T6ScriptSet::default();
    for c in &captures {
        for (name, bytes) in &c.raw_files {
            if let Some(n) = name.strip_prefix("script:")
                && !n.starts_with(',')
                && n.ends_with(".gsc")
            {
                scripts.objects.push(bytes.clone());
            }
        }
        for t in &c.string_tables {
            if real(&t.name) {
                scripts
                    .tables
                    .push((t.name.clone(), t.columns, t.rows, t.cells.clone()));
            }
        }
        scripts.entities.extend(c.map_ents.iter().cloned());
        for d in c.destructibles.iter().filter(|d| real(&d.name)) {
            scripts.destructibles.retain(|o| o.name != d.name);
            scripts.destructibles.push(t5_destructible(d));
        }
        for z in &c.zbarriers {
            scripts.zbarriers.retain(|o| !o.name.eq_ignore_ascii_case(&z.name));
            scripts.zbarriers.push(z.clone());
        }
        if !c.path_nodes.is_empty() {
            scripts.path_nodes = c.path_nodes.clone();
        }
        for (name, bytes) in &c.raw_files {
            let lower = name.to_ascii_lowercase();
            if let Some(shock) = lower.strip_prefix("shock/").and_then(|n| n.strip_suffix(".shock")) {
                scripts.shocks.retain(|(n, _)| n != shock);
                scripts
                    .shocks
                    .push((shock.to_owned(), String::from_utf8_lossy(bytes).into_owned()));
            }
            if name.starts_with("animstatedefs/") && name.ends_with(".asd") {
                let text = String::from_utf8_lossy(bytes).into_owned();
                scripts.animstatedefs.retain(|(n, _)| n != name);
                scripts.animstatedefs.push((name.clone(), text));
            }
        }
    }
    // Every animation's timing, root motion and notetracks; later zones win.
    let mut anim_facts: HashMap<String, crate::T6AnimFacts> = HashMap::new();
    for c in &captures {
        for a in c.xanims.iter().filter(|a| real(&a.name)) {
            anim_facts.insert(
                a.name.clone(),
                crate::T6AnimFacts {
                    name: a.name.clone(),
                    numframes: a.numframes,
                    framerate: a.framerate,
                    looping: a.looping,
                    delta_trans: a.delta_trans.clone(),
                    notifies: a.notifies.clone(),
                },
            );
        }
    }
    scripts.anims = anim_facts.into_values().collect();
    let mut strings: HashMap<String, String> = HashMap::new();
    // bo2mc: the other maps' strings first (the perks' hints); the map's
    // own win a name.
    let perk_lang = if perk_caps.is_empty() {
        Vec::new()
    } else {
        super::t6_perks::extra_lang_captures(lang_dir.as_deref(), &mut report)
    };
    for c in perk_lang.iter().chain(&lang_caps) {
        for (k, v) in &c.localize {
            strings.insert(k.clone(), v.clone());
        }
    }
    // Strings a mod zone carries (Zombies Declassified's mod_load), where
    // no language zone has the key.
    for c in &captures {
        for (k, v) in &c.localize {
            strings.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    scripts.strings = strings.into_iter().collect();
    // Every sound alias the banks hold (the scripts' soundexists).
    if let Ok(cat) = &sound {
        scripts.sound_aliases = cat
            .sounds
            .iter()
            .map(|s| s.name.to_ascii_lowercase())
            .collect();
    }
    report.push(format!(
        "t6 scripts: {} server script objects, {} string tables, {} entity strings, {} path nodes, {} animations, {} animstatedefs, {} English strings, {} barrier types",
        scripts.objects.len(),
        scripts.tables.len(),
        scripts.entities.len(),
        scripts.path_nodes.len(),
        scripts.anims.len(),
        scripts.animstatedefs.len(),
        scripts.strings.len(),
        scripts.zbarriers.len()
    ));
    let ui_script_list = ui_scripts(&captures);
    let ui_names = ui_strings(&ui_script_list);
    let hud_icons = hud_icons(&captures, packs, &ui_names, &mut report);
    report.push(format!("t6 ui: {} scripts", ui_script_list.len()));
    let mut ui = crate::T6Ui {
        scripts: ui_script_list,
        strings: std::sync::Arc::new(scripts.strings.clone()),
        tables: std::sync::Arc::new(scripts.tables.clone()),
        game_settings: std::sync::Arc::new(zm_game_settings(&captures)),
    };
    // bo2mc: the Minecraft map's rows and strings for the game's own HUD
    // (its scoreboard names the map).
    let mut mc = Vec::new();
    crate::bo2mc_frontend::add_minecraft_rows(&mut ui, &mut mc);
    report.extend(mc.into_iter().map(|l| format!("bo2mc hud: {l}")));
    // The font sheet's pixels are in the language's own image packs.
    let lang_packs = Some(zone_dir.as_path())
        .and_then(Path::parent)
        .map(|d| d.join("english"))
        .and_then(|d| asset_t6::PackSet::open_dir(&d).ok());
    let hud_fonts = hud_fonts(&lang_caps, lang_packs.as_ref().or(packs), &mut report);
    // bo2zm: the bullet penetration table (patch_zm's `info/bullet_penetration_mp`).
    let pen_table = captures
        .iter()
        .flat_map(|c| c.raw_files.iter())
        .filter_map(|(name, bytes)| asset_game::capture_pen_table(name, bytes, false))
        .last();
    report.push(format!(
        "t6 bullet penetration table: {}",
        if pen_table.is_some() {
            "loaded"
        } else {
            "missing (bullets stop at the first hit)"
        }
    ));
    T6Combat {
        weapons,
        fpv,
        world_weapons,
        xanims,
        fx,
        fx_models,
        tracers,
        impact_fx,
        sound,
        footsteps,
        room_tone,
        scripts,
        hud_icons,
        hud_fonts,
        ui,
        pen_table,
        report,
    }
}

/// A BO2 `DestructibleDef` in the shape the server's destructibles run on
/// (Black Ops' own, the same layout).
fn t5_destructible(d: &asset_t6::m2::DestructibleRef) -> xmodel_runtime::T5DestructibleDef {
    xmodel_runtime::T5DestructibleDef {
        name: d.name.clone(),
        model: d.model.clone(),
        pieces: d
            .pieces
            .iter()
            .map(|p| xmodel_runtime::T5DestructiblePiece {
                stages: p.stages.clone().map(|st| xmodel_runtime::T5DestructibleStage {
                    show_bone: st.show_bone,
                    break_health: st.break_health,
                    max_time: st.max_time,
                    flags: st.flags,
                    break_effect: st.break_effect,
                    break_sound: st.break_sound,
                    break_notify: st.break_notify,
                    loop_sound: st.loop_sound,
                    has_phys_preset: st.has_phys_preset,
                    spawn_models: st.spawn_models,
                }),
                parent_piece: p.parent_piece,
                parent_damage_percent: p.parent_damage_percent,
                bullet_damage_scale: p.bullet_damage_scale,
                explosive_damage_scale: p.explosive_damage_scale,
                health: p.health,
                // BO2's piece launch is not read yet.
                launch: None,
                hide_bones: p.hide_bones,
            })
            .collect(),
        client_only: d.client_only,
    }
}
