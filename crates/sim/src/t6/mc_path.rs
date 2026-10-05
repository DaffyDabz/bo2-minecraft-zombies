//! bo2mc (Minecraft Zombies): the zombies' way over blocks. An A* search
//! over the block grid: a zombie stands in an air cell over a solid block
//! with one more air cell above (it is two blocks tall), walks to its eight
//! neighbours (no corner cutting), steps up one block, climbs two or three
//! with a climb animation, drops up to four, and claws through blocks that
//! may be broken (everything but the spawn room's walls, floor and roof, and
//! what the world side says is unbreakable) at a cost.
//!
//! This part is pure: blocks come through [`Blocks`], so the tests run on a
//! made-up world. `crate::t6::mc_nav` runs it on the live block world and
//! walks the zombies along what it finds.

use std::collections::{BinaryHeap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

/// What a block is to a zombie walking through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Nothing to bump into (air, grass, flowers, water).
    Air,
    /// Low enough to walk over (carpet, a bottom slab, a snow layer).
    Low,
    /// In the way.
    Solid,
    /// A thin panel on one edge of the block, lying along x (a door): a
    /// walk along x passes it, one along z does not.
    PanelX,
    /// The same lying along z.
    PanelZ,
}

/// A block's kind from its collision boxes (block space, `[min, max]`).
pub(crate) fn classify(boxes: &[[f32; 6]]) -> Kind {
    if boxes.is_empty() {
        return Kind::Air;
    }
    if boxes.iter().all(|b| b[4] <= 0.5625) {
        return Kind::Low;
    }
    let edge = |lo: usize, hi: usize| {
        boxes.iter().all(|b| b[hi] - b[lo] <= 0.25)
            && (boxes.iter().all(|b| b[hi] <= 0.25) || boxes.iter().all(|b| b[lo] >= 0.75))
    };
    if edge(2, 5) {
        return Kind::PanelX;
    }
    if edge(0, 3) {
        return Kind::PanelZ;
    }
    Kind::Solid
}

/// The block world as the search sees it.
pub(crate) trait Blocks {
    fn kind(&self, b: [i32; 3]) -> Kind;
    /// Seconds to claw block `b` away when planning; None = never.
    fn dig(&self, b: [i32; 3]) -> Option<f32>;
}

/// How a step is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Move {
    /// Walk across (straight or diagonal), clawing what is in the way first.
    Walk,
    /// Up this many blocks onto the next cell.
    Climb(u8),
    /// Off the edge, landing this many blocks lower.
    Drop(u8),
    /// Claw the block it stands on and fall into its place.
    DigDown,
}

/// One step of a way: the cell the zombie's feet end in, how it gets there,
/// and the blocks it claws away first (in order).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
    pub cell: [i32; 3],
    pub mv: Move,
    pub dig: Vec<[i32; 3]>,
}

/// What a search found.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    /// The steps from the start (not included) on.
    pub steps: Vec<Step>,
    /// It ends next to the goal (else it ends where it got closest).
    pub reached: bool,
    /// Cells it looked at.
    pub expanded: usize,
}

/// Search options.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Opts {
    /// Cells the search may look at.
    pub budget: usize,
    /// Two- and three-block climbs (a climb animation is loaded).
    pub tall_climbs: bool,
    /// Blocks walked per second of clawing (how much longer a walk round
    /// may be before digging through is the better way).
    pub dig_weight: f32,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            budget: 2500,
            tall_climbs: true,
            dig_weight: 2.5,
        }
    }
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn up(a: [i32; 3], k: i32) -> [i32; 3] {
    [a[0], a[1] + k, a[2]]
}

/// Can the body pass through a block of kind `k` moving along `axis`
/// (0 = x, 2 = z, None = diagonally or up and down)?
fn open(k: Kind, axis: Option<usize>) -> bool {
    match k {
        Kind::Air | Kind::Low => true,
        Kind::Solid => false,
        Kind::PanelX => axis == Some(0),
        Kind::PanelZ => axis == Some(2),
    }
}

