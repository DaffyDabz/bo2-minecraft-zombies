//! The map's sound triggers (every map's client script,
//! clientscripts/mp/_audio.csc, runs them for its own player): walking into
//! an "audio_bump_trigger" plays its sound (Nuketown's cars, truck, boxes
//! and fences) once per entry; sprinting into an "audio_step_trigger"
//! plays its `script_sound` (the truck bed's clunk on), sprinting out its
//! `script_noteworthy` (off). Each is heard by that player only, as the
//! client script plays it for its own player. Inside a step trigger his
//! footsteps also play its `script_label` (`setsteptriggersound`), told to
//! his game as the client dvar `bo2zm_step`.
//!
//! The map's "ambient_package" triggers are its rooms
//! (`script_ambientroom`, `script_ambientpriority`): a player inside one is
//! in that room, elsewhere in the default room. The lowest priority number
//! wins: BO2's engine runs these triggers (`level.usecodetriggers`, so the
//! script's own highest-wins pick never runs), and Nuketown's default room
//! has a trigger over the whole map at 99 with every house, the garage and
//! the truck at 2; highest-wins would never let a house be heard.
//! His game is told which (client dvar `bo2zm_room`) and plays that room's
//! echo on what he hears.

use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::World;

use super::{Zm, frame, with_vm};
use crate::EventAudience;
use crate::world::ClientId;

#[derive(Clone, Debug)]
struct Trig {
    n: u32,
    step: bool,
    enter: String,
    leave: Option<String>,
    /// A step trigger's `script_label`: while a player is inside, each of
    /// his footsteps also plays it (the porch boards, the truck bed).
    label: Option<String>,
}

#[derive(Clone, Debug)]
struct Room {
    n: u32,
    room: String,
    priority: i32,
}

#[derive(Default)]
pub(crate) struct AudioTrigs {
    found: bool,
    trigs: Vec<Trig>,
    rooms: Vec<Room>,
    /// (client, trigger) pairs inside last frame.
    inside: HashSet<(u32, u32)>,
    /// Each player's room as his game was last told ("" = the default).
    room_of: HashMap<u32, String>,
    /// Each player's step sound as his game was last told ("" = none).
    step_of: HashMap<u32, String>,
}

fn find(world: &mut World) {
    let trigs: Vec<Trig> = with_vm(world, |vm, world| {
        let (tn, snd, note, label) = (
            vm.intern("targetname"),
            vm.intern("script_sound"),
            vm.intern("script_noteworthy"),
            vm.intern("script_label"),
        );
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(_, e)| e.map && e.classname == "trigger_multiple")
            .filter_map(|(n, e)| {
                let o = e.obj?;
                let step = match vm.to_text(&vm.raw_field(o, tn)).as_str() {
                    "audio_bump_trigger" => false,
                    "audio_step_trigger" => true,
                    _ => return None,
                };
                let enter = vm.to_text(&vm.raw_field(o, snd));
                let leave = Some(vm.to_text(&vm.raw_field(o, note))).filter(|s| !s.is_empty());
                let label = Some(vm.to_text(&vm.raw_field(o, label)))
                    .filter(|s| step && !s.is_empty());
                Some(Trig {
                    n: *n,
                    step,
                    enter,
                    leave,
                    label,
                })
            })
            .collect()
    })
    .unwrap_or_default();
    let rooms: Vec<Room> = with_vm(world, |vm, world| {
        let (tn, room, pri) = (
            vm.intern("targetname"),
            vm.intern("script_ambientroom"),
            vm.intern("script_ambientpriority"),
        );
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(_, e)| e.map && e.classname == "trigger_multiple")
            .filter_map(|(n, e)| {
                let o = e.obj?;
                if vm.to_text(&vm.raw_field(o, tn)) != "ambient_package" {
                    return None;
                }
                let room = vm.to_text(&vm.raw_field(o, room));
                (!room.is_empty()).then(|| Room {
                    n: *n,
                    room,
                    priority: vm.to_text(&vm.raw_field(o, pri)).trim().parse().unwrap_or(1),
                })
            })
            .collect()
    })
    .unwrap_or_default();
    diag::info!(
        Sim,
        "bo2zm t6 sound triggers: {} bumps, {} steps, {} rooms",
        trigs.iter().filter(|t| !t.step).count(),
        trigs.iter().filter(|t| t.step).count(),
        rooms.len()
    );
    for r in &rooms {
        let e = world.resource::<Zm>().ents.get(&r.n).cloned();
        let bounds = e.as_ref().and_then(|e| {
            let n = super::triggers::brush_index(e)?;
            let f = frame(world);
            let m = f.clip_cmodels().models.get(n as usize)?;
            Some((
                std::array::from_fn::<i32, 3, _>(|i| (e.origin[i] + m.mins[i]) as i32),
                std::array::from_fn::<i32, 3, _>(|i| (e.origin[i] + m.maxs[i]) as i32),
            ))
        });
        diag::info!(Sim, "bo2zm t6 room {} priority {} ent{} bounds {bounds:?}", r.room, r.priority, r.n);
    }
    let mut zm = world.resource_mut::<Zm>();
    zm.audio_trigs.found = true;
    zm.audio_trigs.trigs = trigs;
    zm.audio_trigs.rooms = rooms;
}

