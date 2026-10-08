//! bo2mc (Minecraft Zombies): the souls round's hellhounds.
//!
//! His ask (10-07): a "fetch me their souls" round every five rounds,
//! Nacht der Untoten's dog rounds. The dogs are Nuketown's own
//! (`aitype/zm_nuked_dog`: the wolf model with the hellhound eyes, the
//! `zm_dog_*` animscripts), spawned straight from here: BO2's dog rounds
//! stay off (`allowdogs` 0) and `mc_rules`' souls round calls `spawn`. Each
//! dog gets BO2's own setup (`_zm_ai_dogs::dog_init`) and entrance
//! (`dog_spawn_fx`: the lightning bolt, the earthquake, the spawn sounds)
//! and hunts player 0. A Minecraft lightning bolt strikes each spot as
//! BO2's bolt sound plays, 1.5 seconds in (his ask: Minecraft lightning).

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, Value};

use super::{Ent, Zm, actors, with_vm};
use crate::bo2mc;

/// The spawner classname `actors::spawn_actor` turns into the aitype.
const SPAWNER: &str = "actor_zm_nuked_dog";
const DOGS: &str = "maps/mp/zombies/_zm_ai_dogs";
/// A hellhound is lower and narrower than a zombie.
const RADIUS: f32 = 10.0;
const HEIGHT: f32 = 36.0;
/// `dog_spawn_fx` waits this long before the bolt sound and the dog.
const BOLT_DELAY: f32 = 1.5;

/// Spawn `count` hellhounds at spread-out spawn spots around the player.
/// Returns their actor numbers.
pub(super) fn spawn(world: &mut World, round: i32, count: usize) -> Vec<u32> {
    let points = bo2mc::spawn_candidates();
    let player = world.resource::<Zm>().players.get(&0).map(|p| p.obj);
    if points.is_empty() || count == 0 {
        diag::info!(Sim, "bo2mc hellhounds: no spawn spots ({} points)", points.len());
        return Vec::new();
    }
    let health = (200 + round * 40).min(1600);
    let dogs = with_vm(world, |vm, world| {
        let level = vm.level;
        let f = vm.intern("dog_health");
        vm.set_raw_field(level, f, Value::Int(health));
        // A spawner entity for `spawn_actor`, moved to each spot in turn.
        let sn = world.resource_mut::<Zm>().alloc_entnum();
        let spawner = vm.alloc_object(ObjKind::Entity(sn));
        world.resource_mut::<Zm>().ents.insert(
            sn,
            Ent { obj: Some(spawner), classname: SPAWNER.into(), ..Default::default() },
        );
        let step = (points.len() / count).max(1);
        let mut dogs = Vec::new();
        for p in points.iter().step_by(step).take(count) {
            if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&sn) {
                e.origin = *p;
            }
            let Some(dog) = actors::spawn_actor(vm, world, spawner) else {
                diag::info!(Sim, "bo2mc hellhounds: {SPAWNER} would not spawn");
                break;
            };
            let Some(ObjKind::Entity(n)) = vm.kind(dog) else { continue };
            if let Some(a) = world.resource_mut::<Zm>().actors.get_mut(&n) {
                a.radius = RADIUS;
                a.height = HEIGHT;
                a.health = health;
                a.maxhealth = health;
            }
            for (k, v) in [("health", Value::Int(health)), ("maxhealth", Value::Int(health))] {
                let f = vm.intern(k);
                vm.set_raw_field(dog, f, v);
            }
            if let Some(pl) = player {
                let f = vm.intern("favoriteenemy");
                vm.set_raw_field(dog, f, Value::Object(pl));
            }
            vm.spawn_named(world, DOGS, "dog_init", Value::Object(dog), vec![]);
            // The entrance takes the spot to strike: the dog's own origin.
            let loc = vm.alloc_object(ObjKind::Struct);
            for (k, v) in [("origin", Value::Vec3(*p)), ("angles", Value::Vec3([0.0; 3]))] {
                let f = vm.intern(k);
                vm.set_raw_field(loc, f, v);
            }
            vm.spawn_named(
                world,
                DOGS,
                "dog_spawn_fx",
                Value::Object(level),
                vec![Value::Object(dog), Value::Object(loc)],
            );
            super::mc_rules::push(bo2mc::Request::Lightning { at: *p, delay: BOLT_DELAY });
            dogs.push(n);
        }
        world.resource_mut::<Zm>().ents.remove(&sn);
        vm.free_object(world, spawner);
        dogs
    })
    .unwrap_or_default();
    diag::info!(Sim, "bo2mc hellhounds: {} of {count} spawned, health {health}", dogs.len());
    dogs
}
