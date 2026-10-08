//! Script vehicles on paths: `attachpath(node)` then `startpath()` moves a
//! vehicle entity along its chain of `info_vehicle_node`s (each node's
//! `target` names the next; a chain may loop back on itself), turned along
//! the way; `reached_node` (with the node) at each node it passes,
//! `reached_end_node` at the last one. Nuketown's perk machines ride
//! `perk_arrival_vehicle` down from the sky this way (linked to it).
//!
//! Speed: each node's `speed` (miles per hour, 17.6 units a second each)
//! until the script drives it with `setspeed(mph, accel, decel)` (eased,
//! miles per hour per second) or `setspeedimmediate`; `resumespeed` hands
//! it back to the nodes. Tranzit's bus is driven this way (its stop
//! schedule waits on `reached_node` and polls `getspeed`).

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{Value, Vm};

use super::{Zm, arg, entnum, num, vec3, with_vm};

const MPH: f32 = 17.6;

#[derive(Clone, Debug, Default)]
pub(crate) struct VPath {
    /// The node it left last.
    pub from: u32,
    /// The node it drives to.
    pub to: u32,
}

/// A vehicle path node: where it is, the node its `target` names, its speed
/// (units a second).
#[derive(Clone, Debug)]
pub(crate) struct VNode {
    pub origin: [f32; 3],
    pub next: Option<u32>,
    pub speed: f32,
}

/// A vehicle's speed state (units a second).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VSpeed {
    pub now: f32,
    /// The speed the script asked for (`setspeed`); `None` follows the nodes.
    pub target: Option<f32>,
    pub accel: f32,
    pub decel: f32,
    /// `setvehmaxspeed`.
    pub max: Option<f32>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Vehicles {
    /// The node a vehicle's path starts at (`attachpath`).
    pub attached: BTreeMap<u32, u32>,
    pub moving: BTreeMap<u32, VPath>,
    pub speed: BTreeMap<u32, VSpeed>,
    /// Every `info_vehicle_node` by entity number (built on first use).
    pub nodes: Option<BTreeMap<u32, VNode>>,
}

fn build_nodes(vm: &mut Vm<World>, world: &World) -> BTreeMap<u32, VNode> {
    let zm = world.resource::<Zm>();
    let (tn, tg, sp) = (
        vm.intern("targetname"),
        vm.intern("target"),
        vm.intern("speed"),
    );
    let raw: Vec<(u32, [f32; 3], String, String, f32)> = zm
        .ents
        .iter()
        .filter(|(_, e)| e.classname.starts_with("info_vehicle_node"))
        .filter_map(|(n, e)| {
            let o = e.obj?;
            let name = vm.to_text(&vm.raw_field(o, tn));
            let target = vm.to_text(&vm.raw_field(o, tg));
            let v = vm.raw_field(o, sp);
            let speed = v
                .as_float()
                .or_else(|| vm.to_text(&v).parse().ok())
                .unwrap_or(30.0);
            Some((*n, e.origin, name, target, speed))
        })
        .collect();
    let by_name: BTreeMap<&str, u32> = raw
        .iter()
        .filter(|n| !n.2.is_empty())
        .map(|n| (n.2.as_str(), n.0))
        .collect();
    raw.iter()
        .map(|n| {
            let next = (!n.3.is_empty())
                .then(|| by_name.get(n.3.as_str()).copied())
                .flatten();
            (
                n.0,
                VNode {
                    origin: n.1,
                    next,
                    speed: n.4.max(1.0) * MPH,
                },
            )
        })
        .collect()
}

