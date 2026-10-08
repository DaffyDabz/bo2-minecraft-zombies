//! Thrown grenades as the scripts see them (BO2's engine side): a player's
//! throw becomes a script entity ("grenade") and the player is notified
//! `grenade_fire` (grenade, weapon name); the entity follows the grenade,
//! is notified `stationary` when it comes to rest and `explode` (origin)
//! then `death` when it goes off. The Monkey Bomb lives on this: its script
//! hides the grenade, links the monkey model to it, waits for it to land,
//! then claps, sings and draws the zombies until the fuse it resets runs out.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, Value};

use super::{Ent, Zm, frame, with_vm};
use crate::ProjectileId;

#[derive(Clone, Debug)]
pub(crate) struct Thrown {
    id: ProjectileId,
    entnum: i32,
    weapon: u32,
    n: u32,
    stationary: bool,
    hidden: bool,
    /// The last zombie-distance log line (IW4L_T6_SNDLOG).
    logged: i64,
}

/// Far enough ahead that the grenade is never drawn (the client skips a
/// missile until its launch time).
const NEVER_DRAWN_MS: i32 = 1 << 30;

pub(super) fn advance(world: &mut World, now: i64) {
    // A grenade that went off last tick: its watchers read its fields at
    // that frame's end (the bookmark watcher does), so it goes away now.
    let free: Vec<gsc_t6::ObjRef> = {
        let mut zm = world.resource_mut::<Zm>();
        let (due, keep) = std::mem::take(&mut zm.thrown_free)
            .into_iter()
            .partition(|(_, at)| now >= *at);
        zm.thrown_free = keep;
        due.into_iter().map(|(o, _)| o).collect()
    };
    if !free.is_empty() {
        with_vm(world, |vm, world| {
            for o in free {
                if vm.alive(o) {
                    vm.free_object(world, o);
                }
            }
        });
    }
    let live: Vec<crate::ProjectileState> = crate::frame::collect_projectiles(world)
        .into_iter()
        .filter(|p| p.live)
        .collect();
    // Gone off (or cleaned up): `explode`, then `death`, then away.
    let gone: Vec<Thrown> = {
        let mut zm = world.resource_mut::<Zm>();
        let (gone, keep) = std::mem::take(&mut zm.thrown)
            .into_iter()
            .partition(|t| !live.iter().any(|p| p.id == t.id));
        zm.thrown = keep;
        gone
    };
    for t in gone {
        let (obj, origin) = {
            let mut zm = world.resource_mut::<Zm>();
            let e = zm.ents.remove(&t.n);
            zm.movers.list.remove(&t.n);
            (e.as_ref().and_then(|e| e.obj), e.map_or([0.0; 3], |e| e.origin))
        };
        let Some(o) = obj else { continue };
        with_vm(world, |vm, world| {
            if vm.alive(o) {
                vm.notify_str(world, o, "explode", &[Value::Vec3(origin)]);
                vm.notify_str(world, o, "death", &[]);
            }
        });
        world.resource_mut::<Zm>().thrown_free.push((o, now + 1));
    }
    // Follow the rest; tell the scripts when one comes to rest.
    let tracked: Vec<Thrown> = world.resource::<Zm>().thrown.clone();
    for t in &tracked {
        let Some(p) = live.iter().find(|p| p.id == t.id) else {
            continue;
        };
        let at = p.origin_at(now as i32);
        let (obj, hidden) = {
            let mut zm = world.resource_mut::<Zm>();
            let Some(e) = zm.ents.get_mut(&t.n) else {
                continue;
            };
            e.origin = at;
            e.angles = p.apos.tr_base;
            (e.obj, e.hidden)
        };
        if hidden && !t.hidden {
            let mut f = frame(world);
            if let Some(p) = f.projectile_mut_by_number(t.entnum).filter(|p| p.id == t.id) {
                p.launch_time = (now as i32).saturating_add(NEVER_DRAWN_MS);
            }
        }
        let rest = p.grounded && !t.stationary;
        {
            let mut zm = world.resource_mut::<Zm>();
            if let Some(x) = zm.thrown.iter_mut().find(|x| x.id == t.id) {
                x.hidden |= hidden;
                x.stationary |= p.grounded;
            }
        }
        if t.stationary && now - t.logged >= 1000 && std::env::var_os("IW4L_T6_SNDLOG").is_some() {
            log_zombies(world, t.n, at, now);
        }
        if rest && let Some(o) = obj {
            with_vm(world, |vm, world| {
                if vm.alive(o) {
                    vm.notify_str(world, o, "stationary", &[]);
                }
            });
        }
    }
    // New throws: an entity for each and `grenade_fire` to the thrower.
    for p in &live {
        if world.resource::<Zm>().thrown.iter().any(|t| t.id == p.id) {
            continue;
        }
        let Some(player) = world.resource::<Zm>().players.get(&p.owner.0).map(|x| x.obj) else {
            continue;
        };
        let is_grenade = frame(world)
            .combat_facts_for(p.weapon)
            .is_some_and(|f| f.weap_type == weapon_iw4::WEAPTYPE_GRENADE);
        if !is_grenade {
            continue;
        }
        let name = super::weapon_text(world, p.weapon);
        let at = p.origin_at(now as i32);
        let n = world.resource_mut::<Zm>().alloc_entnum();
        with_vm(world, |vm, world| {
            let obj = vm.alloc_object(ObjKind::Entity(n));
            world.resource_mut::<Zm>().ents.insert(
                n,
                Ent {
                    obj: Some(obj),
                    classname: "grenade".to_owned(),
                    origin: at,
                    angles: p.apos.tr_base,
                    ..Default::default()
                },
            );
            let owner = vm.strings.intern("owner");
            vm.set_field(world, obj, owner, Value::Object(player));
            let weaponname = vm.strings.intern("weaponname");
            let w = vm.string(&name);
            vm.set_field(world, obj, weaponname, w.clone());
            vm.notify_str(world, player, "grenade_fire", &[Value::Object(obj), w]);
        });
        if std::env::var_os("IW4L_T6_SNDLOG").is_some() {
            diag::info!(Sim, "bo2zm t6 grenade_fire {name} ent{n}");
        }
        world.resource_mut::<Zm>().thrown.push(Thrown {
            id: p.id,
            entnum: p.entnum,
            weapon: p.weapon,
            n,
            stationary: false,
            hidden: false,
            logged: 0,
        });
    }
}

