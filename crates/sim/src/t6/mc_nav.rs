//! bo2mc (Minecraft Zombies): zombies walk the block world. While the block
//! world stands in for the map (`crate::bo2mc::enabled()` and
//! `crate::voxel::active()`), an actor's goal is planned over blocks
//! (`mc_path::search`) instead of the map's path nodes; following that way
//! it walks, hops up one block, climbs two or three with BO2's jump-up
//! animation, drops off ledges (jump-down animation from two blocks), and at
//! a block in the way it stops, faces it and swipes (`Request::Claw` every
//! tick) until the world side breaks it. BO2's own scripts still pick the
//! goal (the player), swing at him when he is in reach and run the rest.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ObjKind, ObjRef, Value, Vm};

use super::mc_path::{self, Blocks, Kind, Move, Opts, Step};
use super::Zm;

/// `Traverse::end_node` of a block climb or drop: a plain hop (the walk
/// animation goes on) and one played with BO2's traverse animation.
pub(crate) const HOP: u32 = u32::MAX;
pub(crate) const CLIMB_ANIM: u32 = u32::MAX - 1;

/// A plan holds this long before the next goal plans again.
const REPLAN_MS: i64 = 1000;
/// Cells searched per plan and per tick for all zombies.
const PLAN_BUDGET: usize = 2500;
const TICK_BUDGET: usize = 6000;

/// Per-zombie state on the block world.
#[derive(Clone, Debug, Default)]
pub(crate) struct McActor {
    pub steps: Vec<Step>,
    /// The next step.
    pub i: usize,
    /// The cell the plan starts from, and the goal cell it was made for.
    pub start: [i32; 3],
    pub goal: [i32; 3],
    pub planned_ms: i64,
    pub revision: u64,
    pub has_plan: bool,
    pub reached: bool,
    /// Since when no way gets it any closer (R4 d).
    pub no_path_since: Option<i64>,
    /// It stands clawing the next step's blocks.
    pub clawing: bool,
    /// The block it claws and since when (R4 c).
    pub claw: Option<([i32; 3], i64)>,
    /// The swipe it plays at blocks (substate index).
    pub swipe: Option<(String, i32)>,
    /// Where it was when it last moved on, and when (replan when stuck).
    pub moved: Option<([f32; 3], i64)>,
    /// Its best distance to the player and when it got it (R4 b).
    pub best: Option<(f32, i64)>,
}

/// The block world's zombies.
#[derive(Default)]
pub(crate) struct McState {
    pub actors: BTreeMap<u32, McActor>,
    pub budget_tick: i64,
    pub budget_used: usize,
    /// The node `getnegotiationstartnode` hands a zombie climbing blocks.
    pub node: Option<ObjRef>,
    /// Traverse animations the zombies' animstatedef has (by alias).
    pub traverses: Option<Vec<String>>,
}

/// The block world stands in for the map: zombies plan over blocks.
pub(crate) fn active() -> bool {
    crate::bo2mc::enabled() && crate::voxel::active()
}

/// The live block world, read under one lock.
pub(crate) struct Live<'a, 'b> {
    r: &'a crate::voxel::BlockReader<'b>,
    kinds: std::cell::RefCell<Vec<Option<Kind>>>,
}

impl Blocks for Live<'_, '_> {
    fn kind(&self, b: [i32; 3]) -> Kind {
        let id = usize::from(self.r.shape(b[0], b[1], b[2]));
        if id == 0 {
            return Kind::Air;
        }
        let mut kinds = self.kinds.borrow_mut();
        if kinds.len() <= id {
            kinds.resize(id + 1, None);
        }
        *kinds[id].get_or_insert_with(|| mc_path::classify(self.r.boxes(id as u16)))
    }
    fn dig(&self, b: [i32; 3]) -> Option<f32> {
        crate::bo2mc::dig_estimate(b)
    }
}

pub(crate) fn with_live<R>(f: impl FnOnce(&Live<'_, '_>) -> R) -> Option<R> {
    crate::voxel::with_reader(|r| {
        let live = Live {
            r,
            kinds: std::cell::RefCell::new(Vec::new()),
        };
        f(&live)
    })
}

