//! bo2mc: Minecraft's chat (T, or / to start a command): what was typed
//! said in the chat, and Minecraft's commands carried out: /gamemode, /time,
//! /give, /tp, /kill, /summon, /seed, /help; and zombies' own /round and
//! /points.

use super::{Bo2mcWorld, Frame};

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const GRAY: [f32; 3] = [0.67, 0.67, 0.67];
const RED: [f32; 3] = [1.0, 0.33, 0.33];
/// Lines the chat keeps.
const KEEP: usize = 100;

fn say(f: &mut Frame<'_>, text: impl Into<String>, color: [f32; 3]) {
    f.ui.chat_log.push(frame::McChatLine { text: text.into(), color, at: f.now });
    let over = f.ui.chat_log.len().saturating_sub(KEEP);
    f.ui.chat_log.drain(..over);
}

/// Minecraft's names for the game modes.
fn mode_name(mode: u8) -> &'static str {
    match mode {
        sim::bo2mc::CREATIVE => "Creative",
        sim::bo2mc::ADVENTURE => "Adventure",
        sim::bo2mc::SPECTATOR => "Spectator",
        _ => "Survival",
    }
}

fn parse_mode(word: &str) -> Option<u8> {
    match word {
        "survival" | "s" | "0" => Some(sim::bo2mc::SURVIVAL),
        "creative" | "c" | "1" => Some(sim::bo2mc::CREATIVE),
        "adventure" | "a" | "2" => Some(sim::bo2mc::ADVENTURE),
        "spectator" | "sp" | "3" => Some(sim::bo2mc::SPECTATOR),
        _ => None,
    }
}

/// A coordinate: a number, or `~` / `~n` from where the player stands.
fn coordinate(word: &str, here: f64) -> Option<f64> {
    match word.strip_prefix('~') {
        Some("") => Some(here),
        Some(rest) => rest.parse::<f64>().ok().map(|d| here + d),
        // A whole block number means the block's middle, as Minecraft's.
        None if !word.contains('.') => word.parse::<i64>().ok().map(|b| b as f64 + 0.5),
        None => word.parse().ok(),
    }
}

/// The height word of a `[x, y, z]`.
fn y_word<'a>(rest: &[&'a str]) -> &'a str {
    rest.get(1).copied().unwrap_or("~")
}

fn y_relative(word: &str) -> bool {
    word.starts_with('~') || word.contains('.')
}

fn unknown(f: &mut Frame<'_>, line: &str) {
    say(f, "Unknown or incomplete command, see below for error", RED);
    say(f, format!("{line}<--[HERE]"), RED);
}

impl Bo2mcWorld {
    /// The chat's lines this frame: from the keyboard, and from the test
    /// hook IW4L_BO2MC_TEST_CHAT="secs:/cmd|secs:/cmd" (seconds after the
    /// room stood).
    pub(super) fn chat(&mut self, f: &mut Frame<'_>, before: f64) {
        let mut lines = std::mem::take(&mut f.ui.chat_submitted);
        if let Ok(script) = std::env::var("IW4L_BO2MC_TEST_CHAT") {
            for item in script.split('|') {
                if let Some((at, cmd)) = item.split_once(':')
                    && let Ok(at) = at.trim().parse::<f64>()
                    && before < at
                    && self.test_clock >= at
                {
                    lines.push(cmd.trim().to_owned());
                }
            }
        }
        for line in lines {
            diag::info!(World, "bo2mc chat: {line}");
            match line.strip_prefix('/') {
                Some(command) => self.command(f, command),
                None => say(f, format!("<Player> {line}"), WHITE),
            }
        }
    }

