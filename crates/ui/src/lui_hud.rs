//! bo2zm M4: Black Ops II's own Zombies HUD, run from the game's UI
//! scripts: the HavokScript VM (`hks_t6`) loads LUI and CoD's base, then
//! `ui_mp/t6/hud.lua`; the root opens its `HUD` menu as the engine does,
//! and the game's values reach it as the engine's events
//! (`hud_update_rounds_played`, ...). Each frame its elements (pictures and
//! text, in root units 720 high) are drawn as Bevy UI over the game.
//!
//! On unless `BO2ZM_LUI=0`; the hand-made HUD in `zm_hud` keeps what the
//! engine itself draws in BO2 (the use hint, script HUD text, the hurt
//! overlay).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::ClientSet;
use hks_t6::host::{Drawn, Host};
use hks_t6::value::Value;
use net::{LocalPresentClient, PresentedSnapshot};

use crate::bo2_font::Bo2Fonts;
use crate::layers::{UiLayer, UiLayerVisibility};

/// Font metrics the scripts' text measure reads: font name -> (pixel
/// height, advance per letter, space advance).
type Metrics = Rc<RefCell<HashMap<String, (f32, HashMap<char, f32>, f32)>>>;

/// The scripts' font files and the names `Bo2Fonts` keeps them by
/// (`ui/t6/codbase.lua`'s `fonts/<res>/<file>`).
const FONT_FILES: [(&str, &str); 7] = [
    ("normalFont", "Default"),
    ("smallFont", "Condensed"),
    ("bigFont", "Big"),
    ("extraBigFont", "Morris"),
    ("extraSmallFont", "ExtraSmall"),
    ("italicFont", "Italic"),
    ("smallItalicFont", "SmallItalic"),
];

fn font_name(file: &str) -> &'static str {
    let stem = file.rsplit('/').next().unwrap_or(file);
    FONT_FILES
        .iter()
        .find(|(f, _)| f.eq_ignore_ascii_case(stem))
        .map_or("Default", |(_, n)| n)
}

/// On unless `BO2ZM_LUI=0` (then the hand-made HUD in `zm_hud` draws it all).
pub(crate) fn lui_enabled() -> bool {
    std::env::var("BO2ZM_LUI").map_or(true, |v| v != "0")
}

