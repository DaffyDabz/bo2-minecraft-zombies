//! Wall buys as the player sees them, which BO2 draws from its client
//! scripts (clientscripts/mp/zombies/_zm_weapons.csc), done here on the
//! server: each wall buy's chalk effect (`level._effect[<weapon>_fx]`, the
//! M14's by default) plays at its struct once the player is in; when the
//! scripts flip its world clientfield (`<weapon>_<origin>` = 1, bought)
//! the gun's model (the target struct's) slides out of the wall to its
//! place over a second.

use std::collections::HashMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, Value, Vm};

use super::{Ent, Zm, with_vm};

#[derive(Clone, Debug, Default)]
pub(crate) struct WallBuys {
    pub chalk_done: bool,
    /// World clientfields the scripts set (name -> value).
    pub world_fields: HashMap<String, i32>,
    /// Wall buys whose gun is out (by clientfield name).
    pub shown: HashMap<String, u32>,
}

fn field(vm: &mut Vm<World>, o: ObjRef, name: &str) -> Value {
    let f = vm.intern(name);
    vm.raw_field(o, f)
}

/// The struct kinds the server's `init_spawnable_weapon_upgrade` gathers as
/// wall buys: guns and grenades, the Bowie knife, the sickle, Galvaknuckles,
/// built wall buys and Claymores (Nuketown has all but the sickle and the
/// built ones).
fn is_wallbuy(targetname: &str) -> bool {
    matches!(
        targetname,
        "weapon_upgrade"
            | "bowie_upgrade"
            | "sickle_upgrade"
            | "tazer_upgrade"
            | "buildable_wallbuy"
            | "claymore_purchase"
    )
}

/// Every map struct (level.struct).
fn structs(vm: &mut Vm<World>) -> Vec<ObjRef> {
    let all = field(vm, vm.level, "struct");
    let Value::Array(arr) = all else {
        return Vec::new();
    };
    arr.snapshot()
        .values_in_order()
        .filter_map(|v| match v {
            Value::Object(o) => Some(*o),
            _ => None,
        })
        .collect()
}

/// Each tick: the chalk once a player has begun (and a moment after, so
/// his client is listening).
pub(crate) fn advance(world: &mut World, now: i64) {
    if world.resource::<Zm>().wallbuys.chalk_done {
        return;
    }
    let begun = world.resource::<Zm>().players.values().any(|p| p.begun);
    if !begun {
        return;
    }
    let since = {
        let mut zm = world.resource_mut::<Zm>();
        let t = *zm.wallbuy_since.get_or_insert(now);
        now - t
    };
    if since < 1500 {
        return;
    }
    world.resource_mut::<Zm>().wallbuys.chalk_done = true;
    // (effect id or its file, where, facing, up, what it sells)
    type Chalk = (Result<i32, String>, [f32; 3], [f32; 3], [f32; 3], String);
    // Wall buys with no chalk drawing hang their gun on the wall instead
    // (by clientfield name).
    let mut on_wall = Vec::new();
    let effects: Vec<Chalk> = with_vm(world, |vm, _| {
        let fx_table = field(vm, vm.level, "_effect");
        let fx_of = |vm: &mut Vm<World>, key: &str| -> Option<i32> {
            let Value::Array(a) = &fx_table else {
                return None;
            };
            let k = gsc_t6::Key::Str(vm.intern(key));
            a.get(&k).and_then(|v| v.as_int())
        };
        let mut out = Vec::new();
        for s in structs(vm) {
            let tn = field(vm, s, "targetname");
            if !is_wallbuy(&vm.to_text(&tn)) {
                continue;
            }
            let w = field(vm, s, "zombie_weapon_upgrade");
            let w = vm.to_text(&w);
            let Some(origin) = field(vm, s, "origin").as_vec3() else {
                continue;
            };
            let angles = field(vm, s, "angles").as_vec3().unwrap_or([0.0; 3]);
            if NO_CHALK.contains(&w.as_str()) {
                let o = field(vm, s, "origin");
                on_wall.push(format!("{w}_{}", vm.to_text(&o)));
                continue;
            }
            // The map's own chalk for it; else its chalk file by name
            // (Nuketown's scripts give the Claymore none); else the M14's.
            let fx = match fx_of(vm, &format!("{w}_fx")) {
                Some(fx) => Ok(fx),
                // (BO2 has no frag chalk: the Semtex grenade's.)
                None if w == "frag_grenade_zm" => Err("maps/zombie/fx_zmb_wall_buy_semtex".to_owned()),
                None if !w.is_empty() => Err(format!("maps/zombie/fx_zmb_wall_buy_{}", w.trim_end_matches("_zm"))),
                None => match fx_of(vm, "m14_zm_fx") {
                    Some(fx) => Ok(fx),
                    None => continue,
                },
            };
            let (f, _, u) = gsc_t6::math::angle_vectors(angles);
            out.push((fx, origin, f, u, w));
        }
        out
    })
    .unwrap_or_default();
    for (fx, origin, forward, up, what) in effects {
        let name = match fx {
            Ok(fx) => super::natives_fx::fx_name(world, fx),
            Err(file) => Some(file),
        };
        if let Some(name) = name {
            diag::info!(
                Sim,
                "wall buy chalk: {what} at {:.0},{:.0},{:.0}: {name}",
                origin[0],
                origin[1],
                origin[2]
            );
            super::natives_fx::effect_axis(world, &name, origin, forward, up);
        }
    }
    for name in on_wall {
        diag::info!(Sim, "wall buy on the wall: {name}");
        with_vm(world, |vm, world| show_gun(vm, world, &name, false));
    }
}

