//! The map's AI navigation graph: Black Ops II's own path nodes and links
//! (GameWorldMp path data, baked by the map compiler), nearest-node lookup
//! and A* over the links. Negotiation links (jumps, climbs, mantles) join a
//! begin node to its end node; the begin node names the traverse animscript.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use super::T6PathNode;

pub(crate) const NODE_PATH: u32 = 1;
pub(crate) const NODE_NEGOTIATION_BEGIN: u32 = 17;
pub(crate) const NODE_NEGOTIATION_END: u32 = 18;
/// Spawnflag of a node placed on a mover, its `target` naming the entity
/// (Die Rise's elevator cars and escape pod): it moves with it.
pub(crate) const NODE_ON_MOVER: u32 = 0x100;

const CELL: f32 = 256.0;

#[derive(Clone, Debug, Default)]
pub(crate) struct Nav {
    pub nodes: Vec<T6PathNode>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    pub by_targetname: HashMap<String, Vec<u32>>,
    /// Links an entity's `disconnectpaths` cut (a closed door, a debris
    /// pile), by entity, until its `connectpaths`; both directions.
    cut_by: HashMap<u32, Vec<(u32, u32)>>,
    cut: HashSet<(u32, u32)>,
    /// Nodes on movers: (node, entity, its place in the entity's own space
    /// once first seen).
    movers: Vec<(u32, u32, Option<[f32; 3]>)>,
}

/// Does the segment a -> b pass through the box?
fn segment_hits_box(a: [f32; 3], b: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for i in 0..3 {
        let d = b[i] - a[i];
        if d.abs() < 1e-6 {
            if a[i] < lo[i] || a[i] > hi[i] {
                return false;
            }
            continue;
        }
        let (mut near, mut far) = ((lo[i] - a[i]) / d, (hi[i] - a[i]) / d);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, PartialEq)]
struct Open {
    f: f32,
    node: u32,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn cell(p: [f32; 3]) -> (i32, i32) {
    ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32)
}

impl Nav {
    pub(crate) fn new(nodes: Vec<T6PathNode>) -> Self {
        dump(&nodes);
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut by_targetname: HashMap<String, Vec<u32>> = HashMap::new();
        for (i, n) in nodes.iter().enumerate() {
            grid.entry(cell(n.origin)).or_default().push(i as u32);
            if !n.targetname.is_empty() {
                by_targetname
                    .entry(n.targetname.clone())
                    .or_default()
                    .push(i as u32);
            }
        }
        Self {
            nodes,
            grid,
            by_targetname,
            ..Default::default()
        }
    }