struct LuiHud {
    host: Host,
    metrics: Metrics,
    /// bo2zm M4: Black Ops II's front end (main menu, lobby), not the HUD.
    frontend: bool,
    /// When the front end last had its lobby update (LUI ms).
    lobby_update_at: f64,
    /// Its loading screen is up (a map loads).
    loading_open: bool,
    /// The lobby it opens on (back from a game), else the main menu.
    return_menu: Option<&'static str>,
    /// The left stick's menu step (its direction and when it last stepped).
    stick: Option<(&'static str, f64)>,
    /// BO2's scoreboard is open (Tab held, or the game-over shot).
    board_open: bool,
    /// The scripts it was built from (their count).
    built_from: usize,
    opened: bool,
    /// The mouse in root units, as last sent.
    mouse_at: Option<Vec2>,
    /// A bind row asked for a key and the capture has not finished.
    binding: bool,
    /// His settings' revision last written into BO2's names, and what.
    settings_rev: Option<u64>,
    written: Vec<(String, String)>,
    /// BO2ZM_LUI_DUMP: how many elements the last dump listed.
    dumped: usize,
    /// BO2ZM_LUI_PAUSE_AT's progress, and when the HUD opened.
    test_step: usize,
    /// BO2ZM_LUI_ASKED's one report went out.
    asked_logged: bool,
    /// BO2ZM_LUI_OPEN_MENU's event went out.
    open_menu_sent: bool,
    opened_at: f64,
    /// The values last sent (events go out when one changes).
    sent: Option<Values>,
    aspect: f32,
}

/// The HUD's VM lives on the main thread (its values are `Rc`); beside it,
/// the front end's lobby while a game it started runs.
#[derive(Default)]
pub(crate) struct LuiHudSlot(Option<LuiHud>, Option<FrontState>);

/// bo2zm M4: where the front end was when its game started (the playlist,
/// game and session modes, the playlist filter): after the game BO2 goes
/// back to that lobby.
struct FrontState {
    playlist: usize,
    game_modes: Vec<f32>,
    session_modes: Vec<f32>,
    filter: Option<String>,
    /// bo2mc: the map picked (its ui dvars and the profile's last map), so
    /// the lobby after a game shows that map's card, not the first map's.
    map: Vec<(String, String)>,
    last_map: Option<String>,
}

/// bo2mc: the dvars that name the picked map (kept across a game).
const MAP_DVARS: [&str; 4] = [
    "ui_mapname",
    "ui_zm_mapstartlocation",
    "ui_gametype",
    "ui_zm_gamemodegroup",
];

#[derive(Component)]
struct LuiRoot;

/// What BO2's menus take in and hand out: his keys and mouse, the
/// classic-menu handshake, the pause, and the engine calls they make.
#[derive(bevy::ecs::system::SystemParam)]
struct LuiIo<'w, 's> {
    /// His monitors (BO2's display options count them).
    monitors: Query<'w, 's, &'static bevy::window::Monitor>,
    /// His controller (Start opens the pause menu; A, B, the d-pad and the
    /// shoulders work its menus).
    pads: Query<'w, 's, &'static bevy::input::gamepad::Gamepad>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    menus: ResMut<'w, frame::LuiMenus>,
    paused: ResMut<'w, frame::GamePaused>,
    exec: MessageWriter<'w, frame::UiExecCommand>,
    sounds: MessageWriter<'w, frame::UiPlaySound>,
    /// The front end's music (`Engine.PlayMenuMusic`), stopped at Start.
    music: MessageWriter<'w, frame::UiPlayMusic>,
    music_stop: MessageWriter<'w, frame::UiStopMusic>,
    inbox: Option<ResMut<'w, net::ClientActionInbox>>,
    ids: Option<ResMut<'w, net::ActionRequestIds>>,
    role: Option<Res<'w, frame::RuntimeRole>>,
    /// The key capture BO2's bind rows start (`Engine.BindCommand`).
    bind: MessageWriter<'w, frame::UiBindRequest>,
    capture: Res<'w, frame::UiBindingCapture>,
    /// His settings, shown and changed by BO2's Settings and Controls.
    settings: ResMut<'w, frame::GameSettings>,
    /// How much the menus blur the world (`Engine.BlurWorld`).
    blur: ResMut<'w, frame::WorldBlur>,
    /// His last input (pad or keys) and his pad's kind: BO2's prompts.
    devices: Option<Res<'w, frame::InputDevices>>,
    /// bo2mc: a test run's pointer (`cursor x y`), else the window's.
    test_cursor: Res<'w, frame::TestCursor>,
}

/// The resolutions BO2's video mode offers (his own first if it is not
/// one of them).
const RESOLUTIONS: [&str; 6] = [
    "1280x720",
    "1366x768",
    "1600x900",
    "1920x1080",
    "2560x1440",
    "3840x2160",
];

/// His settings as BO2's profile and hardware-profile names.
fn profile_of(s: &frame::GameSettings) -> Vec<(String, String)> {
    let b = |v: bool| if v { "1" } else { "0" }.to_owned();
    vec![
        ("r_mode".into(), s.resolution.to_string()),
        ("r_fullscreen".into(), b(s.fullscreen)),
        ("r_vsync".into(), b(s.vsync)),
        ("cg_fov_default".into(), format!("{}", s.fov.round())),
        ("sm_enable".into(), b(s.shadows)),
        // The Shadows row (off -1, low 0 .. high 2): ours are on or off.
        (
            "sm_spotQuality".into(),
            if s.shadows { "2" } else { "-1" }.to_owned(),
        ),
        ("cg_mature".into(), b(s.mature)),
        ("ai_corpseCount".into(), s.corpses.to_string()),
        (
            "cg_drawFPS".into(),
            if s.draw_fps { "Simple" } else { "Off" }.to_owned(),
        ),
        ("com_maxfps".into(), s.max_fps.to_string()),
        ("r_fxaa".into(), b(s.fxaa)),
        ("r_aaSamples".into(), s.aa_samples.to_string()),
        ("r_texFilterQuality".into(), s.tex_filter.to_string()),
        (
            "r_picmip".into(),
            if s.bo2_fast && s.tex_quality < 0 {
                3
            } else {
                s.tex_quality
            }
            .to_string(),
        ),
        // BO2 keeps whether texture quality was picked by hand (1) or is
        // automatic (0); its menu reads it.
        (
            "r_picmip_manual".into(),
            b(s.tex_quality >= 0 || s.bo2_fast),
        ),
        ("snd_menu_master".into(), format!("{:.2}", s.master_volume)),
        ("snd_menu_music".into(), format!("{:.2}", s.music_volume)),
        ("snd_menu_sfx".into(), format!("{:.2}", s.sfx_volume)),
        ("snd_menu_voice".into(), format!("{:.2}", s.voice_volume)),
        (
            "snd_menu_cinematic".into(),
            format!("{:.2}", s.cinematic_volume),
        ),
        // PC Controls' mouse slider (0.01 to 30, default 5: our scale).
        ("mouseSensitivity".into(), format!("{:.2}", s.sensitivity)),
        // The pad's look sensitivity (BO2 0.4 to 4, default 1; the engine's
        // look sensitivity is on BO2's scale since the 10-01 presets).
        ("input_viewSensitivity".into(), format!("{:.2}", s.pad_look_sensitivity())),
        // PC Controls' invert mouse (yes -0.022, no 0.022).
        (
            "m_pitch".into(),
            if s.invert_mouse { "-0.022" } else { "0.022" }.to_owned(),
        ),
        // The pad's rows: look inversion, vibration.
        ("input_invertpitch".into(), b(s.pad_invert)),
        ("gpad_rumble".into(), b(s.pad_vibration)),
        // The brightness screen's slider (0.5 to 1.5): his brightness.
        ("r_gamma".into(), format!("{:.2}", 1.0 + s.brightness * 2.5)),
    ]
}

/// One of BO2's names, changed by its menus, into his settings.
fn apply_profile(s: &mut frame::GameSettings, key: &str, value: &str) {
    let on = value.trim() != "0" && !value.trim().is_empty();
    let num = value.trim().parse::<f32>().ok();
    match key {
        "r_mode" => {
            if let Some((w, h)) = value.trim().split_once('x')
                && let (Ok(w), Ok(h)) = (w.parse(), h.parse())
            {
                s.resolution = frame::DisplayResolution::new(w, h);
            }
        }
        "r_fullscreen" => s.fullscreen = on,
        "r_vsync" => s.vsync = on,
        "cg_fov_default" => s.fov = num.unwrap_or(s.fov),
        "sm_enable" => s.shadows = on,
        "snd_menu_master" => s.master_volume = num.unwrap_or(s.master_volume).clamp(0.0, 1.0),
        "snd_menu_music" => s.music_volume = num.unwrap_or(s.music_volume).clamp(0.0, 1.0),
        "snd_menu_sfx" => s.sfx_volume = num.unwrap_or(s.sfx_volume).clamp(0.0, 1.0),
        "snd_menu_voice" => s.voice_volume = num.unwrap_or(s.voice_volume).clamp(0.0, 1.0),
        "snd_menu_cinematic" => {
            s.cinematic_volume = num.unwrap_or(s.cinematic_volume).clamp(0.0, 1.0)
        }
        "mouseSensitivity" => s.sensitivity = num.unwrap_or(s.sensitivity).clamp(0.1, 30.0),
        "input_viewSensitivity" => {
            // bo2zm: the slider sets a custom sensitivity (no preset), kept
            // in the range it had before the presets (1 to 10 on the old x3
            // scale).
            s.pad_custom_sensitivity =
                num.map_or(s.pad_look_sensitivity(), |v| v.clamp(1.0 / 3.0, 10.0 / 3.0));
            s.pad_sensitivity_preset = 0;
        }
        "m_pitch" => s.invert_mouse = num.is_some_and(|v| v < 0.0),
        "input_invertpitch" => s.pad_invert = on,
        "gpad_rumble" => s.pad_vibration = on,
        "sm_spotQuality" => s.shadows = num.is_none_or(|v| v >= 0.0),
        "cg_mature" => s.mature = on,
        "ai_corpseCount" => s.corpses = num.map_or(s.corpses, |v| v as u8),
        "cg_drawFPS" => {
            s.draw_fps = !value.trim().eq_ignore_ascii_case("off") && value.trim() != "0"
        }
        "com_maxfps" => s.max_fps = num.map_or(s.max_fps, |v| v.max(0.0) as u16),
        "r_fxaa" => s.fxaa = on,
        "r_aaSamples" => s.aa_samples = num.map_or(s.aa_samples, |v| v as u8),
        "r_texFilterQuality" => {
            s.tex_filter = num.map_or(s.tex_filter, |v| v.clamp(0.0, 2.0) as u8)
        }
        "r_picmip_manual" if !on => {
            s.tex_quality = -1;
            s.bo2_fast = false;
        }
        // Texture quality: low is the fast quality (from the next start).
        "r_picmip" => {
            if let Some(v) = num {
                s.tex_quality = v.clamp(-1.0, 3.0) as i8;
                s.bo2_fast = s.tex_quality == 3;
            }
        }
        "r_gamma" => {
            if let Some(g) = num {
                s.brightness = ((g - 1.0) / 2.5).clamp(-0.2, 0.2);
            }
        }
        _ => {}
    }
}

/// His settings <-> BO2's names: his go in when they change (not by the
/// menus), and a name the menus changed comes back into them.
fn settings_bridge(hud: &mut LuiHud, settings: &mut frame::GameSettings) {
    if hud.settings_rev != Some(settings.revision) {
        // What the game's own code follows straight away: gore and corpses
        // (the game's rules), texture filtering (the samplers).
        sim::set_t6_profile(settings.mature, settings.corpses);
        asset_core::t6_set_aniso(match settings.tex_filter {
            0 => 1,
            1 => 4,
            _ => 16,
        });
        let pairs = profile_of(settings);
        let mut v = hud.host.values.borrow_mut();
        v.player_name = settings.player_name.clone();
        for (k, x) in &pairs {
            v.profile.insert(k.clone(), x.clone());
            v.dvars.insert(k.clone(), x.clone());
        }
        let mut modes: Vec<String> = RESOLUTIONS.iter().map(|r| (*r).to_owned()).collect();
        let mine = settings.resolution.to_string();
        if !modes.contains(&mine) {
            modes.insert(0, mine);
        }
        v.enums.insert("r_mode".to_owned(), modes);
        hud.written = pairs;
        hud.settings_rev = Some(settings.revision);
        return;
    }
    let changed: Vec<(String, String)> = {
        let v = hud.host.values.borrow();
        hud.written
            .iter()
            .filter_map(|(k, old)| {
                let now = v
                    .profile
                    .get(k)
                    .filter(|x| *x != old)
                    .or_else(|| v.dvars.get(k).filter(|x| *x != old))?;
                Some((k.clone(), now.clone()))
            })
            .collect()
    };
    if changed.is_empty() {
        return;
    }
    for (k, x) in &changed {
        diag::info!(Ui, "bo2zm lui: setting {k} = {x}");
        apply_profile(settings, k, x);
    }
    settings.touch();
}

/// His keys as BO2's pad buttons (what its menus listen for).
const MENU_KEYS: [(KeyCode, &str); 7] = [
    (KeyCode::Escape, "secondary"),
    (KeyCode::Enter, "primary"),
    (KeyCode::NumpadEnter, "primary"),
    (KeyCode::ArrowUp, "up"),
    (KeyCode::ArrowDown, "down"),
    (KeyCode::ArrowLeft, "left"),
    (KeyCode::ArrowRight, "right"),
];

/// Keys and mouse into BO2's menus: Esc opens its pause menu (the HUD's
/// `open_ingame_menu`, menu `class`); with a menu open, keys become pad
/// buttons and the mouse its pointer (root units).
fn menu_input(
    hud: &mut LuiHud,
    io: &LuiIo<'_, '_>,
    console_open: bool,
    cursor: Option<Vec2>,
    scale: f32,
) {
    if console_open {
        return;
    }
    use bevy::input::gamepad::GamepadButton as P;
    let pad_start = io.pads.iter().any(|p| p.just_pressed(P::Start));
    // (The front end always has a menu up, inside its black cover.)
    if !hud.frontend && hud.host.open_menus().is_empty() {
        if io.keys.just_pressed(KeyCode::Escape) || pad_start {
            hud.host
                .root_event("open_ingame_menu", &[("menuName", Value::str("class"))]);
        }
        return;
    }
    for (key, button) in MENU_KEYS {
        if io.keys.just_pressed(key) {
            if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                diag::info!(Ui, "bo2zm lui key {key:?} -> {button}");
                for line in hud.host.menu_report() {
                    diag::info!(Ui, "bo2zm lui menu {line}");
                }
            }
            hud.host.button(button, true);
        } else if io.keys.just_released(key) {
            hud.host.button(button, false);
        }
    }
    // The pad's buttons by BO2's names (Start backs out like Esc).
    for pad in &io.pads {
        for (b, name) in [
            (P::South, "primary"),
            (P::East, "secondary"),
            (P::Start, "secondary"),
            (P::West, "alt1"),
            (P::North, "alt2"),
            (P::DPadUp, "up"),
            (P::DPadDown, "down"),
            (P::DPadLeft, "left"),
            (P::DPadRight, "right"),
            (P::LeftTrigger, "shoulderl"),
            (P::RightTrigger, "shoulderr"),
        ] {
            if pad.just_pressed(b) {
                hud.host.button(name, true);
            } else if pad.just_released(b) {
                hud.host.button(name, false);
            }
        }
    }
    // The left stick moves through the menus like the d-pad (one step per
    // push; it repeats while held, as BO2's menus do).
    let stick = io.pads.iter().find_map(|pad| {
        let x = pad
            .get(bevy::input::gamepad::GamepadAxis::LeftStickX)
            .unwrap_or(0.0);
        let y = pad
            .get(bevy::input::gamepad::GamepadAxis::LeftStickY)
            .unwrap_or(0.0);
        let dir = if y > 0.6 {
            Some("up")
        } else if y < -0.6 {
            Some("down")
        } else if x < -0.6 {
            Some("left")
        } else if x > 0.6 {
            Some("right")
        } else {
            None
        };
        dir.or((x.abs() > 0.3 || y.abs() > 0.3).then_some("hold"))
    });
    let now = hud.host.now_ms();
    match stick {
        Some(dir) if dir != "hold" => {
            let fresh = hud.stick.is_none_or(|(d, _)| d != dir);
            let repeat = hud
                .stick
                .is_some_and(|(d, at)| d == dir && now - at > 220.0);
            if fresh || repeat {
                // The first repeat waits longer (a held stick, not a flick).
                let at = if fresh { now + 230.0 } else { now };
                hud.stick = Some((dir, at));
                hud.host.button(dir, true);
                hud.host.button(dir, false);
            }
        }
        Some(_) => {}
        None => hud.stick = None,
    }
    if let Some(c) = cursor {
        let p = c / scale.max(1e-3);
        if hud.mouse_at != Some(p) {
            hud.mouse_at = Some(p);
            hud.host.mouse("mousemove", p.x, p.y, "");
        }
        for (b, name) in [(MouseButton::Left, "left"), (MouseButton::Right, "right")] {
            if io.mouse.just_pressed(b) {
                hud.host.mouse("mousedown", p.x, p.y, name);
            }
            if io.mouse.just_released(b) {
                hud.host.mouse("mouseup", p.x, p.y, name);
            }
        }
    }
}

