//! bo2zm: Black Ops II's room echo. Each map room has an echo preset from
//! the sound banks (`SndRadverb`: early reflections, a late tail, filters);
//! every sound sends its alias's share (`reverbSend`) into the echo of the
//! room the player is in when it starts. Each voice carries its own small
//! reverb (early taps, four damped combs and two allpasses a side) and plays
//! on for the tail after its clip ends.

use std::sync::{Arc, RwLock};

use bevy::prelude::*;
use net::{LocalPresentClient, PresentedSnapshot};

use crate::playback::SoundBank;

/// One room's echo, independent of sample rate.
#[derive(Clone, Debug, PartialEq)]
pub struct EchoParams {
    /// Early reflections: the last tap's delay (ms) and their level.
    pub early_ms: f32,
    pub early_gain: f32,
    /// Late tail: time to fall 60 dB (s) and its level.
    pub decay_s: f32,
    pub late_gain: f32,
    /// The whole echo's level (the room's wet times the preset's return).
    pub wet: f32,
    /// Low-pass on what goes in and on the tail as it rings (Hz).
    pub input_lpf: f32,
    pub damp_lpf: f32,
    /// Room size: scales the comb and allpass lengths (1 = Freeverb's).
    pub size: f32,
    /// Allpass feedback (smears the echo; 0.5 = Freeverb's).
    pub diffusion: f32,
}

static CURRENT: RwLock<Option<Arc<EchoParams>>> = RwLock::new(None);

/// The echo of the room the local player is in now.
pub fn current() -> Option<Arc<EchoParams>> {
    CURRENT.read().ok().and_then(|g| g.clone())
}

pub fn set(params: Option<Arc<EchoParams>>) {
    if let Ok(mut g) = CURRENT.write() {
        *g = params;
    }
}

/// Freeverb's lengths at 44.1 kHz; the right side is 23 samples longer.
const COMBS: [usize; 4] = [1116, 1277, 1422, 1557];
const ALLPASSES: [usize; 2] = [556, 341];
const SPREAD: usize = 23;
/// Early taps: where (share of `early_ms`) and how loud, alternating sides.
const TAPS: [(f32, f32); 6] = [
    (0.19, 0.85),
    (0.33, 0.7),
    (0.47, 0.6),
    (0.62, 0.5),
    (0.81, 0.4),
    (1.0, 0.32),
];
/// The longest tail a voice keeps playing for after its clip (s).
const MAX_TAIL_S: f32 = 3.0;

struct Comb {
    buf: Vec<f32>,
    i: usize,
    feedback: f32,
    filt: f32,
}