/// Weapons BO2 has no chalk drawing for (Mob of the Dead's, never wall
/// buys there): his 10-07 "just put it on the wall" until he has outlines.
const NO_CHALK: [&str; 3] = ["blundergat_zm", "spoon_zm_alcatraz", "spork_zm_alcatraz"];

/// A world clientfield changed: a wall buy bought shows its gun.
pub(super) fn world_field(vm: &mut Vm<World>, world: &mut World, name: &str, value: i32) {
    let old = world
        .resource_mut::<Zm>()
        .wallbuys
        .world_fields
        .insert(name.to_owned(), value);
    if old == Some(value) || value != 1 || world.resource::<Zm>().wallbuys.shown.contains_key(name)
    {
        return;
    }
    show_gun(vm, world, name, true);
}

/// A wall buy's gun out on the wall (clientfield `name`), sliding out of
/// it or (no chalk) already there.
fn show_gun(vm: &mut Vm<World>, world: &mut World, name: &str, slide: bool) {
    // The wall buy this field belongs to: `<weapon>_<origin>`.
    let mut found = None;
    for s in structs(vm) {
        let tn = field(vm, s, "targetname");
        if !is_wallbuy(&vm.to_text(&tn)) {
            continue;
        }
        let w = field(vm, s, "zombie_weapon_upgrade");
        let o = field(vm, s, "origin");
        if format!("{}_{}", vm.to_text(&w), vm.to_text(&o)) == name {
            let target = field(vm, s, "target");
            found = Some((vm.to_text(&target), vm.to_text(&w)));
            break;
        }
    }
    let Some((target, weapon)) = found else { return };
    let mut model_at = None;
    for s in structs(vm) {
        let tn = field(vm, s, "targetname");
        if vm.to_text(&tn) != target {
            continue;
        }
        let m = field(vm, s, "model");
        let (Some(origin), angles) = (
            field(vm, s, "origin").as_vec3(),
            field(vm, s, "angles").as_vec3().unwrap_or([0.0; 3]),
        ) else {
            continue;
        };
        model_at = Some((vm.to_text(&m), origin, angles));
        break;
    }
    let Some((mut model, origin, angles)) = model_at else {
        return;
    };
    // bo2mc's copied wall buys (frag grenades, the vault's guns) keep the
    // copied gun's model for their use box: the one that comes out is
    // the weapon's own.
    if target.contains("_bo2mc_")
        && let Ok(w) = super::weapon(world, &weapon)
        && let Some((m, _)) = super::frame(world).weapon_world_model(w)
    {
        model = m.to_owned();
    }
    // Out of the wall: from 8 units along its right to its place.
    let (_, right, _) = gsc_t6::math::angle_vectors(angles);
    let from = if slide {
        std::array::from_fn(|i| origin[i] + right[i] * 8.0)
    } else {
        origin
    };
    let now = world.resource::<Zm>().now_ms;
    let n = world.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(ObjKind::Entity(n));
    let mut zm = world.resource_mut::<Zm>();
    zm.ents.insert(
        n,
        Ent {
            obj: Some(obj),
            classname: "script_model".into(),
            origin: from,
            angles,
            model,
            ..Default::default()
        },
    );
    zm.movers.move_to(n, from, origin, now, 1.0, 0.0, 0.0);
    zm.wallbuys.shown.insert(name.to_owned(), n);
}