/// Carry out what the menus asked of the engine.
fn engine_calls(hud: &mut LuiHud, io: &mut LuiIo<'_, '_>, local: Option<&LocalPresentClient>) {
    for call in hud.host.take_calls() {
        diag::info!(Ui, "bo2zm lui engine call: {call:?}");
        match call {
            hks_t6::host::EngineCall::MenuResponse(menu, response) => {
                let (Some(inbox), Some(ids), Some(local)) =
                    (io.inbox.as_mut(), io.ids.as_mut(), local)
                else {
                    continue;
                };
                let (Some(m), Some(r)) = (
                    sim::menu_response_field(&menu),
                    sim::menu_response_field(&response),
                ) else {
                    diag::warn!(
                        Ui,
                        "bo2zm lui: menu response {menu} {response} does not fit"
                    );
                    continue;
                };
                let request_id = ids.allocate();
                if let Err(e) = inbox.push(
                    local.0,
                    sim::ClientAction::MenuResponse {
                        request_id,
                        menu: m,
                        response: r,
                    },
                ) {
                    diag::warn!(Ui, "bo2zm lui: menu response not queued: {e}");
                }
            }
            hks_t6::host::EngineCall::Exec(text) => {
                // BO2's commands as ours; the ones with nothing here to do
                // (pad rumble, party, on-screen keyboard) are dropped.
                let word = text.split_whitespace().next().unwrap_or("");
                // A dvar the menus set as a command (`ui_mapname
                // zm_transit`, Custom Games' default): kept, the map one
                // this game plays.
                if (word.starts_with("ui_") || word.starts_with("party_"))
                    && let Some(value) = text
                        .split_once(' ')
                        .map(|(_, v)| v.trim())
                        .filter(|v| !v.is_empty())
                {
                    let mut v = hud.host.values.borrow_mut();
                    let value = if word.eq_ignore_ascii_case("ui_mapname")
                        && !v.maps.iter().any(|m| m == value)
                    {
                        v.maps.first().cloned().unwrap_or_else(|| value.to_owned())
                    } else {
                        value.to_owned()
                    };
                    let key = v
                        .dvars
                        .keys()
                        .find(|k| k.eq_ignore_ascii_case(word))
                        .cloned()
                        .unwrap_or_else(|| word.to_owned());
                    v.dvars.insert(key, value);
                    continue;
                }
                let ours = match word {
                    "fast_restart" | "map_restart" => Some("map_restart".to_owned()),
                    // bo2mc (his ask 10-05): Exit Game inside a match goes back
                    // to the main menu, never out of the program.
                    "quit" if !hud.frontend && frame::bo2_frontend().is_some() => {
                        Some("disconnect".to_owned())
                    }
                    "disconnect" | "quit" => Some(word.to_owned()),
                    // bo2zm M4: the playlist's party starts (Solo, Public
                    // Match): its rule is that everyone readies up (Start
                    // Match), as the engine sets from the playlist.
                    "xstartpartyhost" | "xstartparty" if hud.frontend => {
                        hud.host
                            .values
                            .borrow_mut()
                            .dvars
                            .insert("party_readyPercentRequired".to_owned(), "1".to_owned());
                        None
                    }
                    // bo2zm M4: the front end's Start Match (the party is
                    // ready): load the map the game started for.
                    "xpartyready" | "xpartygo"
                        if hud.frontend
                            && (word == "xpartygo"
                                || text.split_whitespace().nth(1) == Some("1")) =>
                    {
                        io.music_stop.write(frame::UiStopMusic);
                        // The game's settings as the menus left them (Custom
                        // Games' starting round, difficulty, magic,
                        // headshots only; dog rounds stay off here).
                        let v = hud.host.values.borrow();
                        let picked = ["startRound", "zmDifficulty", "magic", "headshotsonly"]
                            .iter()
                            .filter_map(|k| {
                                v.settings.get(*k).map(|x| ((*k).to_owned(), *x as i32))
                            })
                            .collect();
                        drop(v);
                        sim::set_t6_game_settings(picked);
                        // bo2mc: MINECRAFT loads Nuketown's game on the
                        // Minecraft world, NUKETOWN plain Nuketown (one
                        // switch, set before the map loads).
                        let map = hud.host.picked_map().unwrap_or_default();
                        let minecraft = map == assets::bo2mc_frontend::MAP;
                        sim::bo2mc::set_enabled(minecraft);
                        diag::info!(Ui, "bo2mc: Start Match on {map} (Minecraft world {minecraft})");
                        frame::bo2_frontend().map(|zone| format!("map {zone}"))
                    }
                    _ => None,
                };
                match ours {
                    Some(text) => {
                        io.exec.write(frame::UiExecCommand { text });
                    }
                    None => diag::info!(
                        Ui,
                        "bo2zm lui: engine command `{text}` has nothing to do here"
                    ),
                }
            }
            hks_t6::host::EngineCall::PlaySound(alias) => {
                io.sounds.write(frame::UiPlaySound { alias });
            }
            hks_t6::host::EngineCall::PlayMusic(alias) => {
                io.music.write(frame::UiPlayMusic { alias });
            }
            hks_t6::host::EngineCall::BlurWorld(amount) => {
                io.blur.0 = amount.max(0.0);
            }
            hks_t6::host::EngineCall::BindCommand(command, _index) => {
                // The game's own capture binds the next key he presses.
                io.bind.write(frame::UiBindRequest { command });
                hud.binding = true;
            }
        }
    }
}

/// One drawn element (its LUI id) and what it shows (rebuilt on change;
/// its alpha is set in place).
#[derive(Component)]
struct LuiNode {
    id: usize,
    shows: String,
    alpha: f32,
}