impl Comb {
    #[inline]
    fn tick(&mut self, x: f32, damp: f32) -> f32 {
        let out = self.buf[self.i];
        self.filt = out * (1.0 - damp) + self.filt * damp;
        self.buf[self.i] = x + self.filt * self.feedback;
        self.i += 1;
        if self.i == self.buf.len() {
            self.i = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    i: usize,
}

impl Allpass {
    #[inline]
    fn tick(&mut self, x: f32, g: f32) -> f32 {
        let b = self.buf[self.i];
        self.buf[self.i] = x + b * g;
        self.i += 1;
        if self.i == self.buf.len() {
            self.i = 0;
        }
        b - x
    }
}

/// One voice's echo.
pub struct Echo {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    early: Vec<f32>,
    early_i: usize,
    taps: Vec<(usize, f32)>,
    in_lp: f32,
    in_coef: f32,
    damp: f32,
    diffusion: f32,
    early_gain: f32,
    late_gain: f32,
    send: f32,
    tail_frames: usize,
}

fn lowpass_coef(hz: f32, rate: f32) -> f32 {
    if hz <= 0.0 || hz >= rate * 0.5 {
        return 0.0;
    }
    (-std::f32::consts::TAU * hz / rate).exp()
}

impl Echo {
    /// `send`: the alias's share into the echo (0..1).
    pub fn new(p: &EchoParams, sample_rate: u32, send: f32) -> Self {
        let rate = sample_rate.max(8000) as f32;
        let scale = rate / 44_100.0 * p.size.clamp(0.25, 2.5);
        let decay = p.decay_s.clamp(0.05, 10.0);
        let combs = std::array::from_fn(|side| {
            COMBS
                .iter()
                .map(|&n| {
                    let len = (((n + side * SPREAD) as f32 * scale) as usize).max(16);
                    Comb {
                        buf: vec![0.0; len],
                        i: 0,
                        feedback: 10f32.powf(-3.0 * len as f32 / (decay * rate)),
                        filt: 0.0,
                    }
                })
                .collect()
        });
        let allpasses = std::array::from_fn(|side| {
            ALLPASSES
                .iter()
                .map(|&n| Allpass {
                    buf: vec![0.0; (((n + side * SPREAD) as f32 * scale) as usize).max(8)],
                    i: 0,
                })
                .collect()
        });
        let early_len = ((p.early_ms.clamp(1.0, 200.0) * 0.001 * rate) as usize).max(2) + 1;
        let taps = TAPS
            .iter()
            .map(|&(at, g)| (((early_len - 1) as f32 * at) as usize, g))
            .collect();
        Self {
            combs,
            allpasses,
            early: vec![0.0; early_len],
            early_i: 0,
            taps,
            in_lp: 0.0,
            in_coef: lowpass_coef(p.input_lpf, rate),
            damp: lowpass_coef(p.damp_lpf, rate),
            diffusion: p.diffusion.clamp(0.0, 0.9),
            early_gain: p.early_gain.max(0.0) * p.wet.max(0.0),
            // Four combs a side ring together; this keeps a 0 dB late tail
            // near the dry sound's level.
            late_gain: p.late_gain.max(0.0) * p.wet.max(0.0) * 0.12,
            send: send.clamp(0.0, 1.0),
            tail_frames: (decay.min(MAX_TAIL_S) * rate) as usize,
        }
    }

    /// Frames the echo rings on for after the clip ends.
    pub fn tail_frames(&self) -> usize {
        self.tail_frames
    }

    /// One frame: the dry sound in (mono), the echo out (left, right).
    #[inline]
    pub fn tick(&mut self, x: f32) -> (f32, f32) {
        self.in_lp = x * self.send * (1.0 - self.in_coef) + self.in_lp * self.in_coef;
        let x = self.in_lp;
        self.early[self.early_i] = x;
        let n = self.early.len();
        let (mut el, mut er) = (0.0, 0.0);
        for (k, &(d, g)) in self.taps.iter().enumerate() {
            let s = self.early[(self.early_i + n - d) % n] * g;
            if k % 2 == 0 {
                el += s;
            } else {
                er += s;
            }
        }
        self.early_i = (self.early_i + 1) % n;
        let mut out = [0.0f32; 2];
        for (side, o) in out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for c in &mut self.combs[side] {
                sum += c.tick(x, self.damp);
            }
            for a in &mut self.allpasses[side] {
                sum = a.tick(sum, self.diffusion);
            }
            *o = sum * self.late_gain;
        }
        (
            out[0] + el * self.early_gain,
            out[1] + er * self.early_gain,
        )
    }
}

/// A bank preset (`SndRadverb`'s 16 values) with the room's wet level.
/// The bank keeps them as the sound designers' dial positions: gains are
/// linear, the filters 0..1 (0 = off), sizes about 3 (a room) to 12 (the
/// open street). Nuketown's house: early 0.87, late 0.82, return 0.5,
/// tail 0.37; outdoors: early 0.33, late 0.12.
pub fn params_from(v: &[f32; 16], wet: f32) -> EchoParams {
    let [
        _smoothing,
        early_time,
        late_time,
        early_gain,
        late_gain,
        return_gain,
        _early_lpf,
        _late_lpf,
        input_lpf,
        damp_lpf,
        _wall_reflect,
        _dry_gain,
        _early_size,
        late_size,
        diffusion,
        _return_highpass,
    ] = *v;
    EchoParams {
        early_ms: early_time.clamp(0.0, 1.0) * 80.0,
        early_gain: early_gain.max(0.0),
        decay_s: late_time.max(0.0) * 1.6,
        late_gain: late_gain.max(0.0),
        wet: return_gain.max(0.0) * wet,
        input_lpf: dial_hz(input_lpf),
        damp_lpf: dial_hz(damp_lpf),
        size: late_size / 4.0,
        diffusion: diffusion.clamp(0.0, 1.0) * 0.7,
    }
}

/// A 0..1 filter dial as a cutoff: 0 is off, 0.5 about 3 kHz, 1 20 kHz.
fn dial_hz(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    500.0 * 40f32.powf(v.min(1.0))
}

/// Follow the room the server says the local player is in (client dvar
/// `bo2zm_room`, empty = the map's default room) and switch the echo new
/// sounds send into.
pub(crate) fn update(
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    bank: Option<Res<SoundBank>>,
    mut last: Local<Option<String>>,
) {
    let (Some(presented), Some(local), Some(bank)) = (presented, local, bank) else {
        return;
    };
    let Some(snap) = presented.snapshot() else {
        return;
    };
    let room = snap
        .meta
        .script_dvars(local.0)
        .string("bo2zm_room")
        .unwrap_or_default();
    let rooms = bank.0.scripted_map_fx.as_ref().map(|fx| fx.rooms.as_slice()).unwrap_or_default();
    let row = rooms
        .iter()
        .find(|r| !room.is_empty() && r.0 == room)
        .or_else(|| rooms.first());
    let key = row.map(|r| format!("{} {}", r.0, r.1)).unwrap_or_default();
    if last.as_deref() == Some(key.as_str()) {
        return;
    }
    let params = row.and_then(|r| {
        let v = bank.0.radverbs.get(&r.1)?;
        Some(Arc::new(params_from(v, r.3)))
    });
    diag::info!(Audio, "audio: room echo `{key}` -> {params:?}");
    set(params);
    *last = Some(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> EchoParams {
        EchoParams {
            early_ms: 30.0,
            early_gain: 0.5,
            decay_s: 0.8,
            late_gain: 1.0,
            wet: 1.0,
            input_lpf: 8000.0,
            damp_lpf: 5000.0,
            size: 1.0,
            diffusion: 0.5,
        }
    }

    #[test]
    fn rings_then_dies() {
        let mut e = Echo::new(&room(), 44_100, 1.0);
        let (mut energy_early, mut energy_late) = (0.0f64, 0.0f64);
        for i in 0..e.tail_frames() {
            let (l, r) = e.tick(if i < 441 { 0.5 } else { 0.0 });
            assert!(l.is_finite() && r.is_finite());
            let p = f64::from(l * l + r * r);
            if i < 22_050 {
                energy_early += p;
            } else {
                energy_late += p;
            }
        }
        assert!(energy_early > 0.0);
        assert!(energy_late < energy_early * 0.05, "{energy_early} {energy_late}");
    }

    /// Nuketown's house preset against a noise burst: the echo's energy
    /// against the dry sound's (printed; `cargo test -- --nocapture`).
    #[test]
    fn house_level() {
        let house = [
            0.525, 0.55, 0.37, 0.87, 0.816, 0.5, 0.57, 0.595, 0.455, 0.495, 0.39, 0.0, 5.66,
            3.84, 0.16, 0.0,
        ];
        let p = params_from(&house, 1.0);
        let mut e = Echo::new(&p, 44_100, 1.0);
        let (mut dry, mut wet) = (0.0f64, 0.0f64);
        let mut seed = 1u32;
        for i in 0..(4410 + e.tail_frames()) {
            let x = if i < 4410 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            } else {
                0.0
            };
            dry += f64::from(x * x) * 2.0;
            let (l, r) = e.tick(x);
            wet += f64::from(l * l + r * r);
        }
        let ratio_db = 10.0 * (wet / dry).log10();
        println!("house: {p:?} echo/dry {ratio_db:.1} dB, tail {} frames", e.tail_frames());
        assert!((-20.0..0.0).contains(&ratio_db), "{ratio_db}");
    }

    #[test]
    fn no_send_is_silent() {
        let mut e = Echo::new(&room(), 48_000, 0.0);
        for _ in 0..10_000 {
            assert_eq!(e.tick(0.7), (0.0, 0.0));
        }
    }
}
