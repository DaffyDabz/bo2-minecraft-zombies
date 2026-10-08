//! The map's moving props (BO2 drives them from a client script,
//! zm_nuked_fx.csc): every map model named "fxanim" plays its scene's
//! animation (washing on the lines, shutters, curtains, cabinets, roaches,
//! the roof vent) 3 s in plus its own `fxanim_wait`; the wires
//! ("wirespark_*") start 2 s later and spark at their end joint every 4 s;
//! the far dust devils ("fxanim_mp_dustdevil") whirl every 15-30 s with a
//! fire effect on their joint and the tornado sound for 12 s; the porch
//! creaks and falls 5000-10000 s in (a long game's 83-167 minutes; the
//! script shares this with other maps: Nuketown Zombies has no porch prop).

use bevy_ecs::prelude::World;

use super::{Zm, with_vm};

/// A scene's animation (the script's `level.scr_anim["fxanim_props"]`).
fn scene_anim(scene: &str) -> Option<&'static str> {
    Some(match scene {
        "pant01_fast" => "fxanim_gp_pant01_fast_anim",
        "shirt01_fast" => "fxanim_gp_shirt01_fast_anim",
        "sheet_med" => "fxanim_gp_cloth_sheet_med_fast_anim",
        "wirespark_long" => "fxanim_gp_wirespark_long_anim",
        "wirespark_med" => "fxanim_gp_wirespark_med_anim",
        "roaches" => "fxanim_gp_roaches_anim",
        "wht_shutters" => "fxanim_zom_nuketown_shutters_anim",
        "wht_shutters02" => "fxanim_zom_nuketown_shutters02_anim",
        "win_curtains" => "fxanim_zom_curtains_anim",
        "cabinets_brwn" => "fxanim_zom_nuketown_cabinets_brwn_anim",
        "cabinets_brwn02" => "fxanim_zom_nuketown_cabinets_brwn02_anim",
        "cabinets_red" => "fxanim_zom_nuketown_cabinets_red_anim",
        "porch" => "fxanim_zom_nuketown_porch_anim",
        "roofvent" => "fxanim_gp_roofvent_small_wobble_anim",
        _ => return None,
    })
}

const DEVIL_ANIM: &str = "fxanim_mp_dustdevil_anim";
const WIRE_FX: &str = "electrical/fx_elec_wire_spark_burst_xsm";

#[derive(Clone, Debug)]
enum Kind {
    Scene,
    /// The joint its spark comes from.
    Wire(&'static str),
    Devil,
    Porch,
}

#[derive(Clone, Debug)]
struct Prop {
    n: u32,
    kind: Kind,
    clip: String,
    len: i64,
    looping: bool,
    /// When its animation (re)started.
    start: Option<i64>,
    /// The next spark / whirl.
    next: i64,
    /// A whirl's fire effect, taken off at that time.
    fire_off: Option<(i64, &'static str)>,
}

#[derive(Default)]
pub(crate) struct FxProps {
    found: bool,
    props: Vec<Prop>,
    seed: u64,
}

impl FxProps {
    fn rand(&mut self) -> u64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.seed >> 33
    }
    /// 15 to 30 s, as `randomfloatrange(15, 30)`.
    fn whirl_wait(&mut self) -> i64 {
        15000 + (self.rand() % 15000) as i64
    }
}

fn find(world: &mut World) {
    let rows: Vec<(u32, String, String, f32)> = with_vm(world, |vm, world| {
        let (tn, scene, wait) = (
            vm.intern("targetname"),
            vm.intern("fxanim_scene_1"),
            vm.intern("fxanim_wait"),
        );
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(_, e)| e.map && e.classname == "script_model")
            .filter_map(|(n, e)| {
                let o = e.obj?;
                let name = vm.to_text(&vm.raw_field(o, tn));
                if name != "fxanim" && name != "fxanim_mp_dustdevil" {
                    return None;
                }
                let s = vm.to_text(&vm.raw_field(o, scene));
                let w = vm.to_text(&vm.raw_field(o, wait)).trim().parse().unwrap_or(0.0);
                Some((*n, name, s, w))
            })
            .collect()
    })
    .unwrap_or_default();
    let mut zm = world.resource_mut::<Zm>();
    let mut props = Vec::new();
    let mut missing = Vec::new();
    for (n, name, scene, wait) in rows {
        let (kind, clip) = if name == "fxanim_mp_dustdevil" {
            (Kind::Devil, DEVIL_ANIM)
        } else {
            let Some(clip) = scene_anim(&scene) else {
                continue;
            };
            let kind = match scene.as_str() {
                "wirespark_long" => Kind::Wire("long_spark_06_jnt"),
                "wirespark_med" => Kind::Wire("med_spark_06_jnt"),
                "porch" => Kind::Porch,
                _ => Kind::Scene,
            };
            (kind, clip)
        };
        let Some(anim) = zm.anims.get(clip) else {
            missing.push(clip);
            continue;
        };
        let len = ((f32::from(anim.numframes) / anim.framerate.max(1.0)) * 1000.0).round() as i64;
        let wait = (wait * 1000.0) as i64;
        let next = match kind {
            Kind::Scene => 3000 + wait,
            Kind::Wire(_) => 5000 + wait,
            Kind::Devil => 3000,
            // `wait randomintrange(5, 10) * 1000` (seconds).
            Kind::Porch => 3000 + wait + (5 + i64::from(n % 5)) * 1_000_000,
        };
        props.push(Prop {
            n,
            looping: anim.looping && !matches!(kind, Kind::Devil),
            kind,
            clip: clip.to_owned(),
            len: len.max(1),
            start: None,
            next,
            fire_off: None,
        });
    }
    missing.sort_unstable();
    missing.dedup();
    diag::info!(
        Sim,
        "bo2zm t6 moving props: {} ({} whirls, {} wires, {} porches){}",
        props.len(),
        props.iter().filter(|p| matches!(p.kind, Kind::Devil)).count(),
        props.iter().filter(|p| matches!(p.kind, Kind::Wire(_))).count(),
        props.iter().filter(|p| matches!(p.kind, Kind::Porch)).count(),
        if missing.is_empty() {
            String::new()
        } else {
            format!("; animations not loaded: {missing:?}")
        }
    );
    zm.fxprops.found = true;
    zm.fxprops.seed = 0x5eed_f00d;
    zm.fxprops.props = props;
}