/// The cell whose floor map point `p` (feet) stands on.
fn cell_of(p: [f32; 3]) -> Option<[i32; 3]> {
    crate::bo2mc::map_to_block([p[0], p[1], p[2] + 1.0])
}

/// Map point of a cell's floor centre.
pub(crate) fn point(c: [i32; 3]) -> [f32; 3] {
    crate::bo2mc::block_to_map(c).unwrap_or([0.0; 3])
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Is block `b` still in the way of a step along `axis`?
fn shut(live: &impl Blocks, b: [i32; 3], down: bool) -> bool {
    let k = live.kind(b);
    if down {
        k != Kind::Air
    } else {
        !matches!(k, Kind::Air | Kind::Low)
    }
}

/// Which traverse animations the zombies have (once).
fn traverses(world: &mut World, n: u32) -> Vec<String> {
    if let Some(t) = &world.resource::<Zm>().mc.traverses {
        return t.clone();
    }
    let found: Vec<String> = {
        let zm = world.resource::<Zm>();
        let asd = zm.actors.get(&n).map(|a| a.asd.clone()).unwrap_or_default();
        zm.asds
            .get(&asd)
            .and_then(|d| d.state("zm_traverse", false))
            .map(|st| st.substates.iter().map(|(a, _)| a.clone()).collect())
            .unwrap_or_default()
    };
    if found.is_empty() {
        return found;
    }
    diag::info!(
        Sim,
        "bo2mc zombies: traverse animations {}",
        found.join(" ")
    );
    world.resource_mut::<Zm>().mc.traverses = Some(found.clone());
    found
}

/// The traverse animscript for a climb (`up`) or drop of `k` blocks, when
/// the zombies have it.
fn traverse_for(world: &mut World, n: u32, k: u8, up: bool) -> Option<String> {
    let have = traverses(world, n);
    let wants: &[&str] = match (up, k) {
        (true, 2) => &["jump_up_72"],
        (true, 3) => &["jump_up_96", "jump_up_127"],
        (false, 2) => &["jump_down_72"],
        (false, 3) => &["jump_down_96"],
        (false, 4) => &["jump_down_127", "jump_down_96"],
        _ => &[],
    };
    wants
        .iter()
        .find(|w| have.iter().any(|h| h == *w))
        .map(|w| format!("zm_{w}"))
}

/// Plan actor `n`'s way to map point `goal` over blocks. Always true (no
/// `bad_path`: BO2's breadcrumbs mean nothing here); a zombie with no way
/// stands, and after a while rises again closer (`mc_rules`).
pub(crate) fn plan(world: &mut World, n: u32, goal: [f32; 3]) -> bool {
    let now = world.resource::<Zm>().now_ms;
    let Some(me) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else {
        return false;
    };
    // Not in the middle of a climb.
    if world
        .resource::<Zm>()
        .actors
        .get(&n)
        .is_some_and(|a| a.traverse.is_some())
    {
        return true;
    }
    let (Some(from), Some(to)) = (cell_of(me), cell_of(goal)) else {
        return false;
    };
    let kept = world.resource::<Zm>().mc.actors.get(&n).cloned().unwrap_or_default();
    let goal_moved = |g: [i32; 3]| (g[0] - kept.goal[0]).abs().max((g[2] - kept.goal[2]).abs()) >= 2 || (g[1] - kept.goal[1]).abs() >= 2;
    let settled = with_live(|live| {
        (
            mc_path::settle(live, from, 4),
            mc_path::settle(live, to, 8).unwrap_or(to),
        )
    });
    let Some((start, goal_cell)) = settled else {
        return false;
    };
    let fresh = now - kept.planned_ms < REPLAN_MS;
    let world_same = !crate::voxel::changed_near(kept.revision, me, 24, 8);
    if kept.has_plan && fresh && world_same && !goal_moved(goal_cell) {
        return true;
    }
    {
        let mut zm = world.resource_mut::<Zm>();
        if zm.mc.budget_tick != now {
            zm.mc.budget_tick = now;
            zm.mc.budget_used = 0;
        }
        if zm.mc.budget_used >= TICK_BUDGET && kept.has_plan {
            return true;
        }
    }
    let Some(start) = start else {
        // In the air or stuck in a block: no plan until it stands.
        return true;
    };
    let tall = traverse_for(world, n, 2, true).is_some();
    let budget = {
        let zm = world.resource::<Zm>();
        PLAN_BUDGET.min(TICK_BUDGET.saturating_sub(zm.mc.budget_used)).max(400)
    };
    let revision = crate::voxel::revision();
    let opts = Opts {
        budget,
        tall_climbs: tall,
        ..Opts::default()
    };
    let Some(found) = with_live(|live| mc_path::search(live, start, goal_cell, &opts)) else {
        return false;
    };
    let points: Vec<[f32; 3]> = found.steps.iter().map(|s| point(s.cell)).collect();
    if super::actors::trace_actor(n) {
        diag::info!(
            Sim,
            "bo2mc actor {n} plan {start:?} -> {goal_cell:?}: {} steps, reached {}, {} cells, digs {}",
            found.steps.len(),
            found.reached,
            found.expanded,
            found.steps.iter().map(|s| s.dig.len()).sum::<usize>()
        );
    }
    let mut zm = world.resource_mut::<Zm>();
    zm.mc.budget_used += found.expanded;
    let progress = found.reached || !found.steps.is_empty();
    let st = zm.mc.actors.entry(n).or_default();
    st.steps = found.steps;
    st.i = 0;
    st.start = start;
    st.goal = goal_cell;
    st.planned_ms = now;
    st.revision = revision;
    st.has_plan = true;
    st.reached = found.reached;
    st.clawing = false;
    if progress {
        st.no_path_since = None;
    } else {
        st.no_path_since.get_or_insert(now);
    }
    if let Some(a) = zm.actors.get_mut(&n) {
        a.path = points;
        a.path_nodes.clear();
        a.path_i = 0;
        a.goal_node = None;
    }
    true
}

/// Forget actor `n` (dead, gone).
pub(crate) fn forget(world: &mut World, n: u32) {
    world.resource_mut::<Zm>().mc.actors.remove(&n);
}

/// Does actor `n` stand clawing?
pub(crate) fn clawing(world: &World, n: u32) -> bool {
    world.resource::<Zm>().mc.actors.get(&n).is_some_and(|s| s.clawing)
}

/// The cell a step starts from.
fn from_cell(st: &McActor, i: usize) -> [i32; 3] {
    if i == 0 {
        st.start
    } else {
        st.steps[i - 1].cell
    }
}

/// Turn toward `want` (degrees) at the zombies' turn rate.
fn turn_to(world: &mut World, n: u32, want: f32) {
    let mut zm = world.resource_mut::<Zm>();
    if let Some(e) = zm.ents.get_mut(&n) {
        let delta = gsc_t6::math::angle_clamp180(want - e.angles[1]);
        let step = super::actors::TURN_DEG_PER_S * 0.05;
        e.angles[1] += delta.clamp(-step, step);
    }
    if let Some(a) = zm.actors.get_mut(&n) {
        a.yaw_goal = Some(want);
    }
}

fn yaw_to(from: [f32; 3], to: [f32; 3]) -> Option<f32> {
    let d = [to[0] - from[0], to[1] - from[1]];
    (d[0] * d[0] + d[1] * d[1] > 1e-4).then(|| d[1].atan2(d[0]).to_degrees())
}

/// One tick of walking the way (the actor's `move` script runs).
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow(
    vm: &mut Vm<World>,
    world: &mut World,
    n: u32,
    obj: ObjRef,
    a: &super::actors::Actor,
    me: [f32; 3],
    dt: f32,
    now: i64,
) {
    let Some(mut st) = world.resource::<Zm>().mc.actors.get(&n).cloned() else {
        return;
    };
    if st.i >= st.steps.len() {
        if let Some(act) = world.resource_mut::<Zm>().actors.get_mut(&n) {
            act.path.clear();
        }
        return;
    }
    // Stuck in place while walking: plan again from here.
    match st.moved {
        Some((at, since)) if dist2(at, me) < 8.0 => {
            if now - since > 1500 {
                st.planned_ms = i64::MIN / 2;
                st.moved = Some((me, now));
            }
        }
        _ => st.moved = Some((me, now)),
    }
    // Blocks to claw away first: stand on the cell before and claw.
    let i = st.i;
    let step = st.steps[i].clone();
    let down = step.mv == Move::DigDown;
    let to_claw = with_live(|live| step.dig.iter().copied().find(|b| shut(live, *b, down))).flatten();
    let stand = point(from_cell(&st, i));
    if to_claw.is_some() {
        if dist2(me, stand) > 10.0 {
            walk_to(world, n, a, me, stand, dt, now);
        } else {
            st.clawing = true;
        }
        world.resource_mut::<Zm>().mc.actors.insert(n, st);
        return;
    }
    let target = point(step.cell);
    match step.mv {
        Move::Climb(k) | Move::Drop(k) if k >= 2 || matches!(step.mv, Move::Climb(_)) => {
            if dist2(me, stand) > 14.0 {
                walk_to(world, n, a, me, stand, dt, now);
            } else {
                let up = matches!(step.mv, Move::Climb(_));
                let script = traverse_for(world, n, k, up);
                start_hop(vm, world, n, obj, me, target, script, k, now);
            }
        }
        _ => {
            // Look ahead along plain walks it can make straight.
            let mut j = i;
            let plain = |s: &Step| s.mv == Move::Walk && s.dig.is_empty();
            while j + 1 < st.steps.len()
                && j < i + 3
                && plain(&st.steps[j])
                && plain(&st.steps[j + 1])
                && st.steps[j + 1].cell[1] == st.steps[i].cell[1]
                && super::actors::can_walk(world, me, point(st.steps[j + 1].cell), a.radius, a.height)
            {
                j += 1;
            }
            st.i = j;
            let target = point(st.steps[j].cell);
            let pos = walk_to(world, n, a, me, target, dt, now);
            let falling = matches!(st.steps[j].mv, Move::Drop(_) | Move::DigDown);
            let arrived = dist2(pos, target) < 12.0
                && if falling {
                    pos[2] <= target[2] + 8.0
                } else {
                    (pos[2] - target[2]).abs() < 24.0
                };
            if arrived {
                st.i = j + 1;
            }
        }
    }
    let left: Vec<[f32; 3]> = st.steps[st.i.min(st.steps.len())..]
        .iter()
        .map(|s| point(s.cell))
        .collect();
    let mut zm = world.resource_mut::<Zm>();
    if let Some(act) = zm.actors.get_mut(&n) {
        act.path = left;
        act.path_i = 0;
    }
    zm.mc.actors.insert(n, st);
}

/// Walk toward `target` this tick at the move animation's speed; the new
/// place.
fn walk_to(
    world: &mut World,
    n: u32,
    a: &super::actors::Actor,
    me: [f32; 3],
    target: [f32; 3],
    dt: f32,
    _now: i64,
) -> [f32; 3] {
    let speed = a
        .playing
        .as_ref()
        .and_then(|p| world.resource::<Zm>().anims.get(&p.anim).map(super::actors::anim_speed))
        .filter(|s| *s > 1.0)
        .unwrap_or(60.0);
    let d = [target[0] - me[0], target[1] - me[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let step = (speed * dt).min(len);
    let mv = if len > 1e-3 {
        [d[0] / len * step, d[1] / len * step]
    } else {
        [0.0, 0.0]
    };
    let (pos, clear) = super::actors::walk_move(world, me, mv, a.radius, a.height, a.clear);
    {
        let mut zm = world.resource_mut::<Zm>();
        if let Some(e) = zm.ents.get_mut(&n) {
            e.origin = pos;
        }
        if clear
            && !a.clear
            && let Some(act) = zm.actors.get_mut(&n)
        {
            act.clear = true;
        }
    }
    if let Some(yaw) = yaw_to(me, target) {
        turn_to(world, n, yaw);
    }
    pos
}

/// Start a climb or drop from `from` to `to`: BO2's traverse animation when
/// it has one (`script`), else a hop while it walks.
#[allow(clippy::too_many_arguments)]
fn start_hop(
    vm: &mut Vm<World>,
    world: &mut World,
    n: u32,
    obj: ObjRef,
    from: [f32; 3],
    to: [f32; 3],
    script: Option<String>,
    k: u8,
    now: i64,
) {
    if let Some(yaw) = yaw_to(from, to) {
        let mut zm = world.resource_mut::<Zm>();
        if let Some(e) = zm.ents.get_mut(&n) {
            e.angles[1] = yaw;
        }
    }
    match script {
        Some(script) => {
            super::actors::start_traverse(vm, world, n, obj, CLIMB_ANIM, CLIMB_ANIM, from, to, &script, now);
        }
        None => {
            if let Some(act) = world.resource_mut::<Zm>().actors.get_mut(&n) {
                act.traverse = Some(super::actors::Traverse {
                    from,
                    to,
                    start_ms: now,
                    length_ms: 300 + 120 * i64::from(k),
                    end_node: HOP,
                });
            }
        }
    }
}

/// Where a block climb or drop puts the body at fraction `f`: up first
/// then across when climbing, across first then down when dropping.
pub(crate) fn hop_point(from: [f32; 3], to: [f32; 3], f: f32) -> [f32; 3] {
    let z = if to[2] >= from[2] {
        from[2] + (to[2] - from[2]) * (f / 0.5).min(1.0)
    } else {
        from[2] + (to[2] - from[2]) * ((f - 0.35) / 0.65).clamp(0.0, 1.0)
    };
    let across = if to[2] >= from[2] {
        ((f - 0.2) / 0.8).clamp(0.0, 1.0)
    } else {
        (f / 0.6).min(1.0)
    };
    [
        from[0] + (to[0] - from[0]) * across,
        from[1] + (to[1] - from[1]) * across,
        z,
    ]
}

/// A climb or drop ended: on to the next step.
pub(crate) fn hop_done(world: &mut World, n: u32) {
    if let Some(st) = world.resource_mut::<Zm>().mc.actors.get_mut(&n) {
        st.i += 1;
        st.moved = None;
    }
}

/// The node a zombie climbing blocks reports as its traverse start (the
/// traverse script faces its angles).
pub(crate) fn climb_node(vm: &mut Vm<World>, world: &mut World, n: u32) -> Option<ObjRef> {
    let (from, to) = {
        let zm = world.resource::<Zm>();
        let tr = zm.actors.get(&n)?.traverse.as_ref()?;
        (tr.end_node == CLIMB_ANIM).then_some((tr.from, tr.to))?
    };
    let node = match world.resource::<Zm>().mc.node {
        Some(o) if vm.alive(o) => o,
        _ => {
            let o = vm.alloc_object(ObjKind::Struct);
            world.resource_mut::<Zm>().mc.node = Some(o);
            o
        }
    };
    let yaw = yaw_to(from, to).unwrap_or(0.0);
    for (k, v) in [
        ("origin", Value::Vec3(from)),
        ("angles", Value::Vec3([0.0, yaw, 0.0])),
    ] {
        let f = vm.intern(k);
        vm.set_raw_field(node, f, v);
    }
    Some(node)
}

/// One tick of clawing (the actor's `claw` state): face the block, swipe,
/// tell the world side; done when the step's blocks are gone.
pub(crate) fn claw(vm: &mut Vm<World>, world: &mut World, n: u32, me: [f32; 3], dt: f32, now: i64) {
    let Some(mut st) = world.resource::<Zm>().mc.actors.get(&n).cloned() else {
        return;
    };
    let Some(step) = st.steps.get(st.i).cloned() else {
        st.clawing = false;
        world.resource_mut::<Zm>().mc.actors.insert(n, st);
        return;
    };
    let down = step.mv == Move::DigDown;
    let block = with_live(|live| step.dig.iter().copied().find(|b| shut(live, *b, down))).flatten();
    let Some(block) = block else {
        st.clawing = false;
        st.claw = None;
        st.moved = None;
        world.resource_mut::<Zm>().mc.actors.insert(n, st);
        return;
    };
    // Settle onto what it stands on (a dug floor drops it).
    let (r, h, clear) = world
        .resource::<Zm>()
        .actors
        .get(&n)
        .map_or((12.0, 72.0, true), |a| (a.radius, a.height, a.clear));
    let (pos, _) = super::actors::walk_move(world, me, [0.0, 0.0], r, h, clear);
    if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
        e.origin = pos;
    }
    let centre = point(block);
    if let Some(yaw) = yaw_to(pos, centre) {
        turn_to(world, n, yaw);
    }
    crate::bo2mc::push(crate::bo2mc::Request::Claw {
        block,
        seconds: dt,
    });
    if st.claw.is_none_or(|(b, _)| b != block) {
        st.claw = Some((block, now));
    }
    // The swipe: BO2's melee animation, again each time it ends.
    let ended = world
        .resource::<Zm>()
        .actors
        .get(&n)
        .and_then(|a| a.playing.as_ref().map(|p| p.ended || !p.notify.contains("melee")))
        .unwrap_or(true);
    if ended {
        let mut started = None;
        let picked = st.swipe.clone();
        let tries: Vec<String> = match &picked {
            Some((s, _)) => vec![s.clone()],
            None => ["zm_walk_melee", "zm_run_melee", "zm_window_melee"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        };
        for state in tries {
            let sub = picked.as_ref().map_or(Value::Undefined, |(_, i)| Value::Int(*i));
            if let Some(i) = super::actors::set_state(vm, world, n, &state, &sub) {
                started = Some((state, i as i32));
                break;
            }
        }
        if picked.is_none() {
            st.swipe = started;
        }
    }
    world.resource_mut::<Zm>().mc.actors.insert(n, st);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hops_go_up_then_across_and_across_then_down() {
        let from = [0.0, 0.0, 0.0];
        let up = [36.0, 0.0, 36.0];
        let mid = hop_point(from, up, 0.5);
        assert!((mid[2] - 36.0).abs() < 1e-3);
        assert!(mid[0] < 36.0);
        assert_eq!(hop_point(from, up, 1.0), up);
        let down = [36.0, 0.0, -72.0];
        let mid = hop_point(from, down, 0.3);
        assert!(mid[2] > -1.0 && mid[0] > 10.0);
        assert_eq!(hop_point(from, down, 1.0), down);
    }

    /// The live block world (voxel::activate + set_chunk): a flat stone
    /// floor with a two-high wall that must be dug and the room's walls
    /// protected through the bridge.
    #[test]
    fn plans_over_the_live_block_world() {
        let _guard = crate::voxel::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let shapes = vec![Vec::new(), vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]];
        crate::voxel::activate(&[], [0.5, 64.0, 0.5], shapes);
        for cx in -3..=3 {
            for cz in -3..=3 {
                let mut ids = vec![0u16; 16 * 16 * 80];
                for y in 0..80 {
                    for z in 0..16 {
                        for x in 0..16 {
                            let (bx, bz) = (cx * 16 + x, cz * 16 + z);
                            let solid = y < 64 || (bx == 5 && (64..66).contains(&y) && (-40..40).contains(&bz));
                            if solid {
                                ids[((y * 16 + z) * 16 + x) as usize] = 1;
                            }
                        }
                    }
                }
                crate::voxel::set_chunk(
                    cx,
                    cz,
                    crate::voxel::VoxelChunk {
                        min_y: 0,
                        height: 80,
                        shapes: ids,
                    },
                );
            }
        }
        let protect: std::collections::HashSet<[i32; 3]> =
            (-40..40).map(|z| [5, 65, z]).collect();
        crate::bo2mc::set_room_built(protect);
        let found = with_live(|live| {
            assert_eq!(live.kind([0, 63, 0]), Kind::Solid);
            assert_eq!(live.kind([0, 64, 0]), Kind::Air);
            assert!(mc_path::stands(live, [0, 64, 0]));
            mc_path::search(live, [0, 64, 0], [10, 64, 0], &Opts { tall_climbs: false, ..Opts::default() })
        })
        .unwrap();
        crate::bo2mc::reset();
        crate::voxel::deactivate();
        assert!(found.reached, "{found:?}");
        let dug: Vec<[i32; 3]> = found.steps.iter().flat_map(|s| s.dig.clone()).collect();
        // The wall's protected top row is never dug (it goes round or under).
        assert!(dug.iter().all(|b| !(b[0] == 5 && b[1] == 65)), "{dug:?}");
    }
}