/// Each tick: vehicles along their paths.
pub(crate) fn advance(world: &mut World, dt: f32) {
    chase_cam(world);
    let ids: Vec<u32> = world
        .resource::<Zm>()
        .vehicles
        .moving
        .keys()
        .copied()
        .collect();
    if ids.is_empty() {
        return;
    }
    let Some(nodes) = world.resource_mut::<Zm>().vehicles.nodes.take() else {
        return;
    };
    // (vehicle, node reached, end of the path)
    let mut reached: Vec<(u32, u32, bool)> = Vec::new();
    for v in ids {
        let Some(mut path) = world.resource::<Zm>().vehicles.moving.get(&v).cloned() else {
            continue;
        };
        let Some(mut pos) = world.resource::<Zm>().ents.get(&v).map(|e| e.origin) else {
            world.resource_mut::<Zm>().vehicles.moving.remove(&v);
            continue;
        };
        let mut sp = world
            .resource::<Zm>()
            .vehicles
            .speed
            .get(&v)
            .copied()
            .unwrap_or_default();
        sp.now = match sp.target {
            Some(t) if sp.now < t => (sp.now + sp.accel * dt).min(t),
            Some(t) => (sp.now - sp.decel * dt).max(t),
            None => nodes.get(&path.from).map_or(0.0, |n| n.speed),
        };
        if let Some(m) = sp.max {
            sp.now = sp.now.min(m);
        }
        let mut step = sp.now * dt;
        let mut yaw = None;
        let mut done = false;
        let mut hops = 0;
        while step > 0.0 && hops < 64 {
            let Some(to) = nodes.get(&path.to) else {
                done = true;
                break;
            };
            let d = [
                to.origin[0] - pos[0],
                to.origin[1] - pos[1],
                to.origin[2] - pos[2],
            ];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if len > 1e-3 && (d[0].abs() + d[1].abs()) > 1e-3 {
                yaw = Some(d[1].atan2(d[0]).to_degrees());
            }
            if len <= step {
                pos = to.origin;
                step -= len;
                hops += 1;
                let end = to.next.is_none();
                reached.push((v, path.to, end));
                if end {
                    done = true;
                    break;
                }
                path.from = path.to;
                path.to = to.next.unwrap_or(path.to);
            } else {
                pos = std::array::from_fn(|i| pos[i] + d[i] / len * step);
                step = 0.0;
            }
        }
        let mut zm = world.resource_mut::<Zm>();
        if let Some(e) = zm.ents.get_mut(&v) {
            e.origin = pos;
            if let Some(y) = yaw {
                e.angles = [0.0, y, 0.0];
            }
        }
        if done {
            sp.now = 0.0;
            zm.vehicles.moving.remove(&v);
        } else {
            zm.vehicles.moving.insert(v, path);
        }
        zm.vehicles.speed.insert(v, sp);
    }
    world.resource_mut::<Zm>().vehicles.nodes = Some(nodes);
    if reached.is_empty() {
        return;
    }
    with_vm(world, |vm, world| {
        for (v, node, end) in reached {
            let zm = world.resource::<Zm>();
            let vo = zm.ents.get(&v).and_then(|e| e.obj);
            let no = zm.ents.get(&node).and_then(|e| e.obj);
            let Some(vo) = vo else { continue };
            if vehicle_log() {
                let at = zm.ents.get(&v).map_or([0.0; 3], |e| e.origin);
                diag::info!(
                    Sim,
                    "bo2zm t6 vehicle {v} reached node {node} at ({:.0} {:.0} {:.0}){}",
                    at[0],
                    at[1],
                    at[2],
                    if end { ", end of path" } else { "" }
                );
            }
            let arg = no.map_or(Value::Undefined, Value::Object);
            vm.notify_str(world, vo, "reached_node", &[arg]);
            if end {
                vm.notify_str(world, vo, "reached_end_node", &[]);
            }
        }
    });
}

fn set_speed(
    vm: &mut Vm<World>,
    world: &mut World,
    s: &Value,
    a: &[Value],
    immediate: bool,
) -> Result<Value, String> {
    let Some(v) = entnum(vm, s) else {
        return Ok(Value::Undefined);
    };
    let mph = num(a, 0).unwrap_or(0.0).max(0.0);
    let accel = num(a, 1).unwrap_or(10.0).max(0.1);
    let decel = num(a, 2).unwrap_or(accel).max(0.1);
    let mut zm = world.resource_mut::<Zm>();
    let sp = zm.vehicles.speed.entry(v).or_default();
    sp.target = Some(mph * MPH);
    sp.accel = accel * MPH;
    sp.decel = decel * MPH;
    if immediate {
        sp.now = mph * MPH;
    }
    Ok(Value::Undefined)
}

