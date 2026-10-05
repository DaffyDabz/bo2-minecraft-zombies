use bevy::prelude::{Entity, Message, Resource};

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivePad(pub Option<Entity>);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PromptStyle {
    Xbox,
    PlayStation,
    #[default]
    Generic,
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputDevices {
    pub pad_prompts: bool,
    pub style: PromptStyle,
    pub focused: bool,
    pub aiming_with_pad: bool,
}

#[derive(Message)]
pub struct TestControllerRumble;

/// bo2zm: a Black Ops II game is being played (its HUD runs). The pad's aim
/// assist works there, as BO2's does on a controller (slowdown, staying on a
/// zombie, the snap when aiming down the sights; IW4 on PC has none), and pad
/// buttons show as BO2's button pictures.
static BO2_GAME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_bo2_game(on: bool) {
    BO2_GAME.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn bo2_game() -> bool {
    BO2_GAME.load(std::sync::atomic::Ordering::Relaxed)
}

/// bo2zm M4: the map Black Ops II's own front end starts (`iw4l frontend
/// t6:zm_nuked`): the game opens on its main menu, Start loads the map and
/// the end of a game goes back to the menu. None = straight into the map.
static BO2_FRONTEND: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_bo2_frontend(zone: &str) {
    let _ = BO2_FRONTEND.set(zone.to_owned());
}

pub fn bo2_frontend() -> Option<&'static str> {
    BO2_FRONTEND.get().map(String::as_str)
}

/// bo2zm: the characters that stand for BO2's Xbox button pictures
/// (`xenonbutton_*` in code_post_gfx_zm): private-use code points the BO2
/// font draws as those pictures.
pub const BO2_XBOX_GLYPHS: [(char, &str); 19] = [
    ('\u{E100}', "xenonbutton_a"),
    ('\u{E101}', "xenonbutton_b"),
    ('\u{E102}', "xenonbutton_x"),
    ('\u{E103}', "xenonbutton_y"),
    ('\u{E104}', "xenonbutton_lb"),
    ('\u{E105}', "xenonbutton_rb"),
    ('\u{E106}', "xenonbutton_lt"),
    ('\u{E107}', "xenonbutton_rt"),
    ('\u{E108}', "xenonbutton_back"),
    ('\u{E109}', "xenonbutton_start"),
    ('\u{E10A}', "xenonbutton_ls"),
    ('\u{E10B}', "xenonbutton_rs"),
    ('\u{E10C}', "xenonbutton_dpad_all"),
    ('\u{E10D}', "xenonbutton_dpad_ud"),
    ('\u{E10E}', "xenonbutton_dpad_rl"),
    ('\u{E10F}', "xenonbutton_dpad_up"),
    ('\u{E110}', "xenonbutton_dpad_down"),
    ('\u{E111}', "xenonbutton_dpad_left"),
    ('\u{E112}', "xenonbutton_dpad_right"),
];

/// One-character strings of `BO2_XBOX_GLYPHS`, in its order.
const BO2_XBOX_TEXT: [&str; 19] = [
    "\u{E100}", "\u{E101}", "\u{E102}", "\u{E103}", "\u{E104}", "\u{E105}", "\u{E106}", "\u{E107}",
    "\u{E108}", "\u{E109}", "\u{E10A}", "\u{E10B}", "\u{E10C}", "\u{E10D}", "\u{E10E}", "\u{E10F}",
    "\u{E110}", "\u{E111}", "\u{E112}",
];

/// bo2zm: a pad button as BO2 shows it: on an Xbox pad its picture; on a
/// PlayStation pad the face shapes (□ ○ △ ×, drawn by the BO2 font), the
/// shoulders and sticks as L1 R1 L2 R2 L3 R3, the d-pad as BO2's d-pad
/// picture. Names: south east west north lb rb lt rt ls rs select start
/// dpad_all dpad_ud dpad_rl dpad_up dpad_down dpad_left dpad_right.
pub fn bo2_pad_glyph(button: &str, style: PromptStyle) -> Option<&'static str> {
    let ps = style == PromptStyle::PlayStation;
    let x = |i: usize| BO2_XBOX_TEXT[i];
    Some(match button {
        "south" => {
            if ps {
                "×"
            } else {
                x(0)
            }
        }
        "east" => {
            if ps {
                "○"
            } else {
                x(1)
            }
        }
        "west" => {
            if ps {
                "□"
            } else {
                x(2)
            }
        }
        "north" => {
            if ps {
                "△"
            } else {
                x(3)
            }
        }
        "lb" => {
            if ps {
                "L1"
            } else {
                x(4)
            }
        }
        "rb" => {
            if ps {
                "R1"
            } else {
                x(5)
            }
        }
        "lt" => {
            if ps {
                "L2"
            } else {
                x(6)
            }
        }
        "rt" => {
            if ps {
                "R2"
            } else {
                x(7)
            }
        }
        "select" => {
            if ps {
                "SHARE"
            } else {
                x(8)
            }
        }
        "start" => {
            if ps {
                "OPTIONS"
            } else {
                x(9)
            }
        }
        "ls" => {
            if ps {
                "L3"
            } else {
                x(10)
            }
        }
        "rs" => {
            if ps {
                "R3"
            } else {
                x(11)
            }
        }
        "dpad_all" => x(12),
        "dpad_ud" => x(13),
        "dpad_rl" => x(14),
        "dpad_up" => x(15),
        "dpad_down" => x(16),
        "dpad_left" => x(17),
        "dpad_right" => x(18),
        _ => return None,
    })
}