    fn command(&mut self, f: &mut Frame<'_>, command: &str) {
        let words: Vec<&str> = command.split_whitespace().collect();
        let line = format!("/{command}");
        match words.as_slice() {
            ["gamemode", mode, ..] => match parse_mode(&mode.to_ascii_lowercase()) {
                Some(mode) => {
                    sim::bo2mc::set_game_mode(mode);
                    say(f, format!("Set own game mode to {} Mode", mode_name(mode)), WHITE);
                }
                None => say(f, format!("Unknown game mode: {mode}"), RED),
            },
            ["gamemode"] => say(f, format!("Your game mode is {} Mode", mode_name(sim::bo2mc::game_mode())), WHITE),
            ["time", "set", when] => {
                let ticks = match *when {
                    "day" => Some(1000.0),
                    "noon" => Some(6000.0),
                    "night" => Some(13_000.0),
                    "midnight" => Some(18_000.0),
                    n => n.trim_end_matches('t').parse::<f64>().ok(),
                };
                match ticks {
                    Some(ticks) => {
                        self.day_move = None;
                        // The day number is kept; only the time of day moves.
                        let day = (f.day.ticks / 24_000.0).floor();
                        f.day.ticks = day * 24_000.0 + ticks.rem_euclid(24_000.0);
                        sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::TimeMoved);
                        say(f, format!("Set the time to {}", ticks as i64), WHITE);
                    }
                    None => unknown(f, &line),
                }
            }
            ["time", "add", n] => match n.trim_end_matches('t').parse::<f64>() {
                Ok(n) => {
                    self.day_move = None;
                    f.day.ticks += n;
                    sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::TimeMoved);
                    say(f, format!("Set the time to {}", f.day.ticks.rem_euclid(24_000.0) as i64), WHITE);
                }
                Err(_) => unknown(f, &line),
            },
            ["time", "query", what] => {
                let value = match *what {
                    "daytime" => f.day.ticks.rem_euclid(24_000.0),
                    "gametime" => f.day.ticks,
                    "day" => (f.day.ticks / 24_000.0).floor(),
                    _ => return unknown(f, &line),
                };
                say(f, format!("The time is {}", value as i64), WHITE);
            }
            ["give", rest @ ..] if !rest.is_empty() => {
                // The target is always this player (@p, @s or a name).
                let rest = if rest.len() >= 2 && (rest[0].starts_with('@') || rest[0].eq_ignore_ascii_case("player")) {
                    &rest[1..]
                } else {
                    rest
                };
                let Some(item) = rest.first() else {
                    return unknown(f, &line);
                };
                let id = if item.contains(':') { item.to_ascii_lowercase() } else { format!("minecraft:{}", item.to_ascii_lowercase()) };
                let count = match rest.get(1).map(|n| n.parse::<u32>()) {
                    None => 1,
                    Some(Ok(n)) if (1..=6400).contains(&n) => n,
                    Some(_) => return unknown(f, &line),
                };
                // Leaf litter is gone from this world (his 10-08).
                let known = id != "minecraft:leaf_litter"
                    && f
                    .entities
                    .as_deref()
                    .is_some_and(|e| e.inventory.recipes.item_catalog().is_none_or(|c| c.get(&id).is_some()));
                if !known {
                    say(f, format!("Unknown item '{id}'"), RED);
                    return;
                }
                self.give(f, &id, count);
                let name = id.split(':').nth(1).unwrap_or(&id).replace('_', " ");
                say(f, format!("Gave {count} [{name}] to Player"), WHITE);
            }
            ["kill", target, ..] if target.starts_with("@e") => {
                // Every Minecraft mob within 64 blocks.
                let n = f.entities.as_deref_mut().map_or(0, |e| e.shock(f.feet, 64.0, 100_000.0));
                say(f, format!("Killed {n} entities"), WHITE);
            }
            ["summon", kind, rest @ ..] => {
                let id = if kind.contains(':') { kind.to_ascii_lowercase() } else { format!("minecraft:{}", kind.to_ascii_lowercase()) };
                let at = match rest {
                    [] => Some(f.feet),
                    [x, y, z] => match (coordinate(x, f.feet[0]), coordinate(y, f.feet[1]), coordinate(z, f.feet[2])) {
                        // A whole block number for height is the block's floor.
                        (Some(x), Some(y), Some(z)) => Some([x, if y_relative(y_word(rest)) { y } else { y.floor() }, z]),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(at) = at else {
                    return unknown(f, &line);
                };
                match f.entities.as_deref_mut() {
                    Some(e) => {
                        e.summon(&id, at);
                        let name = id.split(':').nth(1).unwrap_or(&id).replace('_', " ");
                        say(f, format!("Summoned new {name}"), WHITE);
                    }
                    None => say(f, "Unable to summon entity", RED),
                }
            }
            ["kill", ..] => {
                sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::Kill);
                say(f, "Player fell out of the world", WHITE);
            }
            ["tp" | "teleport", rest @ ..] => {
                let rest = if rest.len() == 4 { &rest[1..] } else { rest };
                let [x, y, z] = rest else {
                    return unknown(f, &line);
                };
                let (Some(x), Some(y), Some(z)) =
                    (coordinate(x, f.feet[0]), coordinate(y, f.feet[1]).map(|y| y.floor()), coordinate(z, f.feet[2]))
                else {
                    return unknown(f, &line);
                };
                let to = sim::voxel::to_map(f.origin, [x, y, z]);
                sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::Teleport { to });
                say(f, format!("Teleported Player to {x:.1}, {y:.1}, {z:.1}"), WHITE);
            }
            // Zombies' own: the round, and points.
            ["round", n] => match n.parse::<i32>() {
                Ok(n) if (1..=255).contains(&n) => {
                    sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::Round(n));
                    say(f, format!("Set the round to {n}"), WHITE);
                }
                _ => say(f, format!("Round must be 1 to 255, found {n}"), RED),
            },
            ["points", "set", n] => match n.parse::<i32>() {
                Ok(n) if (0..=10_000_000).contains(&n) => {
                    sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::Points { amount: n, set: true });
                    say(f, format!("Set your points to {n}"), WHITE);
                }
                _ => unknown(f, &line),
            },
            ["points", n] | ["points", "add", n] => match n.parse::<i32>() {
                Ok(n) if n.abs() <= 10_000_000 => {
                    sim::bo2mc::push_player_event(sim::bo2mc::PlayerEvent::Points { amount: n, set: false });
                    say(f, format!("Gave {n} points to Player"), WHITE);
                }
                _ => unknown(f, &line),
            },
            ["seed"] => {
                let seed = f.world.seed;
                say(f, format!("Seed: [{seed}]"), WHITE);
            }
            ["help", ..] => {
                for help in [
                    "/gamemode <survival|creative|adventure|spectator>",
                    "/time set <day|noon|night|midnight|ticks>",
                    "/time add <ticks>",
                    "/time query <daytime|gametime|day>",
                    "/give <item> [count]",
                    "/tp <x> <y> <z>",
                    "/kill",
                    "/kill @e",
                    "/summon <mob> [x y z]",
                    "/round <1-255>",
                    "/points <amount>",
                    "/points set <amount>",
                    "/seed",
                ] {
                    say(f, help, GRAY);
                }
            }
            _ => unknown(f, &line),
        }
    }
}