fn build(ui: &assets::T6Ui, frontend: bool) -> LuiHud {
    let metrics: Metrics = Rc::default();
    let m = metrics.clone();
    let measure = Box::new(move |text: &str, font: &str, h: f32| {
        let m = m.borrow();
        let Some((px, adv, space)) = m.get(font_name(font)) else {
            return text.chars().count() as f32 * h * 0.5;
        };
        let w: f32 = text
            .chars()
            .map(|c| {
                if c == ' ' {
                    *space
                } else {
                    adv.get(&c).copied().unwrap_or(*space)
                }
            })
            .sum();
        w * h / px.max(1.0)
    });
    let scripts = ui
        .scripts
        .iter()
        .map(|(n, b)| (n.clone(), b.to_vec()))
        .collect();
    let mut host = Host::new(scripts, measure);
    host.load_base();
    {
        let mut v = host.values.borrow_mut();
        v.localize = ui
            .strings
            .iter()
            .map(|(k, t)| (k.to_ascii_uppercase(), t.clone()))
            .collect();
        diag::info!(
            Ui,
            "bo2zm lui: string tables {}",
            ui.tables
                .iter()
                .map(|t| t.0.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
        for (name, cols, _rows, cells) in ui.tables.iter() {
            let rows = cells
                .chunks((*cols).max(1))
                .map(<[String]>::to_vec)
                .collect();
            v.tables.insert(name.clone(), rows);
        }
        // The game settings' defaults (Custom Games' rows start on them).
        for (name, value) in ui.game_settings.iter() {
            v.settings.insert(name.clone(), *value);
        }
    }
    zombies_values(&mut host);
    host.values.borrow_mut().front_end = frontend;
    // bo2mc: MINECRAFT beside NUKETOWN in the map pick (when the front end
    // has its rows), picked when the menu first opens.
    if frontend {
        let mut v = host.values.borrow_mut();
        let has = v.tables.get("zm/mapstable.csv").is_some_and(|rows| {
            rows.iter()
                .any(|r| r.first().is_some_and(|m| m == assets::bo2mc_frontend::MAP))
        });
        if has {
            v.maps = vec![
                assets::bo2mc_frontend::MAP.to_owned(),
                "zm_nuked".to_owned(),
            ];
            for (k, x) in [
                ("ui_mapname", assets::bo2mc_frontend::MAP),
                ("ui_zm_mapstartlocation", assets::bo2mc_frontend::LOCATION),
            ] {
                v.dvars.insert(k.to_owned(), x.to_owned());
            }
            v.profile
                .insert("map_zm".to_owned(), assets::bo2mc_frontend::MAP.to_owned());
        }
    }
    // BO2's front end (its own script state, as BO2 keeps it apart from
    // the game's HUD) out of a game; test aid: BO2ZM_LUI_FRONTEND=1 builds
    // it over a game too.
    if frontend || std::env::var_os("BO2ZM_LUI_FRONTEND").is_some() {
        host.require("T6.Main");
        // Its loading screen (the engine opens `Loading` while a map loads).
        host.require("T6.HUD.Loading");
    } else {
        host.require("T6.HUD");
    }
    // Test aid: BO2ZM_LUI_REQUIRE=<module>[,<module>] loads more of BO2's UI
    // (the front end: `T6.Main`).
    if let Ok(more) = std::env::var("BO2ZM_LUI_REQUIRE") {
        for m in more.split(',').filter(|m| !m.is_empty()) {
            host.require(m.trim());
        }
    }
    LuiHud {
        host,
        metrics,
        frontend,
        lobby_update_at: 0.0,
        loading_open: false,
        return_menu: None,
        stick: None,
        built_from: ui.scripts.len(),
        opened: false,
        mouse_at: None,
        test_step: 0,
        asked_logged: false,
        open_menu_sent: false,
        dumped: 0,
        opened_at: 0.0,
        binding: false,
        settings_rev: None,
        written: Vec::new(),
        sent: None,
        aspect: 0.0,
        board_open: false,
    }
}

/// Nuketown Zombies as the engine reports it: survival, standard rules,
/// the HUD shown, one player on the allies.
fn zombies_values(host: &mut Host) {
    let hud_bit = host
        .field("CoD", "BIT_HUD_VISIBLE")
        .as_num()
        .map_or(0, |n| n as i32);
    let allies = host
        .field("CoD", "TEAM_ALLIES")
        .as_num()
        .map_or(1, |n| n as i32);
    // An offline game (Restart Level in the pause menu).
    let offline = host.field("CoD", "SESSIONMODE_OFFLINE").as_num();
    let mut v = host.values.borrow_mut();
    for (k, x) in [
        ("r_fontResolution", "720"),
        ("ui_gametype", "zstandard"),
        ("g_gametype", "zstandard"),
        ("ui_zm_gamemodegroup", "zsurvival"),
        ("ui_zm_mapstartlocation", "nuked"),
        ("ui_mapname", "zm_nuked"),
        ("mapname", "zm_nuked"),
        // He hosts his own game (CoD.isHost: the leave-game popup ends it).
        ("sv_running", "1"),
        // The engine's own (the front end reads them).
        ("developer", "0"),
        // His party: up to four players (BO2's zombies party), one at this
        // PC.
        ("party_maxplayers", "4"),
        ("party_maxlocalplayers", "1"),
        // Each lobby kind's local players (one: this PC) and party size.
        ("party_maxlocalplayers_mainlobby", "1"),
        ("party_maxlocalplayers_playermatch", "1"),
        ("party_maxlocalplayers_privatematch", "1"),
        ("party_maxlocalplayers_theater", "1"),
        ("party_maxlocalplayers_systemlink", "1"),
        ("party_maxlocalplayers_local_splitscreen", "1"),
        ("party_maxplayers_privatematch", "4"),
        ("party_maxplayers_theater", "4"),
        ("party_maxplayers_systemlink", "4"),
        ("party_maxplayers_local_splitscreen", "1"),
        ("party_maxplayers_partylobby", "4"),
        ("party_playerCount", "1"),
        // Everyone in the party must be ready (he is: Start Match).
        ("party_readyPercentRequired", "1"),
    ] {
        v.dvars.insert(k.to_owned(), x.to_owned());
    }
    v.settings.insert("startRound".to_owned(), 1.0);
    // The one map this rebuild plays.
    v.maps = vec!["zm_nuked".to_owned()];
    // One team (the scoreboard draws a team's rows only when it counts
    // it), and Zombies' columns (its `setscoreboardcolumns`).
    v.settings.insert("teamCount".to_owned(), 1.0);
    v.columns = [
        "MPUI_SCORE",
        "MPUI_KILLS",
        "MPUI_DOWNS",
        "MPUI_REVIVES",
        "MPUI_HEADSHOTS",
    ]
    .map(str::to_owned)
    .to_vec();
    v.bits.insert(hud_bit);
    v.team = allies;
    v.session_modes = offline.into_iter().collect();
}

/// The values the HUD shows, from the presented snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
struct Values {
    round: i32,
    score: i32,
    weapon: i32,
    clip: i32,
    /// A dual-wield weapon's left clip.
    clip_left: Option<i32>,
    stock: i32,
    /// His grenades and mines (`name:count` by `;`).
    offhand: String,
    /// His HUD client fields (`field:value` by `,`: perks, power-ups).
    icons: String,
    /// His points counters (`score_cf_damage:3,...`).
    scorecf: String,
    /// His d-pad slots (`slot:weapon` by `;`, the scripts' setactionslot).
    slots: String,
    /// The held weapon's name and display-name key (`ZOMBIE_WEAPON_...`).
    weapon_name: String,
    weapon_key: String,
    /// The scoreboard's rows (`clientnum:name:score:kills:downs:revives:
    /// headshots` by `;`).
    board: String,
    /// The scripts hid his HUD (the game over).
    hidden: bool,
    /// The scripts hid his ammo counter and points (the game over's
    /// setclientammocounterhide / setclientminiscoreboardhide).
    ammo_hide: bool,
    /// The game-over shot (he watches the intermission camera).
    game_over: bool,
}

/// A d-pad weapon's icon.
fn slot_icon(weapon: &str) -> Option<&'static str> {
    Some(match weapon {
        "claymore_zm" => "hud_icon_claymore",
        _ => return None,
    })
}

/// The HUD icon of a grenade the engine names in `hud_update_offhand`, and
/// its slot.
fn offhand_slot(weapon: &str) -> Option<(&'static str, &'static str)> {
    Some(match weapon {
        "frag_grenade_zm" => ("lethal", "hud_us_grenade"),
        "sticky_grenade_zm" => ("lethal", "hud_icon_sticky_grenade"),
        "cymbal_monkey_zm" => ("tactical", "hud_cymbal_monkey"),
        _ => return None,
    })
}

/// (field, value) pairs of `bo2zm_icons`.
fn icon_fields(value: &str) -> Vec<(&str, i32)> {
    value
        .split(',')
        .filter_map(|e| {
            let (f, v) = e.split_once(':')?;
            Some((f, v.trim().parse().unwrap_or(0)))
        })
        .collect()
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

fn read_values(presented: &PresentedSnapshot, local: &LocalPresentClient) -> Option<Values> {
    let snap = presented.snapshot()?;
    let dvars = snap.meta.script_dvars(local.0);
    let hidden = dvars.string("bo2zm_hud_hidden") == Some("1");
    let round = dvars.string("bo2zm_round")?.trim().parse().ok()?;
    let meta = snap.meta.for_client(local.0)?;
    let ps = presented.player(local.0);
    Some(Values {
        round,
        score: meta.score,
        weapon: ps.map_or(0, |p| p.weapon as i32),
        clip: meta.ammo_clip,
        clip_left: ps
            .filter(|p| p.last_weapon_hand == 1)
            .map(|p| clip_of(&p.ammoclip, p.weapon as i32, 1)),
        stock: meta.ammo_stock,
        offhand: dvars.string("bo2zm_offhand").unwrap_or_default().to_owned(),
        icons: dvars.string("bo2zm_icons").unwrap_or_default().to_owned(),
        scorecf: dvars.string("bo2zm_scorecf").unwrap_or_default().to_owned(),
        slots: dvars
            .string("bo2zm_actionslots")
            .unwrap_or_default()
            .to_owned(),
        weapon_name: String::new(),
        weapon_key: String::new(),
        board: dvars
            .string("bo2zm_scoreboard")
            .unwrap_or_default()
            .to_owned(),
        hidden,
        ammo_hide: dvars.string("bo2zm_ammo_hide") == Some("1")
            || dvars.string("bo2zm_score_hide") == Some("1"),
        game_over: ps.is_some_and(|p| p.pm_type == playerstate_iw4::PM_TYPE_INTERMISSION),
    })
}

/// The engine's events for what changed since `old` (everything the first
/// time).
fn send_changes(host: &mut Host, old: Option<&Values>, new: &Values) {
    if old != Some(new) && std::env::var_os("BO2ZM_LUI_LOG").is_some() {
        diag::info!(Ui, "bo2zm lui values at {:.0} ms {new:?}", host.now_ms());
    }
    let changed = |f: fn(&Values) -> i64| old.is_none_or(|o| f(o) != f(new));
    if old.is_none_or(|o| o.board != new.board) {
        let mut v = host.values.borrow_mut();
        v.players.clear();
        v.clients.clear();
        for row in new.board.split(';').filter(|r| !r.is_empty()) {
            let mut cells = row.split(':');
            let client = cells.next().and_then(|c| c.parse().ok()).unwrap_or(0);
            v.clients.push(client);
            v.players.push(cells.map(str::to_owned).collect());
        }
    }
    if old.is_some_and(|o| o.ammo_hide != new.ammo_hide) {
        // BO2's HUD bit 12 (CoD.BIT_AMMO_COUNTER_HIDE): its ammo area and
        // score listen for hud_update_bit_12 and read the bit.
        let bit = host
            .field("CoD", "BIT_AMMO_COUNTER_HIDE")
            .as_num()
            .map_or(12, |n| n as i32);
        {
            let mut v = host.values.borrow_mut();
            if new.ammo_hide {
                v.bits.insert(bit);
            } else {
                v.bits.remove(&bit);
            }
        }
        host.root_event(
            &format!("hud_update_bit_{bit}"),
            &[("newValue", Value::Num(f32::from(u8::from(new.ammo_hide))))],
        );
    }
    if changed(|v| i64::from(v.round)) {
        host.root_event(
            "hud_update_rounds_played",
            &[
                ("roundsPlayed", Value::Num(new.round as f32)),
                ("wasDemoJump", Value::Bool(false)),
            ],
        );
    }
    // His points counters first (as BO2 runs client fields before the
    // scoreboard's update): each step flies its own "+10", and the score's
    // change then flies only what they did not cover (spending, bonuses).
    if let Some(o) = old
        && o.scorecf != new.scorecf
    {
        let was = icon_fields(&o.scorecf);
        for (f, v) in icon_fields(&new.scorecf) {
            let before = was.iter().find(|(g, _)| *g == f).map_or(0, |(_, x)| *x);
            if before != v {
                if std::env::var_os("IW4L_T6_HUDLOG").is_some() {
                    diag::info!(Ui, "bo2zm lui points step {f} {before} -> {v}");
                }
                host.root_event(
                    f,
                    &[
                        ("name", Value::str(f)),
                        ("entNum", Value::Num(0.0)),
                        ("newValue", Value::Num(v as f32)),
                        ("oldValue", Value::Num(before as f32)),
                    ],
                );
            }
        }
    }
    if changed(|v| i64::from(v.score)) {
        // One row per player (here the local one, client 0), ours first.
        let rows = hks_t6::value::Table::new_ref();
        let row = hks_t6::value::Table::new_ref();
        row.borrow_mut()
            .set_str("score", Value::Num(new.score as f32));
        row.borrow_mut().set_str("clientNum", Value::Num(0.0));
        rows.borrow_mut().set(Value::Num(1.0), Value::Table(row));
        host.root_event(
            "hud_update_competitive_scoreboard",
            &[
                ("competitivescores", Value::Table(rows)),
                ("selfindex", Value::Num(1.0)),
                ("bWasDemoJump", Value::Bool(false)),
            ],
        );
    }
    if changed(|v| i64::from(v.weapon)) {
        host.root_event(
            "hud_update_weapon",
            &[
                ("weapon", Value::str(&new.weapon_name)),
                ("inventorytype", Value::Num(0.0)),
            ],
        );
        // A switch shows the new gun's name (not the first one he spawns with).
        if old.is_some() && !new.weapon_key.is_empty() {
            host.root_event(
                "hud_update_weapon_select",
                &[("weaponDisplayName", Value::str(&new.weapon_key))],
            );
        }
    }
    if changed(|v| {
        i64::from(v.clip)
            | (i64::from(v.stock) << 20)
            | (i64::from(v.clip_left.unwrap_or(-1)) << 40)
    }) {
        let mut fields = vec![
            ("ammoInClip", Value::Num(new.clip as f32)),
            ("ammoStock", Value::Num(new.stock as f32)),
            ("lowClip", Value::Bool(false)),
        ];
        if let Some(left) = new.clip_left {
            fields.push(("ammoInDWClip", Value::Num(left as f32)));
        }
        host.root_event("hud_update_ammo", &fields);
    }
    if old.is_none_or(|o| o.offhand != new.offhand) {
        // Each slot: { material = <the icon>, ammo = <count> }.
        let mut fields = Vec::new();
        for e in new.offhand.split(';') {
            let Some((w, n)) = e.split_once(':') else {
                continue;
            };
            let Some((slot, icon)) = offhand_slot(w) else {
                continue;
            };
            let n: f32 = n.trim().parse().unwrap_or(0.0);
            if n <= 0.0 {
                continue;
            }
            let t = hks_t6::value::Table::new_ref();
            t.borrow_mut().set_str("material", host.material(icon));
            t.borrow_mut().set_str("ammo", Value::Num(n));
            fields.push((slot, Value::Table(t)));
        }
        host.root_event("hud_update_offhand", &fields);
    }
    if old.is_none_or(|o| o.slots != new.slots || o.offhand != new.offhand) {
        // actionSlotData[slot] = { material, ammo, aspectRatio }.
        let data = hks_t6::value::Table::new_ref();
        for e in new.slots.split(';') {
            let Some((n, w)) = e.split_once(':') else {
                continue;
            };
            let (Ok(n), Some(icon)) = (n.trim().parse::<f32>(), slot_icon(w)) else {
                continue;
            };
            let ammo: f32 = new
                .offhand
                .split(';')
                .find_map(|o| {
                    o.split_once(':')
                        .filter(|(k, _)| *k == w)
                        .and_then(|(_, c)| c.trim().parse().ok())
                })
                .unwrap_or(0.0);
            let t = hks_t6::value::Table::new_ref();
            t.borrow_mut().set_str("material", host.material(icon));
            t.borrow_mut().set_str("ammo", Value::Num(ammo));
            t.borrow_mut().set_str("aspectRatio", Value::Num(1.0));
            data.borrow_mut().set(Value::Num(n), Value::Table(t));
        }
        host.root_event(
            "hud_update_actionslots",
            &[("actionSlotData", Value::Table(data))],
        );
    }
    if old.is_none_or(|o| o.icons != new.icons) {
        // A client field's own event (`perk_juggernaut`, ...); one gone
        // from the list is back to 0.
        let now = icon_fields(&new.icons);
        if let Some(o) = old {
            for (f, _) in icon_fields(&o.icons) {
                if !now.iter().any(|(g, _)| *g == f) {
                    host.root_event(f, &[("newValue", Value::Num(0.0))]);
                }
            }
        }
        for (f, v) in now {
            let was = old.and_then(|o| {
                icon_fields(&o.icons)
                    .into_iter()
                    .find(|(g, _)| *g == f)
                    .map(|(_, x)| x)
            });
            if was != Some(v) {
                host.root_event(
                    f,
                    &[
                        ("newValue", Value::Num(v as f32)),
                        ("oldValue", Value::Num(was.unwrap_or(0) as f32)),
                    ],
                );
            }
        }
    }
}

/// Text without Black Ops II's colour codes (`^1`..`^9`) and button
/// pictures (`^BBUTTON_CYCLE_LEFT^`).
fn plain(text: &str) -> String {
    plain_with(text, None)
}

/// BO2's button token (`BUTTON_LUI_PRIMARY`, the `CoD.buttonStrings`) as
/// the pad button `frame::bo2_pad_glyph` names.
fn lui_button(token: &str) -> Option<&'static str> {
    Some(match token.strip_prefix("BUTTON_LUI_")? {
        "PRIMARY" => "south",
        "SECONDARY" => "east",
        "ALT1" => "west",
        "ALT2" => "north",
        "SELECT" => "select",
        "START" => "start",
        "SHOULDERL" => "lb",
        "SHOULDERR" => "rb",
        "LEFT_TRIGGER" => "lt",
        "RIGHT_TRIGGER" => "rt",
        "LEFT_STICK" | "LEFT_STICK_UP" => "ls",
        "RIGHT_STICK" => "rs",
        "DPAD_ALL" => "dpad_all",
        "DPAD_UD" => "dpad_ud",
        "DPAD_RL" | "DPAD_LR" => "dpad_rl",
        "DPAD_U" | "DPAD_UP" => "dpad_up",
        "DPAD_D" | "DPAD_DOWN" => "dpad_down",
        "DPAD_L" | "DPAD_LEFT" => "dpad_left",
        "DPAD_R" | "DPAD_RIGHT" => "dpad_right",
        _ => return None,
    })
}

/// `plain`, but with a pad's style BO2's button tokens become its button
/// pictures (the BO2 font draws them).
fn plain_with(text: &str, pad: Option<frame::PromptStyle>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c == '^' && it.peek().is_some_and(char::is_ascii_digit) {
            it.next();
            continue;
        }
        if c == '^' && it.peek() == Some(&'B') {
            it.next();
            let mut token = String::new();
            for d in it.by_ref() {
                if d == '^' {
                    break;
                }
                token.push(d);
            }
            if let Some(glyph) = pad
                .and_then(|style| lui_button(&token).and_then(|b| frame::bo2_pad_glyph(b, style)))
            {
                out.push_str(glyph);
            }
            continue;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

#[allow(clippy::too_many_arguments)]
fn lui_hud(
    mut commands: Commands,
    mut slot: NonSendMut<LuiHudSlot>,
    (ui, front_ui, screen): (
        Option<Res<assets::T6Ui>>,
        Option<Res<assets::T6Frontend>>,
        Res<frame::AppScreen>,
    ),
    fonts: Option<Res<Bo2Fonts>>,
    icons: Option<Res<assets::T6HudIcons>>,
    (mut image_assets, mut globes, globe_nodes): (
        ResMut<Assets<Image>>,
        ResMut<Assets<crate::bo2_globe::GlobeMaterial>>,
        Query<&bevy::ui_render::prelude::MaterialNode<crate::bo2_globe::GlobeMaterial>>,
    ),
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    (window, time): (Query<&Window, With<PrimaryWindow>>, Res<Time>),
    (keys, weapons, actions): (
        Option<Res<frame::HudInputView>>,
        Option<Res<assets::PreparedWeapons>>,
        Option<Res<net::ClientActionInput>>,
    ),
    mut io: LuiIo,
    roots: Query<Entity, With<LuiRoot>>,
    mut nodes: Query<(Entity, &mut LuiNode, &mut Node, &mut ZIndex)>,
    (mut image_nodes, mut xforms): (Query<&mut ImageNode>, Query<&mut UiTransform>),
    (children, computed): (
        Query<&Children>,
        Query<(&ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    ),
    (mut pictures, mut logged): (Local<HashMap<String, Handle<Image>>>, Local<usize>),
) {
    if !lui_enabled() {
        return;
    }
    let values = match (presented.as_deref(), local.as_deref()) {
        (Some(p), Some(l)) => read_values(p, l),
        _ => None,
    };
    // The held weapon's names from the weapon table.
    let values = values.map(|mut v| {
        if let Some(w) = weapons.as_deref() {
            v.weapon_name =
                w.0.name_of(v.weapon as u32)
                    .trim_end_matches("_mp")
                    .to_owned();
            v.weapon_key =
                w.0.display_name_key_of(v.weapon as u32)
                    .unwrap_or_default()
                    .to_owned();
        }
        v
    });
    // A Black Ops II game runs while its HUD has values: the pad's aim
    // assist is BO2's there.
    frame::set_bo2_game(values.is_some());
    // bo2zm M4: out of a game, on the menu screen, a game started on BO2's
    // own front end shows it (its scripts and pictures read at the start).
    let front = values.is_none()
        && frame::bo2_frontend().is_some()
        && matches!(
            *screen,
            frame::AppScreen::MainMenu | frame::AppScreen::Loading
        );
    let ui: Option<&assets::T6Ui> = if front {
        front_ui.as_deref().map(|f| &f.ui)
    } else {
        ui.as_deref()
    };
    let icons: Option<&assets::T6HudIcons> = if front {
        front_ui.as_deref().map(|f| &f.icons)
    } else {
        icons.as_deref()
    };
    let values = if front {
        Some(Values::default())
    } else {
        values
    };
    let (Some(ui), Some(values)) = (ui, values) else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        let idle = frame::LuiMenus::default();
        if *io.menus != idle {
            *io.menus = idle;
        }
        if io.paused.0 {
            io.paused.0 = false;
        }
        return;
    };
    // (Re)build when the map's scripts arrive.
    if slot
        .0
        .as_ref()
        .is_none_or(|h| h.built_from != ui.scripts.len() || h.frontend != front)
        && !ui.scripts.is_empty()
    {
        let t = std::time::Instant::now();
        // The front end hands over to the game: keep its lobby.
        let slot = &mut *slot;
        if !front && let Some(old) = slot.0.as_mut().filter(|h| h.frontend) {
            let filter = old
                .host
                .field("CoD", "PlaylistCategoryFilter")
                .as_str()
                .map(str::to_owned);
            let v = old.host.values.borrow();
            let state = FrontState {
                playlist: v.playlist,
                game_modes: v.game_modes.clone(),
                session_modes: v.session_modes.clone(),
                filter,
                map: v
                    .dvars
                    .iter()
                    .filter(|(k, _)| MAP_DVARS.iter().any(|m| m.eq_ignore_ascii_case(k)))
                    .map(|(k, x)| (k.clone(), x.clone()))
                    .collect(),
                last_map: v.profile.get("map_zm").cloned(),
            };
            drop(v);
            slot.1 = Some(state);
        }
        let mut hud = build(ui, front);
        // Back from a game: the lobby it started from.
        if front && let Some(st) = slot.1.take() {
            {
                let mut v = hud.host.values.borrow_mut();
                v.playlist = st.playlist;
                v.game_modes = st.game_modes;
                v.session_modes = st.session_modes;
                for (k, x) in st.map {
                    v.dvars
                        .retain(|old, _| !old.eq_ignore_ascii_case(&k));
                    v.dvars.insert(k, x);
                }
                if let Some(m) = st.last_map {
                    v.profile.insert("map_zm".to_owned(), m);
                }
            }
            if let Some(f) = &st.filter {
                hud.host
                    .set_global_field("CoD", "PlaylistCategoryFilter", Value::str(f));
            }
            let modes = hud.host.values.borrow().game_modes.clone();
            hud.return_menu = Some(if modes.contains(&0.0) {
                "PublicGameLobby"
            } else if modes.contains(&1.0) {
                "PrivateOnlineGameLobby"
            } else {
                "MainLobby"
            });
        }
        diag::info!(
            Ui,
            "bo2zm lui: {} scripts, base + {} in {:.0} ms; {} script errors; missing modules {:?}",
            ui.scripts.len(),
            if front {
                "T6.Main (front end)"
            } else {
                "T6.HUD"
            },
            t.elapsed().as_secs_f64() * 1000.0,
            hud.host.errors.len(),
            hud.host.missing.borrow()
        );
        slot.0 = Some(hud);
        pictures.clear();
        // The old screen goes (the front end's, when the game's HUD takes
        // over); drawing starts next frame on a new root, never this one's.
        if !roots.is_empty() {
            for e in &roots {
                commands.entity(e).despawn();
            }
            return;
        }
    }
    let Some(hud) = slot.0.as_mut() else {
        return;
    };
    // Font metrics for the scripts' text measure, once the fonts load.
    if hud.metrics.borrow().is_empty()
        && let Some(f) = fonts.as_deref()
    {
        *hud.metrics.borrow_mut() = f.advances();
    }
    // The keys his commands are bound to (the d-pad's key prompts).
    if let Some(k) = keys.as_deref() {
        let mut v = hud.host.values.borrow_mut();
        if v.binds.len() != k.binding_keys_all.len()
            || k.binding_keys_all
                .iter()
                .any(|(c, keys)| v.binds.get(c) != Some(keys))
        {
            v.binds = k
                .binding_keys_all
                .iter()
                .map(|(c, keys)| (c.clone(), keys.clone()))
                .collect();
        }
    }
    let Ok(win) = window.single() else {
        return;
    };
    let (ww, wh) = (win.width(), win.height());
    let aspect = ww / wh.max(1.0);
    let scale = wh / 720.0;
    if (hud.aspect - aspect).abs() > 1e-3 {
        hud.aspect = aspect;
        hud.host.root(aspect);
    }
    if !hud.opened {
        hud.opened = true;
        hud.opened_at = time.elapsed_secs_f64() * 1000.0;
        // Test aid: BO2ZM_LUI_FIRST_MENU=<menu> opens that menu instead of
        // the HUD (the front end: `main`).
        let first = std::env::var("BO2ZM_LUI_FIRST_MENU")
            .unwrap_or_else(|_| if hud.frontend { "main" } else { "HUD" }.to_owned());
        hud.host.open_menu(&first);
        if hud.frontend {
            // The engine's `open_menu`: BO2's black cover (`main`) opens
            // the menu it names on it (after a game, its lobby).
            let menu = hud.return_menu.unwrap_or("MainMenu");
            hud.host.root_event(
                "open_menu",
                &[
                    ("controller", Value::Num(0.0)),
                    ("menuName", Value::str(menu)),
                ],
            );
        } else {
            hud.host
                .root_event("first_snapshot", &[("controller", Value::Num(0.0))]);
            hud.host.root_event("hud_update_refresh", &[]);
        }
    }
    // The map loads after Start Match: BO2's loading screen over the menus.
    if hud.frontend && *screen == frame::AppScreen::Loading && !hud.loading_open {
        hud.loading_open = true;
        hud.host.open_menu("Loading");
        // bo2mc: its picture, name, place and mode fade in at once (BO2
        // waits 2 s, then a second each: a map here loads in about 3, so
        // the picture never showed).
        for event in [
            "start_loading",
            "fade_in_map_image",
            "fade_in_map_location",
            "fade_in_gametype",
        ] {
            hud.host
                .root_event(event, &[("controller", Value::Num(0.0))]);
        }
    }
    if !hud.frontend {
        // bo2mc: on the Minecraft world the scoreboard's place is the
        // Minecraft map's ("Survival - The Overworld", not Nuketown's
        // "Nevada, U.S.A."); the map stays zm_nuked to the HUD's scripts.
        {
            let mut v = hud.host.values.borrow_mut();
            const KEPT: &str = "BO2MC_ZMUI_NUKED";
            if !v.localize.contains_key(KEPT)
                && let Some(t) = v.localize.get("ZMUI_NUKED").cloned()
            {
                v.localize.insert(KEPT.to_owned(), t);
            }
            let want = if sim::bo2mc::enabled() {
                v.localize.get("ZMUI_MINECRAFT").cloned()
            } else {
                v.localize.get(KEPT).cloned()
            };
            if let Some(want) = want
                && v.localize.get("ZMUI_NUKED") != Some(&want)
            {
                v.localize.insert("ZMUI_NUKED".to_owned(), want);
            }
        }
        send_changes(&mut hud.host, hud.sent.as_ref(), &values);
    } else if hud.host.now_ms() - hud.lobby_update_at > 1000.0 {
        // The engine's lobby updates (it sends them as the party changes):
        // the lobbies refresh their titles and Start Match from them.
        hud.lobby_update_at = hud.host.now_ms();
        // Their members: him (the party panel's card).
        let members = hks_t6::host::lobby_members(&hud.host.values.borrow());
        hud.host.root_event(
            "gamelobby_update",
            &[
                ("controller", Value::Num(0.0)),
                ("members", members.clone()),
            ],
        );
        hud.host.root_event(
            "partylobby_update",
            &[("controller", Value::Num(0.0)), ("members", members)],
        );
    }
    // Test aid: BO2ZM_LUI_OPEN_MENU=<menu>: a second after the first menu
    // opens, the engine's `open_menu` event asks for that menu (BO2's front
    // end: its black cover opens `MainMenu` on it).
    if let Ok(menu) = std::env::var("BO2ZM_LUI_OPEN_MENU")
        && hud.host.now_ms() - hud.opened_at > 1000.0
        && !hud.open_menu_sent
    {
        hud.open_menu_sent = true;
        hud.host.root_event(
            "open_menu",
            &[
                ("controller", Value::Num(0.0)),
                ("menuName", Value::str(&menu)),
            ],
        );
    }
    // Test aid: BO2ZM_LUI_ASKED=1 logs, 8 s after the menus open, every
    // engine field the scripts asked for that nothing answers (counted).
    // (BO2ZM_LUI_ASKED=<ms>: that long after instead.)
    if let Some(after) = std::env::var("BO2ZM_LUI_ASKED").ok().map(|v| {
        v.parse::<f64>()
            .ok()
            .filter(|ms| *ms > 1.0)
            .unwrap_or(8000.0)
    }) && hud.host.now_ms() - hud.opened_at > after
        && !hud.asked_logged
    {
        hud.asked_logged = true;
        let asked = hud.host.asked.borrow();
        let list: Vec<String> = asked.iter().map(|(k, n)| format!("{k} x{n}")).collect();
        diag::info!(Ui, "bo2zm lui asked ({}): {}", list.len(), list.join(", "));
        diag::info!(
            Ui,
            "bo2zm lui missing widget setups: {:?}",
            hks_t6::lui::missing_methods()
        );
    }
    // BO2's scoreboard: open while Tab (+scores) is held and through the
    // game-over shot (the engine opens it; its scripts refresh it).
    let scores_down = actions
        .as_deref()
        .is_some_and(|a| a.client.kb.scores.active);
    let want_board = !hud.frontend && (scores_down || values.game_over);
    if want_board != hud.board_open {
        hud.board_open = want_board;
        let event = if want_board {
            "open_scoreboard_menu"
        } else {
            "close_scoreboard_menu"
        };
        hud.host
            .root_event(event, &[("controller", Value::Num(0.0))]);
    }
    let hidden = values.hidden;
    hud.sent = Some(values);
    // BO2ZM_LUI_PAUSE_AT=<ms after the HUD opened>[,<button>@<ms>...]: open
    // the pause menu then, and press pad buttons later (test aid). `-`
    // first: no pause menu (the front end's menus are up already).
    if let Ok(plan) = std::env::var("BO2ZM_LUI_PAUSE_AT") {
        let now = hud.host.now_ms() - hud.opened_at;
        let mut steps = plan.split(',');
        let first = steps.next().map_or("", str::trim);
        let at = if first == "-" {
            Some(0.0)
        } else {
            first.parse::<f64>().ok()
        };
        if let Some(at) = at
            && now >= at
            && hud.test_step == 0
        {
            hud.test_step = 1;
            if first != "-" {
                hud.host
                    .root_event("open_ingame_menu", &[("menuName", Value::str("class"))]);
            }
        }
        for (i, step) in steps.enumerate() {
            let Some((button, at)) = step.split_once('@') else {
                continue;
            };
            if hud.test_step == i + 1 && at.trim().parse::<f64>().is_ok_and(|t| now >= t) {
                hud.test_step = i + 2;
                diag::info!(Ui, "bo2zm lui test: {button}");
                // `click:x:y` = the mouse there, pressed and let go (root
                // units); else a pad button.
                let mut c = button.trim().split(':');
                let kind = c.next();
                if kind == Some("move") {
                    // `move:x:y` = the mouse there, no click.
                    let x: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let y: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    hud.host.mouse("mousemove", x, y, "");
                } else if kind == Some("click") {
                    let x: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let y: f32 = c.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    hud.host.mouse("mousemove", x, y, "");
                    hud.host.mouse("mousedown", x, y, "left");
                    hud.host.mouse("mouseup", x, y, "left");
                } else if let Some(b) = button.trim().strip_suffix('+') {
                    // `right+` presses and holds, `right-` lets go.
                    hud.host.button(b, true);
                } else if let Some(b) = button.trim().strip_suffix('-') {
                    hud.host.button(b, false);
                } else {
                    hud.host.button(button.trim(), true);
                    hud.host.button(button.trim(), false);
                }
            }
        }
    }
    // His keys and mouse, then what the menus asked of the engine. While
    // a bind row waits for a key, the key is the capture's; when it is
    // done the rows show the new keys (`key_bound`).
    let console_open = keys.as_deref().is_some_and(|k| k.console_open);
    let capturing = io.capture.command.is_some() || io.capture.consumed_input;
    if hud.binding && !capturing {
        hud.binding = false;
        hud.host.root_event("key_bound", &[]);
    }
    if !capturing {
        let cursor = win.cursor_position().or(io.test_cursor.0);
        menu_input(hud, &io, console_open, cursor, scale);
    }
    engine_calls(hud, &mut io, local.as_deref());
    let monitors = io.monitors.iter().count().max(1);
    {
        let mut v = hud.host.values.borrow_mut();
        v.dvars
            .insert("r_monitorCount".to_owned(), monitors.to_string());
        v.dvars
            .entry("r_monitor".to_owned())
            .or_insert_with(|| "0".to_owned());
    }
    settings_bridge(hud, &mut io.settings);
    // His last input: BO2's prompts follow it (`Engine.LastInput_Gamepad`,
    // `input_source_changed`: source 0 the pad, 1 keys and mouse).
    let pad_style = io
        .devices
        .as_deref()
        .filter(|d| d.pad_prompts)
        .map(|d| d.style);
    let gamepad = pad_style.is_some();
    if hud.host.values.borrow().gamepad != gamepad {
        hud.host.values.borrow_mut().gamepad = gamepad;
        hud.host.root_event(
            "input_source_changed",
            &[
                ("controller", Value::Num(0.0)),
                ("source", Value::Num(if gamepad { 0.0 } else { 1.0 })),
            ],
        );
    }
    let open = !hud.host.open_menus().is_empty();
    let state = frame::LuiMenus { active: true, open };
    if *io.menus != state {
        *io.menus = state;
    }
    // A solo game pauses under its menus (Black Ops II's solo pause).
    let pause = open && !hud.frontend && io.role.as_deref() == Some(&frame::RuntimeRole::Listen);
    if io.paused.0 != pause {
        io.paused.0 = pause;
        diag::info!(
            Ui,
            "bo2zm lui: game {}",
            if pause { "paused" } else { "resumed" }
        );
    }
    hud.host.frame(time.elapsed_secs_f64() * 1000.0);
    if hud.host.errors.len() > *logged {
        for e in &hud.host.errors[*logged..] {
            diag::warn!(Ui, "bo2zm lui script error: {e}");
        }
        *logged = hud.host.errors.len();
    }
    // BO2ZM_LUI_DUMP=1: every element the scripts have, each time their
    // number (or, bo2mc, the number shown) changes (debugging aid).
    if std::env::var_os("BO2ZM_LUI_DUMP").is_some() {
        let all = hud.host.drawn();
        let shown = all.iter().filter(|d| d.alpha > 0.01).count();
        if all.len() * 100_000 + shown != hud.dumped {
            hud.dumped = all.len() * 100_000 + shown;
            for d in &all {
                diag::info!(
                    Ui,
                    "bo2zm lui dump {}: {} {:?} rect {:?} alpha {:.2} rgb {:?} text {:?} align {} anchors {:?}",
                    d.id,
                    d.kind,
                    d.material,
                    d.rect,
                    d.alpha,
                    d.rgb,
                    d.text,
                    d.alignment,
                    d.anchors
                );
            }
        }
    }
    // His HUD hidden (the game over): only the scoreboard shows.
    let only = if hidden {
        Some(
            hud.host
                .menu_element_ids("Menu.Scoreboard")
                .unwrap_or_default(),
        )
    } else {
        None
    };
    let drawn: Vec<Drawn> = hud
        .host
        .drawn()
        .into_iter()
        .filter(|d| {
            only.as_ref().is_none_or(|ids| ids.contains(&d.id))
                && d.alpha > 0.004
                && (d.blur
                    || d.material.is_some()
                    || d.kind == "image"
                    || d.text.as_deref().is_some_and(|t| !t.trim().is_empty())
                    || (d.kind == "dashes" && d.dashes.0 > 0))
        })
        .collect();

    let root = match roots.iter().next() {
        Some(r) => r,
        None => {
            let root = commands
                .spawn((
                    LuiRoot,
                    // The front end draws over the menu and loading screens.
                    if hud.frontend {
                        UiLayer::Overlay
                    } else {
                        UiLayer::Hud
                    },
                    UiLayerVisibility,
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                ))
                .id();
            if hud.frontend {
                // Above the engine's own loading screen.
                commands.entity(root).insert(GlobalZIndex(1_000_000));
            }
            root
        }
    };
    let mut existing: HashMap<usize, Entity> = HashMap::new();
    for (e, n, ..) in &nodes {
        existing.insert(n.id, e);
    }
    let mut keep: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (order, d) in drawn.iter().enumerate() {
        let [x0, y0, x1, y1] = d.rect;
        // Clipped by a stencil ancestor (setUseStencil): wholly outside its
        // box is not drawn; a picture across its edge is cropped below.
        let crop = d.clip.map(|c| {
            [
                x0.min(x1).max(c[0]),
                y0.min(y1).max(c[1]),
                x0.max(x1).min(c[2]),
                y0.max(y1).min(c[3]),
            ]
        });
        if crop.is_some_and(|v| v[2] <= v[0] || v[3] <= v[1]) {
            continue;
        }
        // The scripts' colours are linear (BO2 draws into an sRGB target:
        // the chalk's 0.21 red shows as a mid blood red).
        let color = Color::linear_rgba(d.rgb[0], d.rgb[1], d.rgb[2], d.alpha.clamp(0.0, 1.0));
        let z = ZIndex(order as i32 + 1);
        if d.blur && d.material.is_none() {
            // A popup's blur of what is behind it: a dark veil (the menu
            // under it fades out as BO2's blur hides it).
            keep.insert(d.id);
            let node = Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x0 * scale),
                top: Val::Px(y0 * scale),
                width: Val::Px((x1 - x0).abs() * scale),
                height: Val::Px((y1 - y0).abs() * scale),
                ..default()
            };
            let veil = Color::linear_rgba(0.0, 0.0, 0.0, 0.8 * d.alpha.clamp(0.0, 1.0));
            let shows = "blur".to_owned();
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                *nn = node;
                *zi = z;
                commands.entity(e).insert(BackgroundColor(veil));
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                commands.entity(e).despawn();
            }
            let e = commands
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: veil.alpha(),
                    },
                    node,
                    BackgroundColor(veil),
                    z,
                ))
                .id();
            commands.entity(root).add_child(e);
        } else if d.kind == "dashes" {
            // A slider's bar: `count` dashes across its rectangle, the lit
            // ones bright (BO2's engine draws them; flat bars here).
            keep.insert(d.id);
            let (count, lit) = d.dashes;
            // Each dash is half its pitch wide, two thirds of the height.
            let pitch = d.dash_pitch * scale;
            let node = Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x0 * scale),
                top: Val::Px(y0 * scale),
                width: Val::Px(pitch * count as f32),
                height: Val::Px((y1 - y0).abs() * scale),
                align_items: AlignItems::Center,
                ..default()
            };
            let shows = format!(
                "dashes {count} {lit} {:.2} {:.2} {:.2} {:.2}",
                d.rgb[0], d.rgb[1], d.rgb[2], d.alpha
            );
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                *nn = node;
                *zi = z;
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                commands.entity(e).despawn();
            }
            let e = commands
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: color.alpha(),
                    },
                    node,
                    z,
                ))
                .with_children(|c| {
                    for i in 0..count {
                        let a = if i < lit {
                            color.alpha()
                        } else {
                            color.alpha() * 0.25
                        };
                        c.spawn((
                            Node {
                                width: Val::Px(pitch * 0.5),
                                margin: UiRect::right(Val::Px(pitch * 0.5)),
                                height: Val::Percent(66.0),
                                ..default()
                            },
                            BackgroundColor(color.with_alpha(a)),
                        ));
                    }
                })
                .id();
            commands.entity(root).add_child(e);
        } else if d.kind == "globe" {
            // bo2zm M4: the Zombies globe, by its own shader (its mesh and
            // day map are both globe_map_zm, the material's picture; turned
            // by shader vector 2, revealed by vector 0).
            let Some(map) = d
                .material
                .as_deref()
                .and_then(|m| picture(&mut pictures, icons, &mut image_assets, m))
            else {
                continue;
            };
            let (mesh, day) = (map.clone(), map);
            keep.insert(d.id);
            let node = Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x0 * scale),
                top: Val::Px(y0 * scale),
                width: Val::Px((x1 - x0).abs() * scale),
                height: Val::Px((y1 - y0).abs() * scale),
                ..default()
            };
            let params = crate::bo2_globe::GlobeParams {
                v0: Vec4::from_array(d.shader[0]),
                v2: Vec4::from_array(d.shader[2]),
                color: Vec4::new(d.rgb[0], d.rgb[1], d.rgb[2], d.alpha.clamp(0.0, 1.0)),
            };
            let shows = "globe".to_owned();
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                *nn = node;
                *zi = z;
                if let Ok(m) = globe_nodes.get(e)
                    && let Some(mut mat) = globes.get_mut(&m.0)
                    && mat.params != params
                {
                    mat.params = params;
                }
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                commands.entity(e).despawn();
            }
            let e = commands
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: color.alpha(),
                    },
                    node,
                    crate::bo2_globe::globe_node(&mut globes, params, mesh, day),
                    z,
                ))
                .id();
            commands.entity(root).add_child(e);
        } else if d.kind == "image"
            && d.material.as_deref().is_none_or(|m| {
                m == "white" || picture(&mut pictures, icons, &mut image_assets, m).is_none()
            })
        {
            // A picture with no material (or the engine's plain `white`) is
            // a block of its colour: BO2's dim behind its menus.
            keep.insert(d.id);
            let node = Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x0 * scale),
                top: Val::Px(y0 * scale),
                width: Val::Px((x1 - x0).abs() * scale),
                height: Val::Px((y1 - y0).abs() * scale),
                ..default()
            };
            let shows = "block".to_owned();
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                *nn = node;
                *zi = z;
                commands.entity(e).insert(BackgroundColor(color));
                continue;
            }
            if let Some(&e) = existing.get(&d.id) {
                commands.entity(e).despawn();
            }
            let e = commands
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: color.alpha(),
                    },
                    node,
                    BackgroundColor(color),
                    z,
                ))
                .id();
            commands.entity(root).add_child(e);
        } else if let Some(mat) = &d.material {
            // BO2ZM_LUI_FORCEMAT=<id>:<material> draws that element with
            // another picture (debugging aid).
            let forced = std::env::var("BO2ZM_LUI_FORCEMAT")
                .ok()
                .and_then(|v| {
                    v.split_once(':')
                        .map(|(i, m)| (i.parse::<usize>().ok(), m.to_owned()))
                })
                .filter(|(i, _)| *i == Some(d.id))
                .map(|(_, m)| m);
            let mat = forced.as_ref().unwrap_or(mat);
            let Some(handle) = picture(&mut pictures, icons, &mut image_assets, mat) else {
                continue;
            };
            keep.insert(d.id);
            // Cropped to its clip: the visible part of the box, and the
            // same part of the picture (unturned pictures only).
            let (bx0, by0, bx1, by1) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
            let (vis, part) = match crop {
                Some(v) if d.z_rot.abs() < 0.01 && v != [bx0, by0, bx1, by1] => {
                    let size = image_assets
                        .get(&handle)
                        .map_or(Vec2::ONE, |i| i.size().as_vec2());
                    let (bw, bh) = ((bx1 - bx0).max(1e-3), (by1 - by0).max(1e-3));
                    let part = bevy::math::Rect::new(
                        (v[0] - bx0) / bw * size.x,
                        (v[1] - by0) / bh * size.y,
                        (v[2] - bx0) / bw * size.x,
                        (v[3] - by0) / bh * size.y,
                    );
                    (v, Some(part))
                }
                _ => ([x0, y0, x1, y1], None),
            };
            let node = Node {
                position_type: PositionType::Absolute,
                left: Val::Px(vis[0] * scale),
                top: Val::Px(vis[1] * scale),
                width: Val::Px((vis[2] - vis[0]).abs() * scale),
                height: Val::Px((vis[3] - vis[1]).abs() * scale),
                ..default()
            };
            // setZRot: the picture turned (BO2's right bracket is the left
            // one turned 180 degrees).
            let turn = UiTransform {
                rotation: Rot2::degrees(-d.z_rot),
                ..default()
            };
            let shows = format!("image {mat}");
            if std::env::var("BO2ZM_LUI_CHAIN")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                == Some(d.id)
            {
                diag::info!(
                    Ui,
                    "bo2zm lui chain #{}: {:?}",
                    d.id,
                    hud.host.chain_of(d.id)
                );
            }
            if std::env::var("BO2ZM_LUI_WATCH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                == Some(d.id)
            {
                diag::info!(
                    Ui,
                    "bo2zm lui watch #{} at {:.0} ms: rect {:?} colour {:?} entity {:?} z {} computed {:?}",
                    d.id,
                    hud.host.now_ms(),
                    d.rect,
                    color,
                    existing.get(&d.id),
                    order,
                    existing
                        .get(&d.id)
                        .and_then(|e| computed.get(*e).ok())
                        .map(|(c, t, v)| (c.size(), t.translation, v.get()))
                );
            }
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                *nn = node;
                *zi = z;
                if let Ok(mut img) = image_nodes.get_mut(e) {
                    img.color = color;
                    if img.rect != part {
                        img.rect = part;
                    }
                }
                if let Ok(mut x) = xforms.get_mut(e) {
                    *x = turn;
                }
                continue;
            }
            if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                diag::info!(Ui, "bo2zm lui image #{} {shows} at {:?}", d.id, d.rect);
            }
            if let Some(&e) = existing.get(&d.id) {
                commands.entity(e).despawn();
            }
            let e = commands
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: color.alpha(),
                    },
                    node,
                    // LUI stretches a picture over its whole box (a 64x64
                    // bracket in a 64x30 row is squeezed, not shrunk).
                    ImageNode {
                        image: handle,
                        color,
                        rect: part,
                        image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                        ..default()
                    },
                    turn,
                    z,
                ))
                .id();
            commands.entity(root).add_child(e);
        } else if let (Some(text), Some(bo2)) = (&d.text, fonts.as_deref()) {
            keep.insert(d.id);
            let text = plain_with(text, pad_style);
            let font = font_name(d.font.as_deref().unwrap_or("normalFont"));
            let px = (y1 - y0).abs() * scale;
            let width = bo2.line_width(font, &text, px);
            // LUI.Alignment: 1 left, 2 centre, 3 right; none draws from the
            // left edge, except in a box placed from its parent's centre,
            // which it is centred on (MFTabManager's 2-wide tab titles).
            let align = match (d.alignment, d.anchors) {
                (1..=3, _) => d.alignment,
                (_, (false, false)) => 2,
                _ => 1,
            };
            // A line longer than its box goes onto more lines, broken at
            // spaces, as LUI wraps text to its width (descriptions).
            let box_w = (x1 - x0).abs() * scale;
            let lines = if box_w > px * 2.0 && width > box_w + 1.0 {
                wrap_lines(bo2, font, &text, px, box_w)
            } else {
                vec![text.clone()]
            };
            let node = if lines.len() > 1 {
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x0.min(x1) * scale),
                    top: Val::Px(y0 * scale),
                    width: Val::Px(box_w),
                    flex_direction: FlexDirection::Column,
                    ..default()
                }
            } else {
                let left = match align {
                    2 => (x0 + x1) * 0.5 * scale - width * 0.5,
                    3 => x1 * scale - width,
                    _ => x0 * scale,
                };
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(left),
                    top: Val::Px(y0 * scale),
                    ..default()
                }
            };
            let shows = format!(
                "text {text}|{font}|{px:.1}|{:.2}|{:.2}|{:.2}|{}",
                d.rgb[0],
                d.rgb[1],
                d.rgb[2],
                lines.len()
            );
            if let Some(&e) = existing.get(&d.id)
                && let Ok((_, mut n, mut nn, mut zi)) = nodes.get_mut(e)
                && n.shows == shows
            {
                *nn = node;
                *zi = z;
                // A fade: every letter's alpha.
                if (n.alpha - color.alpha()).abs() > 1e-3 {
                    n.alpha = color.alpha();
                    for g in children.iter_descendants(e) {
                        if let Ok(mut img) = image_nodes.get_mut(g) {
                            img.color.set_alpha(color.alpha());
                        }
                    }
                }
                continue;
            }
            if std::env::var_os("BO2ZM_LUI_LOG").is_some() {
                diag::info!(Ui, "bo2zm lui text #{} {shows} at {:?}", d.id, d.rect);
            }
            if let Some(&e) = existing.get(&d.id) {
                commands.entity(e).despawn();
            }
            let e = commands
                .spawn((
                    LuiNode {
                        id: d.id,
                        shows,
                        alpha: color.alpha(),
                    },
                    node,
                    z,
                ))
                .with_children(|c| {
                    if lines.len() > 1 {
                        let justify = match align {
                            2 => JustifyContent::Center,
                            3 => JustifyContent::FlexEnd,
                            _ => JustifyContent::FlexStart,
                        };
                        for line in &lines {
                            c.spawn(Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(px),
                                justify_content: justify,
                                ..default()
                            })
                            .with_children(|l| {
                                bo2.spawn_line(l, font, &[(line.clone(), color)], px, 0.0);
                            });
                        }
                    } else {
                        bo2.spawn_line(c, font, &[(text.clone(), color)], px, 0.0);
                    }
                })
                .id();
            commands.entity(root).add_child(e);
        }
    }
    for (id, e) in existing {
        if !keep.contains(&id) {
            commands.entity(e).despawn();
        }
    }
}