/// Can a zombie stand with its feet in cell `c`?
pub(crate) fn stands(w: &impl Blocks, c: [i32; 3]) -> bool {
    let k = w.kind(c);
    (k == Kind::Low || (k == Kind::Air && w.kind(up(c, -1)) == Kind::Solid))
        && open(w.kind(up(c, 1)), None)
}

/// Where a body whose feet are in cell `c` really stands: the cell itself,
/// one up when it is inside a block, else the first floor up to `fall`
/// blocks down (one in the air). None when there is none.
pub(crate) fn settle(w: &impl Blocks, c: [i32; 3], fall: i32) -> Option<[i32; 3]> {
    if stands(w, c) {
        return Some(c);
    }
    if !open(w.kind(c), None) && stands(w, up(c, 1)) {
        return Some(up(c, 1));
    }
    for k in 1..=fall {
        let below = up(c, -k);
        if stands(w, below) {
            return Some(below);
        }
        if !open(w.kind(below), None) {
            break;
        }
    }
    None
}

/// Next to the goal: within a block across and one up or down.
fn near(a: [i32; 3], goal: [i32; 3]) -> bool {
    (a[0] - goal[0]).abs() <= 1 && (a[2] - goal[2]).abs() <= 1 && (a[1] - goal[1]).abs() <= 1
}

fn heuristic(a: [i32; 3], goal: [i32; 3]) -> f32 {
    let dx = (a[0] - goal[0]).abs() as f32;
    let dz = (a[2] - goal[2]).abs() as f32;
    let dy = (a[1] - goal[1]).abs() as f32;
    let (lo, hi) = if dx < dz { (dx, dz) } else { (dz, dx) };
    (hi + lo * (std::f32::consts::SQRT_2 - 1.0) + dy * 0.5) * 1.1
}

fn key(c: [i32; 3]) -> u64 {
    (((c[0] + (1 << 23)) as u64 & 0xFF_FFFF) << 40)
        | (((c[1] + (1 << 15)) as u64 & 0xFFFF) << 24)
        | ((c[2] + (1 << 23)) as u64 & 0xFF_FFFF)
}

fn unkey(k: u64) -> [i32; 3] {
    [
        ((k >> 40) & 0xFF_FFFF) as i32 - (1 << 23),
        ((k >> 24) & 0xFFFF) as i32 - (1 << 15),
        (k & 0xFF_FFFF) as i32 - (1 << 23),
    ]
}

/// A quick hash for cell keys.
#[derive(Default)]
struct CellHasher(u64);

impl Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

type CellMap<V> = HashMap<u64, V, BuildHasherDefault<CellHasher>>;

#[derive(Clone, Copy)]
struct Node {
    g: f32,
    parent: u64,
    mv: Move,
    closed: bool,
}