/// Starts, sparks and whirls the props (after the models are shown, so
/// effects bolt to them).
pub(super) fn advance(world: &mut World, now: i64) {
    if !world.resource::<Zm>().fxprops.found {
        if now < 1000 {
            return;
        }
        find(world);
    }
    let mut sparks: Vec<(u32, &'static str, &'static str, bool)> = Vec::new();
    let mut sounds: Vec<(u32, &'static str)> = Vec::new();
    {
        let mut zm = world.resource_mut::<Zm>();
        let mut fx = std::mem::take(&mut zm.fxprops);
        for i in 0..fx.props.len() {
            if let Some((off, name)) = fx.props[i].fire_off
                && now >= off
            {
                sparks.push((fx.props[i].n, name, "dervish_jnt", true));
                fx.props[i].fire_off = None;
            }
            if now < fx.props[i].next {
                continue;
            }
            match fx.props[i].kind {
                Kind::Scene => {
                    fx.props[i].start = Some(now);
                    fx.props[i].next = i64::MAX;
                }
                Kind::Porch => {
                    fx.props[i].start = Some(now);
                    fx.props[i].next = i64::MAX;
                    sounds.push((fx.props[i].n, "zmb_porch_collapse"));
                }
                Kind::Wire(joint) => {
                    if fx.props[i].start.is_none() {
                        fx.props[i].start = Some(now);
                    } else {
                        sparks.push((fx.props[i].n, WIRE_FX, joint, false));
                    }
                    // `randomintrange(4, 5)`: always 4.
                    fx.props[i].next = now + 4000;
                }
                Kind::Devil => {
                    if fx.props[i].start.is_some() || fx.props[i].next > 3000 {
                        let fire = if fx.rand() % 2 == 0 {
                            "maps/zombie/fx_zmb_fire_devil_sm"
                        } else {
                            "maps/zombie/fx_zmb_fire_devil_lg"
                        };
                        fx.props[i].start = Some(now);
                        fx.props[i].fire_off = Some((now + 12000, fire));
                        sparks.push((fx.props[i].n, fire, "dervish_jnt", false));
                        sounds.push((fx.props[i].n, "amb_fire_tornado"));
                    }
                    let w = fx.whirl_wait();
                    fx.props[i].next = now + w;
                }
            }
        }
        zm.fxprops = fx;
    }
    for (n, name, joint, stop) in sparks {
        super::natives_fx::bolt_effect(world, n, name, joint, stop);
    }
    for (n, alias) in sounds {
        let origin = world.resource::<Zm>().ents.get(&n).map(|e| e.origin);
        if let Some(origin) = origin {
            if alias == "zmb_porch_collapse" {
                diag::info!(Sim, "bo2zm t6 the porch falls at {origin:?}");
            }
            super::natives_fx::sound(world, crate::EventAudience::All, alias, origin);
        }
    }
}

/// A prop's playing animation: (clip, fraction, rate per second).
pub(super) fn present(world: &World, n: u32, now: i64) -> Option<(String, f32, f32)> {
    let zm = world.resource::<Zm>();
    let p = zm.fxprops.props.iter().find(|p| p.n == n)?;
    let start = p.start?;
    let elapsed = (now - start).max(0);
    if p.looping {
        Some((
            p.clip.clone(),
            (elapsed % p.len) as f32 / p.len as f32,
            1000.0 / p.len as f32,
        ))
    } else if elapsed >= p.len {
        Some((p.clip.clone(), 1.0, 0.0))
    } else {
        Some((
            p.clip.clone(),
            elapsed as f32 / p.len as f32,
            1000.0 / p.len as f32,
        ))
    }
}
