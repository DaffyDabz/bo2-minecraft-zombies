//! bo2mc - the one switch for Minecraft Zombies: whether the next (and the
//! current) load of Black Ops II's `t6:zm_nuked` stands on a Minecraft world.
//!
//! The front end's map pick sets it at START MATCH (MINECRAFT on, NUKETOWN
//! off) before the map loads, so one session can go between the two. With
//! no pick yet it is `IW4L_BO2MC=1` from the environment (a straight
//! `map t6:zm_nuked` start, tests).
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// 0 = not set by a pick (the environment decides), 1 = off, 2 = on.
static PICKED: AtomicU8 = AtomicU8::new(0);

fn from_env() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("IW4L_BO2MC").is_ok_and(|v| v == "1"))
}

/// Minecraft Zombies is on.
pub fn enabled() -> bool {
    match PICKED.load(Ordering::Acquire) {
        1 => false,
        2 => true,
        _ => from_env(),
    }
}

/// The front end's pick: on for MINECRAFT, off for NUKETOWN. Set before
/// the map loads; the load reads it.
pub fn set_enabled(on: bool) {
    PICKED.store(if on { 2 } else { 1 }, Ordering::Release);
}