/// `text` broken at spaces into lines no wider than `width` pixels (a word
/// wider than that keeps a line to itself).
fn wrap_lines(bo2: &Bo2Fonts, font: &str, text: &str, px: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && bo2.line_width(font, &candidate, px) > width {
            lines.push(std::mem::take(&mut line));
            line = word.to_owned();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A material's picture as an image handle (made once).
fn picture(
    cache: &mut HashMap<String, Handle<Image>>,
    icons: Option<&assets::T6HudIcons>,
    images: &mut Assets<Image>,
    name: &str,
) -> Option<Handle<Image>> {
    if let Some(h) = cache.get(name) {
        return Some(h.clone());
    }
    let Some(img) = icons?.0.iter().find(|(k, _)| k == name).map(|(_, i)| i.clone()) else {
        // BO2ZM_LUI_MISSING=1: each picture the menus draw that the zones
        // did not give (drawn white), once (debugging aid).
        if std::env::var_os("BO2ZM_LUI_MISSING").is_some() {
            static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            if let Ok(mut seen) = SEEN.lock()
                && !seen.iter().any(|s| s == name)
            {
                seen.push(name.to_owned());
                diag::info!(Ui, "bo2zm lui picture missing: {name}");
            }
        }
        return None;
    };
    // BO2ZM_LUI_PICPROFILE=<material>: its size, format and each pixel
    // column's strongest alpha (where a picture's ink sits; debugging aid).
    if std::env::var("BO2ZM_LUI_PICPROFILE").is_ok_and(|v| v == name) {
        let size = img.size();
        let cols: Vec<u8> = img.data.as_ref().map_or(Vec::new(), |d| {
            let (w, h) = (size.x as usize, size.y as usize);
            (0..w)
                .map(|x| {
                    (0..h)
                        .map(|y| d.get((y * w + x) * 4 + 3).copied().unwrap_or(0))
                        .max()
                        .unwrap_or(0)
                })
                .collect()
        });
        diag::info!(
            Ui,
            "bo2zm lui picture {name}: {size:?} {:?} column alpha {cols:?}",
            img.texture_descriptor.format
        );
    }
    let h = images.add((*img).clone());
    cache.insert(name.to_owned(), h.clone());
    Some(h)
}

pub(crate) fn register_lui_hud_systems(app: &mut App) {
    app.insert_non_send(LuiHudSlot::default());
    app.init_resource::<frame::TestCursor>();
    app.add_systems(
        Update,
        lui_hud
            .after(crate::bo2_font::load_bo2_fonts)
            .in_set(ClientSet::Ui),
    );
}