#[derive(PartialEq)]
struct Open {
    f: f32,
    g: f32,
    key: u64,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Lowest f first; on a tie the one further along.
        other
            .f
            .total_cmp(&self.f)
            .then(self.g.total_cmp(&other.g))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

const DIRS: [[i32; 2]; 8] = [
    [1, 0],
    [-1, 0],
    [0, 1],
    [0, -1],
    [1, 1],
    [1, -1],
    [-1, 1],
    [-1, -1],
];

/// The blocks to claw away before taking a step of kind `mv` from `s` to
/// `t` (None when one of them may not be clawed), top first.
pub(crate) fn dig_for(w: &impl Blocks, s: [i32; 3], t: [i32; 3], mv: Move) -> Option<Vec<[i32; 3]>> {
    let axis = if t[0] != s[0] && t[2] == s[2] {
        Some(0)
    } else if t[2] != s[2] && t[0] == s[0] {
        Some(2)
    } else {
        None
    };
    let cells: Vec<([i32; 3], Option<usize>)> = match mv {
        Move::Walk => vec![(up(t, 1), axis), (t, axis)],
        Move::Climb(1) => vec![(up(s, 2), None), (up(t, 1), axis), (t, axis)],
        Move::DigDown => vec![(t, None)],
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for (c, ax) in cells {
        let k = w.kind(c);
        let shut = if mv == Move::DigDown { k != Kind::Air } else { !open(k, ax) };
        if shut {
            w.dig(c)?;
            out.push(c);
        }
    }
    Some(out)
}

/// The steps out of cell `s`: (next cell, move, cost).
fn neighbours(w: &impl Blocks, s: [i32; 3], opts: &Opts, out: &mut Vec<([i32; 3], Move, f32)>) {
    out.clear();
    let dig_cost = |blocks: &[[i32; 3]]| -> f32 {
        blocks
            .iter()
            .map(|b| w.dig(*b).unwrap_or(f32::INFINITY) * opts.dig_weight)
            .sum::<f32>()
    };
    for d in DIRS {
        let t = add(s, [d[0], 0, d[1]]);
        if d[0] != 0 && d[1] != 0 {
            // Diagonal: only a plain walk, every cell it brushes open and
            // floored (no corner cutting, nothing to claw).
            let a = add(s, [d[0], 0, 0]);
            let b = add(s, [0, 0, d[1]]);
            let clear = |c: [i32; 3]| {
                let k = w.kind(c);
                (k == Kind::Air || k == Kind::Low) && matches!(w.kind(up(c, 1)), Kind::Air | Kind::Low)
            };
            if clear(a) && clear(b) && clear(t) && stands(w, t) && stands(w, a) && stands(w, b) {
                out.push((t, Move::Walk, std::f32::consts::SQRT_2));
            }
            continue;
        }
        let axis = Some(if d[0] != 0 { 0 } else { 2 });
        let t1 = up(t, 1);
        // Across on the same level, clawing what is in the way.
        let floor = w.kind(up(t, -1)) == Kind::Solid || w.kind(t) == Kind::Low;
        if floor && let Some(dig) = dig_for(w, s, t, Move::Walk) {
            out.push((t, Move::Walk, 1.0 + dig_cost(&dig)));
        }
        // Up one: the block ahead is the step (what is above it and above
        // its own head clawed away).
        if (w.kind(t) == Kind::Solid || w.kind(t1) == Kind::Low)
            && let Some(dig) = dig_for(w, s, t1, Move::Climb(1))
        {
            out.push((t1, Move::Climb(1), 1.6 + dig_cost(&dig)));
        }
        // Two or three up, with the climb animation.
        if opts.tall_climbs {
            for k in 2..=3 {
                let tk = up(t, k);
                let column = (2..=k + 1).all(|j| open(w.kind(up(s, j)), None));
                if !column {
                    break;
                }
                if stands(w, tk) && open(w.kind(tk), axis) && open(w.kind(up(tk, 1)), axis) {
                    // Only the first floor up: a higher ledge over a lower
                    // one is reached one climb at a time.
                    if (1..k).all(|j| w.kind(up(t, j)) == Kind::Solid) {
                        out.push((tk, Move::Climb(k as u8), 1.0 + k as f32));
                    }
                    break;
                }
            }
        }
        // Off an edge.
        if !floor && open(w.kind(t), axis) && open(w.kind(t1), axis) {
            for k in 1..=4 {
                let c = up(t, -k);
                if !open(w.kind(c), None) {
                    break;
                }
                if stands(w, c) {
                    out.push((c, Move::Drop(k as u8), 1.0 + 0.3 * k as f32));
                    break;
                }
            }
        }
    }
    // Down through the floor.
    let below = up(s, -1);
    if w.kind(below) != Kind::Air && w.kind(up(s, -2)) == Kind::Solid {
        if let Some(dig) = dig_for(w, s, below, Move::DigDown) {
            out.push((below, Move::DigDown, 1.0 + dig_cost(&dig)));
        }
    }
}

/// The way from cell `start` to next to cell `goal` (both cells a body
/// stands in). When the budget runs out or there is no way, the way to the
/// cell it got closest to.
pub(crate) fn search(w: &impl Blocks, start: [i32; 3], goal: [i32; 3], opts: &Opts) -> Plan {
    if near(start, goal) {
        return Plan {
            steps: Vec::new(),
            reached: true,
            expanded: 0,
        };
    }
    let mut nodes: CellMap<Node> = CellMap::default();
    let mut heap = BinaryHeap::new();
    let sk = key(start);
    nodes.insert(
        sk,
        Node {
            g: 0.0,
            parent: sk,
            mv: Move::Walk,
            closed: false,
        },
    );
    heap.push(Open {
        f: heuristic(start, goal),
        g: 0.0,
        key: sk,
    });
    let mut best = (heuristic(start, goal), sk);
    let mut found = None;
    let mut expanded = 0usize;
    let mut next = Vec::with_capacity(32);
    while let Some(Open { g, key: k, .. }) = heap.pop() {
        let Some(node) = nodes.get_mut(&k) else { continue };
        if node.closed || g > node.g + 1e-4 {
            continue;
        }
        node.closed = true;
        let c = unkey(k);
        if near(c, goal) {
            found = Some(k);
            break;
        }
        let h = heuristic(c, goal);
        if h < best.0 {
            best = (h, k);
        }
        expanded += 1;
        if expanded >= opts.budget {
            break;
        }
        neighbours(w, c, opts, &mut next);
        for &(t, mv, cost) in &next {
            if !cost.is_finite() {
                continue;
            }
            let tk = key(t);
            let ng = g + cost;
            match nodes.get_mut(&tk) {
                Some(n) if n.closed || n.g <= ng => continue,
                Some(n) => {
                    n.g = ng;
                    n.parent = k;
                    n.mv = mv;
                }
                None => {
                    nodes.insert(
                        tk,
                        Node {
                            g: ng,
                            parent: k,
                            mv,
                            closed: false,
                        },
                    );
                }
            }
            heap.push(Open {
                f: ng + heuristic(t, goal),
                g: ng,
                key: tk,
            });
        }
    }
    let (end, reached) = match found {
        Some(k) => (k, true),
        None => (best.1, false),
    };
    let mut chain = Vec::new();
    let mut k = end;
    while k != sk {
        let n = nodes[&k];
        chain.push((unkey(n.parent), unkey(k), n.mv));
        k = n.parent;
    }
    chain.reverse();
    let steps = chain
        .into_iter()
        .map(|(from, to, mv)| Step {
            cell: to,
            mv,
            dig: dig_for(w, from, to, mv).unwrap_or_default(),
        })
        .collect();
    Plan {
        steps,
        reached,
        expanded,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A made-up world: ground of stone at y < 0 (unless dug), extra blocks.
    #[derive(Default)]
    pub(crate) struct Grid {
        pub blocks: HashMap<[i32; 3], Kind>,
        pub protected: HashSet<[i32; 3]>,
        pub ground: bool,
    }

    impl Grid {
        pub(crate) fn flat() -> Self {
            Self {
                ground: true,
                ..Self::default()
            }
        }
        pub(crate) fn set(&mut self, b: [i32; 3], k: Kind) {
            self.blocks.insert(b, k);
        }
        pub(crate) fn wall(&mut self, from: [i32; 3], to: [i32; 3], k: Kind) {
            for x in from[0]..=to[0] {
                for y in from[1]..=to[1] {
                    for z in from[2]..=to[2] {
                        self.set([x, y, z], k);
                    }
                }
            }
        }
    }

    impl Blocks for Grid {
        fn kind(&self, b: [i32; 3]) -> Kind {
            if let Some(k) = self.blocks.get(&b) {
                return *k;
            }
            if self.ground && b[1] < 0 {
                Kind::Solid
            } else {
                Kind::Air
            }
        }
        fn dig(&self, b: [i32; 3]) -> Option<f32> {
            (!self.protected.contains(&b)).then_some(2.0)
        }
    }

    fn ends(p: &Plan) -> [i32; 3] {
        p.steps.last().map(|s| s.cell).unwrap()
    }

    #[test]
    fn classify_shapes() {
        assert_eq!(classify(&[]), Kind::Air);
        assert_eq!(classify(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]), Kind::Solid);
        assert_eq!(classify(&[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]), Kind::Low);
        // A closed door on the north edge, and one open along the west edge.
        assert_eq!(classify(&[[0.0, 0.0, 0.0, 1.0, 1.0, 0.1875]]), Kind::PanelX);
        assert_eq!(classify(&[[0.8125, 0.0, 0.0, 1.0, 1.0, 1.0]]), Kind::PanelZ);
        // A glass pane through the middle is in the way.
        assert_eq!(classify(&[[0.0, 0.0, 0.4375, 1.0, 1.0, 0.5625]]), Kind::Solid);
    }

    #[test]
    fn walks_straight_on_flat_ground() {
        let w = Grid::flat();
        let p = search(&w, [0, 0, 0], [10, 0, 0], &Opts::default());
        assert!(p.reached);
        assert!(p.steps.iter().all(|s| s.mv == Move::Walk && s.dig.is_empty()));
        assert_eq!(ends(&p)[1], 0);
        assert!((ends(&p)[0] - 10).abs() <= 1);
        assert!(p.steps.len() <= 10);
    }

    #[test]
    fn steps_up_one_block_and_drops_down() {
        let mut w = Grid::flat();
        // A one-block ledge from x 5 on.
        w.wall([5, 0, -5], [20, 0, 5], Kind::Solid);
        let p = search(&w, [0, 0, 0], [10, 1, 0], &Opts::default());
        assert!(p.reached, "{p:?}");
        assert!(p.steps.iter().any(|s| s.mv == Move::Climb(1)));
        assert!(p.steps.iter().all(|s| s.dig.is_empty()));
        let back = search(&w, [10, 1, 0], [0, 0, 0], &Opts::default());
        assert!(back.reached);
        assert!(back.steps.iter().any(|s| s.mv == Move::Drop(1)));
    }

    #[test]
    fn walks_round_a_short_wall_rather_than_digging() {
        let mut w = Grid::flat();
        // A wall two high across z -2..2 at x 5: round it is 3 more steps.
        w.wall([5, 0, -2], [5, 1, 2], Kind::Solid);
        let p = search(&w, [0, 0, 0], [10, 0, 0], &Opts::default());
        assert!(p.reached);
        assert!(p.steps.iter().all(|s| s.dig.is_empty()), "{p:?}");
    }

    #[test]
    fn digs_through_a_long_wall() {
        let mut w = Grid::flat();
        w.wall([5, 0, -60], [5, 3, 60], Kind::Solid);
        let p = search(&w, [0, 0, 0], [10, 0, 0], &Opts::default());
        assert!(p.reached, "{p:?}");
        let dug: Vec<[i32; 3]> = p.steps.iter().flat_map(|s| s.dig.clone()).collect();
        assert_eq!(dug.len(), 2, "{dug:?}");
        assert!(dug.iter().all(|b| b[0] == 5 && b[1] <= 1));
    }

    #[test]
    fn never_digs_protected_blocks_and_uses_the_door() {
        // The spawn room: inner air x -8..8, z -6..6, y 0..4; walls ring
        // x +-9 / z +-7, roof y 5, floor y -1; doors at (0, 0..1, +-7).
        let mut w = Grid::flat();
        for x in -9i32..=9 {
            for z in -7i32..=7 {
                for y in -1..=5 {
                    let wall = x.abs() == 9 || z.abs() == 7 || y == -1 || y == 5;
                    if wall {
                        w.set([x, y, z], Kind::Solid);
                        w.protected.insert([x, y, z]);
                    }
                }
            }
        }
        for y in 0..=1 {
            for z in [-7, 7] {
                w.protected.remove(&[0, y, z]);
                w.set([0, y, z], Kind::PanelX);
            }
        }
        // From outside the north side to the middle of the room.
        let p = search(&w, [0, 0, -15], [0, 0, 0], &Opts::default());
        assert!(p.reached, "{p:?}");
        let dug: Vec<[i32; 3]> = p.steps.iter().flat_map(|s| s.dig.clone()).collect();
        assert!(!dug.is_empty());
        assert!(dug.iter().all(|b| !w.protected.contains(b)), "{dug:?}");
        assert!(dug.iter().all(|b| b[0] == 0 && b[2] == -7), "{dug:?}");
        // From the east side it goes round to a door too.
        let p = search(&w, [20, 0, 2], [0, 0, 0], &Opts::default());
        assert!(p.reached, "{p:?}");
        let dug: Vec<[i32; 3]> = p.steps.iter().flat_map(|s| s.dig.clone()).collect();
        assert!(dug.iter().all(|b| b[0] == 0 && b[2].abs() == 7), "{dug:?}");
        // An open door (panel along the doorway) is walked through.
        for y in 0..=1 {
            w.set([0, y, -7], Kind::PanelZ);
        }
        let p = search(&w, [0, 0, -15], [0, 0, 0], &Opts::default());
        assert!(p.reached);
        assert!(p.steps.iter().all(|s| s.dig.is_empty()), "{p:?}");
    }

    #[test]
    fn climbs_tall_ledges_only_with_the_animation() {
        let mut w = Grid::flat();
        w.wall([5, 0, -40], [40, 1, 40], Kind::Solid);
        let p = search(&w, [0, 0, 0], [10, 2, 0], &Opts::default());
        assert!(p.reached);
        assert!(p.steps.iter().any(|s| s.mv == Move::Climb(2)), "{p:?}");
        let no_anim = Opts {
            tall_climbs: false,
            ..Opts::default()
        };
        let p = search(&w, [0, 0, 0], [10, 2, 0], &no_anim);
        // Without it, it digs a step into the ledge.
        assert!(p.reached);
        assert!(p.steps.iter().all(|s| !matches!(s.mv, Move::Climb(k) if k > 1)));
        assert!(p.steps.iter().any(|s| !s.dig.is_empty()));
    }

    #[test]
    fn digs_down_to_a_player_underground() {
        let w = Grid::flat();
        let p = search(&w, [0, 0, 0], [0, -6, 0], &Opts::default());
        assert!(p.reached, "{p:?}");
        assert!(p.steps.iter().all(|s| s.dig.iter().all(|b| b[1] < 0)));
    }

    #[test]
    fn no_way_ends_at_the_closest_cell() {
        let mut w = Grid::flat();
        // Unbreakable box round the goal.
        for x in 8..=12 {
            for z in -2i32..=2 {
                for y in -1..=3 {
                    if x == 8 || x == 12 || z.abs() == 2 || y == -1 || y == 3 {
                        w.set([x, y, z], Kind::Solid);
                        w.protected.insert([x, y, z]);
                    }
                }
            }
        }
        let p = search(&w, [0, 0, 0], [10, 0, 0], &Opts::default());
        assert!(!p.reached);
        let end = ends(&p);
        assert!(end[0] >= 5, "{end:?}");
    }

    #[test]
    fn budget_is_kept() {
        let w = Grid::flat();
        let opts = Opts {
            budget: 50,
            ..Opts::default()
        };
        let p = search(&w, [0, 0, 0], [200, 0, 0], &opts);
        assert!(p.expanded <= 50);
        assert!(!p.reached);
        assert!(!p.steps.is_empty());
    }

    #[test]
    fn settle_finds_the_floor() {
        let w = Grid::flat();
        assert_eq!(settle(&w, [0, 0, 0], 4), Some([0, 0, 0]));
        assert_eq!(settle(&w, [0, 3, 0], 4), Some([0, 0, 0]));
        assert_eq!(settle(&w, [0, -1, 0], 4), Some([0, 0, 0]));
        assert_eq!(settle(&w, [0, 9, 0], 4), None);
    }
}