/// A vehicle's speed now (units a second); 0 off its path.
fn speed_now(vm: &Vm<World>, world: &World, s: &Value) -> f32 {
    let zm = world.resource::<Zm>();
    entnum(vm, s)
        .filter(|v| zm.vehicles.moving.contains_key(v))
        .and_then(|v| zm.vehicles.speed.get(&v))
        .map_or(0.0, |sp| sp.now)
}

/// IW4L_T6_CHASECAM="back up": from 12 s player 0 is held `back` units
/// behind and `up` above the first script-driven vehicle (Tranzit's bus),
/// looking at it: shots of it on the road.
fn chase_cam(world: &mut World) {
    static CAM: std::sync::OnceLock<Option<(f32, f32)>> = std::sync::OnceLock::new();
    let Some((back, up)) = *CAM.get_or_init(|| {
        let v = std::env::var("IW4L_T6_CHASECAM").ok()?;
        let mut n = v.split_whitespace().filter_map(|x| x.parse().ok());
        Some((n.next()?, n.next()?))
    }) else {
        return;
    };
    let zm = world.resource::<Zm>();
    if zm.now_ms < 12_000 {
        return;
    }
    let Some((o, ang)) = zm.vehicles.speed.keys().find_map(|v| pose(world, *v)) else {
        return;
    };
    let (f, _, _) = gsc_t6::math::angle_vectors([0.0, ang[1], 0.0]);
    let eye = [o[0] - f[0] * back, o[1] - f[1] * back, o[2] + up];
    let d = gsc_t6::math::sub([o[0], o[1], o[2] + 60.0], eye);
    let pitch = -(d[2].atan2((d[0] * d[0] + d[1] * d[1]).sqrt())).to_degrees();
    let yaw = d[1].atan2(d[0]).to_degrees();
    let client = crate::world::ClientId(0);
    super::teleport_player(world, client, [eye[0], eye[1], eye[2] - 60.0]);
    super::set_player_view(world, client, [pitch, yaw, 0.0]);
}

/// IW4L_T6_VEHLOG=1: a log line at each vehicle path node passed.
fn vehicle_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("IW4L_T6_VEHLOG").is_some())
}

