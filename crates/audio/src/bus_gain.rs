//! bo2zm: Black Ops II's Sound sliders as gains on its sound buses. Each
//! alias names its bus in flags bits 11-14 (measured on Nuketown's banks:
//! 1 game, 2 voice, 4 hdr effects, 5 menu, 6 music; 7 is the movie bus):
//! music and voice have their own sliders, the movie bus the cinematics
//! one, everything else is effects. The master slider stays the global
//! volume.

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::prelude::*;

/// Music, effects, voice, cinematics (f32 bits; 1.0 until set).
static GAINS: [AtomicU32; 4] = [const { AtomicU32::new(0x3f80_0000) }; 4];
const MUSIC: usize = 0;
const SFX: usize = 1;
const VOICE: usize = 2;
const MOVIE: usize = 3;

fn gain(i: usize) -> f32 {
    f32::from_bits(GAINS[i].load(Ordering::Relaxed))
}

/// The music slider.
pub(crate) fn music() -> f32 {
    gain(MUSIC)
}

/// The effects slider.
pub(crate) fn sfx() -> f32 {
    gain(SFX)
}

/// A Black Ops II alias's slider, from its flags.
pub(crate) fn t6_alias(flags: Option<u32>) -> f32 {
    match flags.map(|f| (f >> 11) & 0xf) {
        Some(6) => gain(MUSIC),
        Some(2) => gain(VOICE),
        Some(7) => gain(MOVIE),
        Some(_) => gain(SFX),
        None => 1.0,
    }
}

/// His settings into the gains, every frame (cheap).
pub(crate) fn sync(settings: Option<Res<frame::GameSettings>>) {
    let Some(s) = settings else {
        return;
    };
    for (i, v) in [s.music_volume, s.sfx_volume, s.voice_volume, s.cinematic_volume]
        .into_iter()
        .enumerate()
    {
        GAINS[i].store(v.to_bits(), Ordering::Relaxed);
    }
}
