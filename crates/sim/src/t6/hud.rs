//! Script HUD elements (`newhudelem`, `newclienthudelem`): text and values
//! the zombies scripts put on screen (Game Over, "you survived N rounds",
//! Max Ammo...). The engine keeps each element's text (the game's English
//! with its arguments), fades and moves; every tick each player's visible
//! elements go to his client as one client dvar (`bo2zm_hud`), drawn by
//! the zombies HUD.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, Value, Vm};

use super::{Zm, arg, entnum, frame, with_vm};

/// Separators in the published list: elements, then fields.
pub const ELEM_SEP: char = '\u{1e}';
pub const FIELD_SEP: char = '\u{1f}';

#[derive(Clone, Debug, Default)]
pub(crate) struct HudElem {
    pub obj: Option<ObjRef>,
    /// Shown to one player (`newclienthudelem`), else to all.
    pub client: Option<u32>,
    pub text: String,
    /// A fade in progress: (alpha it started from, start ms, length ms).
    pub fade: Option<(f32, i64, i64)>,
    /// A move in progress: (x, y it started from, start ms, length ms).
    pub mv: Option<([f32; 2], i64, i64)>,
    /// A timer counting down from (seconds, set at ms).
    pub timer: Option<(f32, i64)>,
    /// A picture instead of text (`setshader(name, width, height)`).
    pub shader: Option<(String, f32, f32)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Huds {
    pub elems: BTreeMap<u32, HudElem>,
    /// What each player was last sent.
    pub sent: BTreeMap<u32, String>,
}

fn field(vm: &mut Vm<World>, o: ObjRef, name: &str) -> Value {
    let f = vm.intern(name);
    vm.raw_field(o, f)
}

fn num(vm: &mut Vm<World>, o: ObjRef, name: &str, default: f32) -> f32 {
    field(vm, o, name).as_float().unwrap_or(default)
}

fn txt(vm: &mut Vm<World>, o: ObjRef, name: &str, default: &str) -> String {
    match field(vm, o, name) {
        Value::Undefined => default.to_owned(),
        v => vm.to_text(&v),
    }
}

/// A new element: the engine's defaults, kept for drawing.
fn create(vm: &mut Vm<World>, world: &mut World, client: Option<u32>) -> Value {
    let o = vm.alloc_object(ObjKind::HudElem(0));
    for (k, v) in [
        ("x", Value::Int(0)),
        ("y", Value::Int(0)),
        ("z", Value::Int(0)),
        ("alpha", Value::Float(1.0)),
        ("fontscale", Value::Float(1.0)),
        ("sort", Value::Int(0)),
        ("color", Value::Vec3([1.0, 1.0, 1.0])),
        ("glowalpha", Value::Float(0.0)),
        ("glowcolor", Value::Vec3([0.0, 0.0, 0.0])),
        ("width", Value::Int(0)),
        ("height", Value::Int(0)),
    ] {
        let f = vm.intern(k);
        vm.set_raw_field(o, f, v);
    }
    world.resource_mut::<Zm>().huds.elems.insert(
        o.index,
        HudElem {
            obj: Some(o),
            client,
            ..Default::default()
        },
    );
    Value::Object(o)
}

fn elem_mut<'a>(zm: &'a mut Zm, s: &Value) -> Option<&'a mut HudElem> {
    let Value::Object(o) = s else { return None };
    zm.huds
        .elems
        .get_mut(&o.index)
        .filter(|e| e.obj == Some(*o))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    for name in [
        "newhudelem",
        "newteamhudelem",
        "newdebughudelem",
        "newdamageindicatorhudelem",
    ] {
        vm.bind(name, false, |vm, world, _, _| Ok(create(vm, world, None)));
    }
    for name in ["newclienthudelem", "newscorehudelem"] {
        vm.bind(name, false, |vm, world, _, a| {
            let client = entnum(vm, arg(a, 0));
            Ok(create(vm, world, client))
        });
    }
    vm.bind("settext", true, |vm, world, s, a| {
        let text = super::localize(vm, world, arg(a, 0), a.get(1..).unwrap_or(&[]));
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.text = text;
            e.timer = None;
        }
        Ok(Value::Undefined)
    });
    vm.bind("setvalue", true, |vm, world, s, a| {
        let text = match arg(a, 0) {
            Value::Float(f) if f.fract() != 0.0 => format!("{f}"),
            v => v
                .as_float()
                .map_or_else(|| vm.to_text(v), |f| format!("{}", f as i64)),
        };
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.text = text;
            e.timer = None;
        }
        Ok(Value::Undefined)
    });
    vm.bind("setshader", true, |vm, world, s, a| {
        let name = vm.to_text(arg(a, 0));
        let w = arg(a, 1).as_float().unwrap_or(0.0);
        let h = arg(a, 2).as_float().unwrap_or(0.0);
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.shader = Some((name, w, h));
            e.text.clear();
        }
        Ok(Value::Undefined)
    });
    for name in ["settimer", "settenthstimer"] {
        vm.bind(name, true, |_, world, s, a| {
            let secs = arg(a, 0).as_float().unwrap_or(0.0);
            let now = world.resource::<Zm>().now_ms;
            if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
                e.timer = Some((secs, now));
            }
            Ok(Value::Undefined)
        });
    }
    vm.bind("fadeovertime", true, |vm, world, s, a| {
        let secs = arg(a, 0).as_float().unwrap_or(0.0);
        let now = world.resource::<Zm>().now_ms;
        let Value::Object(o) = s else {
            return Ok(Value::Undefined);
        };
        let from = shown_alpha(vm, world, *o, now);
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.fade = Some((from, now, (secs * 1000.0) as i64));
        }
        Ok(Value::Undefined)
    });
    vm.bind("moveovertime", true, |vm, world, s, a| {
        let secs = arg(a, 0).as_float().unwrap_or(0.0);
        let now = world.resource::<Zm>().now_ms;
        let Value::Object(o) = s else {
            return Ok(Value::Undefined);
        };
        let from = [num(vm, *o, "x", 0.0), num(vm, *o, "y", 0.0)];
        if let Some(e) = elem_mut(&mut world.resource_mut::<Zm>(), s) {
            e.mv = Some((from, now, (secs * 1000.0) as i64));
        }
        Ok(Value::Undefined)
    });
    vm.bind("destroy", true, |vm, world, s, _| {
        if let Some(o) = s.as_obj()
            && matches!(vm.kind(o), Some(ObjKind::HudElem(_)))
        {
            world.resource_mut::<Zm>().huds.elems.remove(&o.index);
            vm.free_object(world, o);
        }
        Ok(Value::Undefined)
    });
}