/// Test log: how near the closest zombies are to a grenade at rest (the
/// Monkey Bomb draws them in).
fn log_zombies(world: &mut World, n: u32, at: [f32; 3], now: i64) {
    let mut zm = world.resource_mut::<Zm>();
    if let Some(x) = zm.thrown.iter_mut().find(|x| x.n == n) {
        x.logged = now;
    }
    let mut d: Vec<i32> = zm
        .actors
        .keys()
        .filter_map(|a| zm.ents.get(a))
        .map(|e| {
            let v = [e.origin[0] - at[0], e.origin[1] - at[1], e.origin[2] - at[2]];
            (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() as i32
        })
        .collect();
    d.sort_unstable();
    d.truncate(6);
    diag::info!(Sim, "bo2zm t6 grenade ent{n} at rest, nearest zombies {d:?} t={now}");
    // The monkey the script put on it: where, whether shown, its clip.
    let models: Vec<String> = zm
        .ents
        .iter()
        .filter(|(_, e)| e.model.contains("monkey"))
        .map(|(k, e)| {
            format!(
                "ent{k} {} at {:?} hidden={} invisible_to_all={} link={:?} anim={:?}",
                e.model,
                e.origin.map(|x| x as i32),
                e.hidden,
                e.invisible_to_all,
                e.link.map(|l| l.0),
                e.anim
            )
        })
        .collect();
    diag::info!(Sim, "bo2zm t6 grenade ent{n} monkey models {models:?}");
    // What the scripts made of it: the grenade as a point of interest, and
    // whether the zombies were sent to it (`enemyoverride`) and where.
    let probe = with_vm(world, |vm, world| {
        let zm = world.resource::<Zm>();
        let text = |vm: &gsc_t6::Vm<World>, o: gsc_t6::ObjRef, f: &str| -> String {
            let k = vm.strings.find(f);
            match k.map(|k| vm.raw_field(o, k)) {
                Some(Value::Array(a)) => format!("[{} items]", a.read().len()),
                Some(v) => vm.to_text(&v),
                None => "-".to_owned(),
            }
        };
        let g = zm.ents.get(&n).and_then(|e| e.obj).map(|o| {
            ["script_noteworthy", "poi_active", "poi_radius", "num_poi_attracts", "attract_to_origin", "attractor_array", "attractor_positions", "claimed_attractor_positions"]
                .map(|f| format!("{f}={}", text(vm, o, f)))
                .join(" ")
        });
        let mut zs: Vec<(i32, String)> = zm
            .actors
            .iter()
            .filter(|(_, a)| a.alive)
            .filter_map(|(k, a)| {
                let e = zm.ents.get(k)?;
                let o = a.obj?;
                let v = [e.origin[0] - at[0], e.origin[1] - at[1]];
                let dist = (v[0] * v[0] + v[1] * v[1]).sqrt() as i32;
                Some((
                    dist,
                    format!(
                        "{dist}: override={} ignoreall={} goal={:?} script={} ai_state={}",
                        text(vm, o, "enemyoverride"),
                        text(vm, o, "ignoreall"),
                        a.goal.map(|g| g.map(|x| x as i32)),
                        a.script,
                        text(vm, o, "ai_state")
                    ),
                ))
            })
            .collect();
        zs.sort_by_key(|z| z.0);
        (g, zs.into_iter().take(3).map(|z| z.1).collect::<Vec<_>>())
    });
    if let Some((g, zs)) = probe {
        diag::info!(Sim, "bo2zm t6 grenade ent{n} as a point of interest: {g:?}; zombies {zs:?}");
    }
}

/// `grenade resetmissiledetonationtime([seconds])`: the fuse starts again
/// (the weapon's own fuse when no time is given).
pub(super) fn reset_fuse(world: &mut World, n: u32, secs: Option<f32>, now: i64) {
    let Some(t) = world.resource::<Zm>().thrown.iter().find(|t| t.n == n).cloned() else {
        return;
    };
    let mut f = frame(world);
    let fuse = match secs {
        Some(s) => (s * 1000.0) as i32,
        None => f.equipment_facts_for(t.weapon).map_or(0, |x| x.fuse_time_ms),
    };
    if fuse <= 0 {
        return;
    }
    if let Some(p) = f.projectile_mut_by_number(t.entnum).filter(|p| p.id == t.id) {
        p.detonate_at_ms = Some((now as i32).saturating_add(fuse));
        p.cleanup_at_ms = p.cleanup_at_ms.max((now as i32).saturating_add(fuse + 1000));
    }
}