    /// `disconnectpaths`: every link crossing the box (an entity's brush in
    /// the world) is cut until `connect` for that entity.
    pub(crate) fn disconnect(&mut self, ent: u32, lo: [f32; 3], hi: [f32; 3]) {
        self.connect(ent);
        let centre = std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5);
        let half = dist(lo, hi) * 0.5;
        let mut cut = Vec::new();
        for (a, _) in self.within(centre, half + 512.0) {
            let pa = self.nodes[a as usize].origin;
            for &(b, _, _) in &self.nodes[a as usize].links {
                let b = u32::from(b);
                let Some(nb) = self.nodes.get(b as usize) else {
                    continue;
                };
                // Raised a little off the floor: links run between nodes on
                // the ground, the door stands on it.
                let lift = |p: [f32; 3]| [p[0], p[1], p[2] + 24.0];
                if segment_hits_box(lift(pa), lift(nb.origin), lo, hi) {
                    cut.push((a, b));
                    cut.push((b, a));
                }
            }
        }
        for &l in &cut {
            self.cut.insert(l);
        }
        self.cut_by.insert(ent, cut);
    }

    /// `connectpaths`: the entity's cut links join again.
    pub(crate) fn connect(&mut self, ent: u32) {
        if self.cut_by.remove(&ent).is_some() {
            self.cut = self.cut_by.values().flatten().copied().collect();
        }
    }

    /// Links cut now (both directions).
    pub(crate) fn cut_count(&self) -> usize {
        self.cut.len()
    }

    /// The closest node to `p` (searching rings of cells outward), giving
    /// height a heavier weight so a node on the floor above loses.
    pub(crate) fn nearest(&self, p: [f32; 3]) -> Option<u32> {
        if self.nodes.is_empty() {
            return None;
        }
        let (cx, cy) = cell(p);
        let mut best: Option<(f32, u32)> = None;
        for ring in 0i32..12 {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let Some(list) = self.grid.get(&(cx + dx, cy + dy)) else {
                        continue;
                    };
                    for &i in list {
                        let o = self.nodes[i as usize].origin;
                        let d = [o[0] - p[0], o[1] - p[1], (o[2] - p[2]) * 3.0];
                        let score = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                        if best.is_none_or(|b| score < b.0) {
                            best = Some((score, i));
                        }
                    }
                }
            }
            // A ring further out can't beat a hit closer than its inner edge.
            if let Some((score, _)) = best
                && score < ring as f32 * CELL
            {
                break;
            }
        }
        best.map(|b| b.1)
    }

    /// Nodes within `radius` of `p` (by straight distance), nearest first.
    pub(crate) fn within(&self, p: [f32; 3], radius: f32) -> Vec<(u32, f32)> {
        let (cx, cy) = cell(p);
        let reach = (radius / CELL).ceil() as i32;
        let mut out = Vec::new();
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                let Some(list) = self.grid.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                for &i in list {
                    let d = dist(self.nodes[i as usize].origin, p);
                    if d <= radius {
                        out.push((i, d));
                    }
                }
            }
        }
        out.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        out
    }

    /// A* from `from` to `to` over the links: the node list, both ends
    /// included, or `None` when no path joins them.
    pub(crate) fn path(&self, from: u32, to: u32) -> Option<Vec<u32>> {
        self.path_from(&[(from, 0.0)], to)
    }

    /// A* from any of `seeds` (node, cost to reach it) to `to`.
    pub(crate) fn path_from(&self, seeds: &[(u32, f32)], to: u32) -> Option<Vec<u32>> {
        self.path_from_avoiding(seeds, to, &[])
    }

    /// `path_from` not going through the `avoid` nodes (`to` excepted).
    pub(crate) fn path_from_avoiding(
        &self,
        seeds: &[(u32, f32)],
        to: u32,
        avoid: &[u32],
    ) -> Option<Vec<u32>> {
        let n = self.nodes.len();
        if to as usize >= n || seeds.iter().any(|s| s.0 as usize >= n) {
            return None;
        }
        if let Some(s) = seeds.iter().find(|s| s.0 == to) {
            return Some(vec![s.0]);
        }
        let goal = self.nodes[to as usize].origin;
        let mut g = vec![f32::INFINITY; n];
        let mut parent = vec![u32::MAX; n];
        let mut closed = vec![false; n];
        for &a in avoid {
            if a != to && (a as usize) < n {
                closed[a as usize] = true;
            }
        }
        let mut open = BinaryHeap::new();
        for &(from, cost) in seeds {
            if closed[from as usize] {
                continue;
            }
            if cost < g[from as usize] {
                g[from as usize] = cost;
                open.push(Open {
                    f: cost + dist(self.nodes[from as usize].origin, goal),
                    node: from,
                });
            }
        }
        let mut expanded = 0;
        while let Some(Open { node, .. }) = open.pop() {
            if closed[node as usize] {
                continue;
            }
            if node == to {
                let mut out = vec![to];
                let mut at = to;
                while parent[at as usize] != u32::MAX {
                    at = parent[at as usize];
                    out.push(at);
                }
                out.reverse();
                return Some(out);
            }
            closed[node as usize] = true;
            expanded += 1;
            if expanded > 4096 {
                return None;
            }
            for &(next, d, _neg) in &self.nodes[node as usize].links {
                let next = u32::from(next);
                if next as usize >= n || closed[next as usize] || self.cut.contains(&(node, next)) {
                    continue;
                }
                let cost = g[node as usize] + d.max(1.0);
                if cost < g[next as usize] {
                    g[next as usize] = cost;
                    parent[next as usize] = node;
                    open.push(Open {
                        f: cost + dist(self.nodes[next as usize].origin, goal),
                        node: next,
                    });
                }
            }
        }
        None
    }

    /// `nodesarelinked`: is there a link `a -> b`?
    pub(crate) fn linked(&self, a: u32, b: u32) -> bool {
        self.nodes
            .get(a as usize)
            .is_some_and(|n| n.links.iter().any(|l| u32::from(l.0) == b))
    }

    /// `linknodes`: a one-way link `a -> b` (Die Rise's escape pod doors).
    pub(crate) fn link(&mut self, a: u32, b: u32) {
        if self.linked(a, b) || b as usize >= self.nodes.len() || b > u32::from(u16::MAX) {
            return;
        }
        let d = dist(self.nodes[a as usize].origin, self.nodes[b as usize].origin);
        self.nodes[a as usize].links.push((b as u16, d, false));
    }

    /// `unlinknodes`: drop the link `a -> b`.
    pub(crate) fn unlink(&mut self, a: u32, b: u32) {
        if let Some(n) = self.nodes.get_mut(a as usize) {
            n.links.retain(|l| u32::from(l.0) != b);
        }
    }

    /// `deletepathnode`: no link leads to or from `n` any more.
    pub(crate) fn delete(&mut self, n: u32) {
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if i as u32 == n {
                node.links.clear();
            } else {
                node.links.retain(|l| u32::from(l.0) != n);
            }
        }
    }

    /// The nodes on the mover `targetname` (spawned as entity `ent`) are
    /// carried by it from now on (`follow_movers`).
    pub(crate) fn attach_movers(&mut self, targetname: &str, ent: u32) {
        for (i, n) in self.nodes.iter().enumerate() {
            if n.spawnflags & NODE_ON_MOVER != 0 && n.target.eq_ignore_ascii_case(targetname) {
                self.movers.push((i as u32, ent, None));
            }
        }
    }

    /// Each tick: nodes on movers follow their entity's pose (kept at the
    /// place in its own space they had when first seen), so the AI paths
    /// into a car or the escape pod where it is now. Gives the nodes that
    /// moved, with their new place.
    pub(crate) fn follow_movers(
        &mut self,
        pose: impl Fn(u32) -> Option<([f32; 3], [f32; 3])>,
    ) -> Vec<(u32, [f32; 3])> {
        let mut moved = Vec::new();
        for k in 0..self.movers.len() {
            let (node, ent, local) = self.movers[k];
            let Some((o, a)) = pose(ent) else {
                continue;
            };
            let p = self.nodes[node as usize].origin;
            let (f, r, u) = gsc_t6::math::angle_vectors(a);
            let l = local.unwrap_or_else(|| {
                let d = gsc_t6::math::sub(p, o);
                [
                    gsc_t6::math::dot(d, f),
                    gsc_t6::math::dot(d, r),
                    gsc_t6::math::dot(d, u),
                ]
            });
            self.movers[k].2 = Some(l);
            let to: [f32; 3] =
                std::array::from_fn(|i| o[i] + f[i] * l[0] + r[i] * l[1] + u[i] * l[2]);
            if dist(to, p) < 0.01 {
                continue;
            }
            let (c0, c1) = (cell(p), cell(to));
            if c0 != c1 {
                if let Some(list) = self.grid.get_mut(&c0) {
                    list.retain(|&i| i != node);
                }
                self.grid.entry(c1).or_default().push(node);
            }
            self.nodes[node as usize].origin = to;
            moved.push((node, to));
        }
        moved
    }

    /// Is `a -> b` a negotiation link (a traversal)?
    pub(crate) fn negotiation(&self, a: u32, b: u32) -> bool {
        self.nodes
            .get(a as usize)
            .is_some_and(|n| n.links.iter().any(|l| u32::from(l.0) == b && l.2))
    }
}

/// IW4L_T6_NAVDUMP=<file>: every path node as loaded, one line each
/// (`index type spawnflags x y z targetname target | links`), for reading
/// the map's paths offline.
fn dump(nodes: &[T6PathNode]) {
    let Some(path) = std::env::var_os("IW4L_T6_NAVDUMP") else {
        return;
    };
    let mut out = String::new();
    for (i, n) in nodes.iter().enumerate() {
        let links: Vec<String> = n.links.iter().map(|l| l.0.to_string()).collect();
        out += &format!(
            "{i} {} {:#x} {:.0} {:.0} {:.0} {} {} | {}
",
            n.ty,
            n.spawnflags,
            n.origin[0],
            n.origin[1],
            n.origin[2],
            if n.targetname.is_empty() {
                "-"
            } else {
                &n.targetname
            },
            if n.target.is_empty() { "-" } else { &n.target },
            links.join(" ")
        );
    }
    let _ = std::fs::write(path, out);
}