/// Which room each player is in; his game is told when it changes.
fn rooms(world: &mut World, players: &[u32]) {
    let rooms = world.resource::<Zm>().audio_trigs.rooms.clone();
    if rooms.is_empty() {
        return;
    }
    for &client in players {
        let Some((mins, maxs)) = super::triggers::player_box(world, client) else {
            continue;
        };
        let mut best: Option<&Room> = None;
        for r in &rooms {
            let Some(e) = world.resource::<Zm>().ents.get(&r.n).cloned() else {
                continue;
            };
            if super::triggers::touches(world, &e, mins, maxs)
                && best.is_none_or(|b| r.priority < b.priority)
            {
                best = Some(r);
            }
        }
        let room = best.map_or(String::new(), |r| r.room.clone());
        let was = world.resource::<Zm>().audio_trigs.room_of.get(&client).cloned();
        if was.as_ref() != Some(&room) {
            diag::info!(Sim, "bo2zm t6 room: player {client} now in {:?}", room);
            super::set_client_dvar(world, client, "bo2zm_room", &room);
            world.resource_mut::<Zm>().audio_trigs.room_of.insert(client, room);
        }
    }
}

pub(super) fn advance(world: &mut World, now: i64) {
    if !world.resource::<Zm>().audio_trigs.found {
        // After the map's entities are up (as the moving props wait).
        if now < 1000 {
            return;
        }
        find(world);
    }
    let players: Vec<u32> = world
        .resource::<Zm>()
        .players
        .iter()
        .filter(|(_, p)| p.sessionstate == "playing")
        .map(|(c, _)| *c)
        .collect();
    rooms(world, &players);
    let trigs = world.resource::<Zm>().audio_trigs.trigs.clone();
    if trigs.is_empty() {
        return;
    }
    let mut now_inside = HashSet::new();
    let mut plays: Vec<(u32, String, [f32; 3])> = Vec::new();
    let mut steps: Vec<(u32, String)> = Vec::new();
    let was = std::mem::take(&mut world.resource_mut::<Zm>().audio_trigs.inside);
    for client in players {
        let Some((mins, maxs)) = super::triggers::player_box(world, client) else {
            continue;
        };
        let sprinting = frame(world).player(ClientId(client)).is_some_and(|ps| {
            ps.pm_flags & playerstate_iw4::pm_flags::SPRINTING != 0
        });
        let mut step_sound = String::new();
        for t in &trigs {
            let Some(e) = world.resource::<Zm>().ents.get(&t.n).cloned() else {
                continue;
            };
            let key = (client, t.n);
            let inside = super::triggers::touches(world, &e, mins, maxs);
            if inside {
                now_inside.insert(key);
                if let Some(label) = &t.label {
                    step_sound.clone_from(label);
                }
            }
            let entered = inside && !was.contains(&key);
            let left = !inside && was.contains(&key);
            if entered && (!t.step || sprinting) {
                plays.push((client, t.enter.clone(), e.origin));
            }
            if left
                && t.step
                && sprinting
                && let Some(leave) = &t.leave
            {
                plays.push((client, leave.clone(), e.origin));
            }
        }
        steps.push((client, step_sound));
    }
    world.resource_mut::<Zm>().audio_trigs.inside = now_inside;
    for (client, step) in steps {
        let was = world.resource::<Zm>().audio_trigs.step_of.get(&client).cloned();
        if was.as_deref().unwrap_or("") != step {
            diag::info!(Sim, "bo2zm t6 step sound: player {client} {:?}", step);
            super::set_client_dvar(world, client, "bo2zm_step", &step);
            world.resource_mut::<Zm>().audio_trigs.step_of.insert(client, step);
        }
    }
    for (client, alias, origin) in plays {
        super::natives_fx::sound(
            world,
            EventAudience::Client(ClientId(client)),
            &alias,
            origin,
        );
    }
}
