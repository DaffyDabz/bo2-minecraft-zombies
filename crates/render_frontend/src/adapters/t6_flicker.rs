//! bo2zm: a Black Ops II map's flickering lamps. The client's `_lights.csc`
//! hands every `light` whose targetname is a built-in mixer behaviour
//! (`fire_flicker`, `electrical_flicker`) to the engine with its script keys
//! as mixer params; the engine then moves the lamp's intensity. Here the lamp's
//! row of the light table gets its colour scaled by intensity / its own
//! intensity, so every surface that lamp lights flickers with it.
//!
//! The behaviours themselves are engine code (no script to read). Fire is
//! BO1's `fire_flicker` script with BO2's absolute intensity range: ramp to a
//! random intensity between min and max over a random delay. Electrical holds
//! at max, then snaps between min and max in a short burst.

use std::sync::Arc;

use asset_world::T6Flicker;
use bevy::prelude::*;

use crate::prepare::scene::world::WorldScene;

/// Rewrite the table at most this often (BO1's script stepped at 10 Hz).
const STEP: f32 = 1.0 / 30.0;

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        Update,
        flicker_t6_lights
            .in_set(frame::RenderSet::Anim)
            .after(super::t6_fog::apply_t6_fog_bank),
    );
}

#[derive(Default)]
struct Lamp {
    now: f32,
    from: f32,
    to: f32,
    t: f32,
    dur: f32,
    /// Electrical: snaps left in this burst, and whether it has held on
    /// since the last one.
    snaps: u32,
    held: bool,
}

#[derive(Default)]
struct State {
    /// The lamps of this table (`t6_lights_loaded`); rebuilt on a new map.
    table: Option<Arc<Vec<[f32; 16]>>>,
    lamps: Vec<Lamp>,
    rng: u32,
    since: f32,
    /// The table last written, to tell a fog bank swap from our own write.
    written: Option<Arc<Vec<[f32; 16]>>>,
}

impl State {
    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, [a, b]: [f32; 2]) -> f32 {
        a + (b - a).max(0.0) * self.rand()
    }
}

fn next_segment(state: &mut State, i: usize, f: &T6Flicker) {
    let now = state.lamps[i].now;
    let (to, dur, snaps) = if !f.electrical {
        (state.range([f.min, f.max]), state.range(f.delay), 0)
    } else if state.lamps[i].snaps > 0 {
        // One snap of the burst: the other end from where it is.
        let off = (now - f.min).abs() > (now - f.max).abs();
        let to = if off { f.min } else { f.max };
        (to, state.range([0.03, f.burst_time.max(0.04)]), state.lamps[i].snaps - 1)
    } else if !state.lamps[i].held {
        // Burst done (or the start): on, and hold.
        state.lamps[i].held = true;
        (f.max, state.range(f.wait), 0)
    } else {
        state.lamps[i].held = false;
        let n = state.range(f.burst).round().max(1.0) as u32;
        (f.min, state.range([0.03, f.burst_time.max(0.04)]), n * 2 - 1)
    };
    let lamp = &mut state.lamps[i];
    lamp.from = now;
    lamp.to = to;
    lamp.t = 0.0;
    lamp.dur = dur.max(0.02);
    lamp.snaps = snaps;
}

fn flicker_t6_lights(
    time: Res<Time>,
    scene: Option<ResMut<WorldScene>>,
    mut state: Local<State>,
) {
    let Some(mut scene) = scene else { return };
    if scene.t6_flicker.is_empty() {
        return;
    }
    let fresh = state
        .table
        .as_ref()
        .is_none_or(|t| !Arc::ptr_eq(t, &scene.t6_lights_loaded));
    if fresh {
        state.table = Some(Arc::clone(&scene.t6_lights_loaded));
        state.rng = 0x9e37_79b9;
        state.lamps = scene
            .t6_flicker
            .iter()
            .map(|f| Lamp { now: f.max, from: f.max, to: f.max, ..Default::default() })
            .collect();
        for (i, f) in scene.t6_flicker.clone().iter().enumerate() {
            next_segment(&mut state, i, f);
        }
        diag::info!(World, "bo2zm flickering lamps: {}", scene.t6_flicker.len());
    }
    let dt = time.delta_secs().min(0.25);
    let flicker = scene.t6_flicker.clone();
    for (i, f) in flicker.iter().enumerate() {
        let mut left = dt;
        while left > 0.0 {
            let lamp = &mut state.lamps[i];
            let step = left.min(lamp.dur - lamp.t);
            lamp.t += step;
            left -= step;
            if lamp.t >= lamp.dur {
                lamp.now = lamp.to;
                next_segment(&mut state, i, f);
            } else {
                // Fire ramps; electrical snaps at once.
                lamp.now = if f.electrical {
                    lamp.to
                } else {
                    lamp.from + (lamp.to - lamp.from) * (lamp.t / lamp.dur)
                };
            }
        }
    }
    // A fog bank swap rewrote the table from the loaded one: write now.
    let swapped = state
        .written
        .as_ref()
        .is_none_or(|w| !Arc::ptr_eq(w, &scene.t6_lights));
    state.since += dt;
    if state.since < STEP && !swapped {
        return;
    }
    state.since = 0.0;
    let loaded = Arc::clone(&scene.t6_lights_loaded);
    let mut table = (*scene.t6_lights).clone();
    for (f, lamp) in flicker.iter().zip(&state.lamps) {
        let row = usize::from(f.light);
        let (Some(out), Some(base)) = (table.get_mut(row), loaded.get(row)) else {
            continue;
        };
        let k = if f.base > 0.0 { (lamp.now / f.base).max(0.0) } else { 1.0 };
        for c in 4..7 {
            out[c] = base[c] * k;
        }
    }
    let table = Arc::new(table);
    state.written = Some(Arc::clone(&table));
    scene.t6_lights = table;
}