/// An entity's origin and angles.
fn pose(world: &World, n: u32) -> Option<([f32; 3], [f32; 3])> {
    world
        .resource::<Zm>()
        .ents
        .get(&n)
        .map(|e| (e.origin, e.angles))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    vm.bind("attachpath", true, |vm, world, s, a| {
        if let (Some(v), Some(node)) = (entnum(vm, s), entnum(vm, arg(a, 0))) {
            world.resource_mut::<Zm>().vehicles.attached.insert(v, node);
        }
        Ok(Value::Undefined)
    });
    vm.bind("startpath", true, |vm, world, s, _| {
        let Some(v) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        let Some(start) = world.resource::<Zm>().vehicles.attached.get(&v).copied() else {
            return Ok(Value::Undefined);
        };
        if world.resource::<Zm>().vehicles.nodes.is_none() {
            let nodes = build_nodes(vm, world);
            world.resource_mut::<Zm>().vehicles.nodes = Some(nodes);
        }
        let mut zm = world.resource_mut::<Zm>();
        let Some(first) = zm
            .vehicles
            .nodes
            .as_ref()
            .and_then(|n| n.get(&start))
            .cloned()
        else {
            return Ok(Value::Undefined);
        };
        if let Some(e) = zm.ents.get_mut(&v) {
            e.origin = first.origin;
        }
        let path = VPath {
            from: start,
            to: first.next.unwrap_or(start),
        };
        zm.vehicles.moving.insert(v, path);
        Ok(Value::Undefined)
    });
    // setspeed(mph, accel, decel) / setspeedimmediate(mph): the script
    // drives the vehicle from here on.
    vm.bind("setspeed", true, |vm, world, s, a| {
        set_speed(vm, world, s, a, false)
    });
    vm.bind("setspeedimmediate", true, |vm, world, s, a| {
        set_speed(vm, world, s, a, true)
    });
    vm.bind("resumespeed", true, |vm, world, s, _| {
        if let Some(v) = entnum(vm, s) {
            let mut zm = world.resource_mut::<Zm>();
            zm.vehicles.speed.entry(v).or_default().target = None;
        }
        Ok(Value::Undefined)
    });
    vm.bind("setvehmaxspeed", true, |vm, world, s, a| {
        if let Some(v) = entnum(vm, s) {
            let mph = num(a, 0).unwrap_or(0.0);
            let mut zm = world.resource_mut::<Zm>();
            zm.vehicles.speed.entry(v).or_default().max = (mph > 0.0).then_some(mph * MPH);
        }
        Ok(Value::Undefined)
    });
    // getspeed: units a second; getspeedmph: miles per hour.
    vm.bind("getspeed", true, |vm, world, s, _| {
        Ok(Value::Float(speed_now(vm, world, s)))
    });
    vm.bind("getspeedmph", true, |vm, world, s, _| {
        Ok(Value::Float(speed_now(vm, world, s) / MPH))
    });
    // Entity space <-> world space (forward, left, up), as `linkto` offsets.
    vm.bind("localtoworldcoords", true, |vm, world, s, a| {
        let local = vec3(a, 0)?;
        let Some((o, ang)) = entnum(vm, s).and_then(|n| pose(world, n)) else {
            return Ok(Value::Vec3(local));
        };
        let (f, r, u) = gsc_t6::math::angle_vectors(ang);
        Ok(Value::Vec3(std::array::from_fn(|i| {
            o[i] + f[i] * local[0] - r[i] * local[1] + u[i] * local[2]
        })))
    });
    vm.bind("worldtolocalcoords", true, |vm, world, s, a| {
        let p = vec3(a, 0)?;
        let Some((o, ang)) = entnum(vm, s).and_then(|n| pose(world, n)) else {
            return Ok(Value::Vec3(p));
        };
        let (f, r, u) = gsc_t6::math::angle_vectors(ang);
        let d = gsc_t6::math::sub(p, o);
        Ok(Value::Vec3([
            gsc_t6::math::dot(d, f),
            -gsc_t6::math::dot(d, r),
            gsc_t6::math::dot(d, u),
        ]))
    });
    // spawnvehicle(model, targetname, vehicletype, origin, angles): a
    // script vehicle (Origins' crystal biplane, Maxis drone); it drives a
    // path once given one (attachpath/startpath).
    vm.bind("spawnvehicle", false, |vm, world, _, a| {
        let text = |vm: &Vm<World>, i: usize| match arg(a, i) {
            Value::Undefined => String::new(),
            v => vm.to_text(v),
        };
        let (model, targetname, vehicletype) = (text(vm, 0), text(vm, 1), text(vm, 2));
        let origin = vec3(a, 3).unwrap_or([0.0; 3]);
        let angles = vec3(a, 4).unwrap_or([0.0; 3]);
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(gsc_t6::ObjKind::Entity(n));
        world.resource_mut::<Zm>().ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                classname: "script_vehicle".to_owned(),
                origin,
                angles,
                model,
                solid: true,
                ..Default::default()
            },
        );
        for (k, v) in [
            ("targetname", targetname),
            ("vehicletype", vehicletype),
            ("classname", "script_vehicle".to_owned()),
        ] {
            let k = vm.intern(k);
            let v = vm.string(&v);
            vm.set_field(world, obj, k, v);
        }
        Ok(Value::Object(obj))
    });
    for name in ["setvehgoalpos", "vehicle_detachfrompath"] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
    }
}