/// The alpha an element shows now (mid-fade or its field).
fn shown_alpha(vm: &mut Vm<World>, world: &World, o: ObjRef, now: i64) -> f32 {
    let target = num(vm, o, "alpha", 1.0);
    match world
        .resource::<Zm>()
        .huds
        .elems
        .get(&o.index)
        .and_then(|e| e.fade)
    {
        Some((from, start, len)) if len > 0 && now < start + len => {
            let t = (now - start) as f32 / len as f32;
            from + (target - from) * t
        }
        _ => target,
    }
}

/// Each tick: every player's visible elements, sent when they change.
pub(crate) fn publish(world: &mut World) {
    let now = world.resource::<Zm>().now_ms;
    let elems: Vec<(u32, HudElem)> = world
        .resource::<Zm>()
        .huds
        .elems
        .iter()
        .map(|(k, e)| (*k, e.clone()))
        .collect();
    let rows: Vec<(Option<u32>, String)> = with_vm(world, |vm, world| {
        let mut rows = Vec::new();
        for (_, e) in &elems {
            let Some(o) = e.obj else { continue };
            if !vm.alive(o) {
                continue;
            }
            let alpha = shown_alpha(vm, world, o, now);
            let mut text = e.text.clone();
            if let Some((secs, at)) = e.timer {
                let left = (secs - (now - at) as f32 / 1000.0).max(0.0);
                text = format!("{}:{:02}", left as i32 / 60, left as i32 % 60);
            }
            // A picture rides in the text field as `#shader:<name>:<w>:<h>`.
            if let Some((name, w, h)) = &e.shader {
                text = format!("#shader:{name}:{w}:{h}");
            }
            if text.is_empty() || alpha <= 0.01 {
                continue;
            }
            let (mut x, mut y) = (num(vm, o, "x", 0.0), num(vm, o, "y", 0.0));
            if let Some((from, start, len)) = e.mv
                && len > 0
                && now < start + len
            {
                let t = (now - start) as f32 / len as f32;
                x = from[0] + (x - from[0]) * t;
                y = from[1] + (y - from[1]) * t;
            }
            let color = field(vm, o, "color").as_vec3().unwrap_or([1.0; 3]);
            let row = [
                format!("{x:.1}"),
                format!("{y:.1}"),
                txt(vm, o, "alignx", "left"),
                txt(vm, o, "aligny", "top"),
                txt(vm, o, "horzalign", "fullscreen"),
                txt(vm, o, "vertalign", "fullscreen"),
                format!("{:.2}", num(vm, o, "fontscale", 1.0)),
                format!("{:.2}", color[0]),
                format!("{:.2}", color[1]),
                format!("{:.2}", color[2]),
                format!("{alpha:.2}"),
                format!("{}", num(vm, o, "sort", 0.0) as i32),
                text.replace([ELEM_SEP, FIELD_SEP], " "),
            ]
            .join(&FIELD_SEP.to_string());
            rows.push((e.client, row));
        }
        rows
    })
    .unwrap_or_default();
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        let mine: Vec<&str> = rows
            .iter()
            .filter(|(who, _)| who.is_none_or(|w| w == c))
            .map(|(_, r)| r.as_str())
            .collect();
        let value = mine.join(&ELEM_SEP.to_string());
        if world.resource::<Zm>().huds.sent.get(&c) == Some(&value) {
            continue;
        }
        if std::env::var_os("IW4L_T6_HUDLOG").is_some() {
            let now = world.resource::<Zm>().now_ms;
            diag::info!(
                Sim,
                "bo2zm t6 hud at {}ms: {}",
                now,
                value.replace([ELEM_SEP], " | ").replace([FIELD_SEP], ",")
            );
        }
        let mut f = frame(world);
        if f.client_meta(crate::world::ClientId(c)).is_none() {
            continue;
        }
        let dvars = &mut f.client_meta_mut(crate::world::ClientId(c)).client_dvars;
        match dvars.iter_mut().find(|(k, _)| k == "bo2zm_hud") {
            Some(row) => row.1 = value.clone(),
            None => dvars.push(("bo2zm_hud".to_owned(), value.clone())),
        }
        world.resource_mut::<Zm>().huds.sent.insert(c, value);
    }
}
