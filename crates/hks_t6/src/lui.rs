//! bo2zm M4: the engine side of Black Ops II's LUI widgets: the element
//! objects `ConstructLUIElement` hands the scripts (`LUI.UIElement.new`),
//! their tree, anchored layout, colour, animation states and timed
//! animations. The widget classes themselves are the game's Lua
//! (ui/lui/*.lua); this is what they call into.
//!
//! An element is userdata whose metatable has `__newindex` = its own field
//! table (the scripts' `setClass` puts the class behind that table) and
//! `__index` = a lookup of that table first, then these natives.
//!
//! Layout: `setLeftRight(leftAnchor, rightAnchor, left, right)` places the
//! left and right edges from the parent's left edge, right edge, or (no
//! anchor) its centre; `setTopBottom` likewise. Animation:
//! `beginAnimation(name, ms, easeIn, easeOut)` starts from the current
//! state and the setters that follow give the end state;
//! `animateToState(name, ms, ...)` ends at a registered state. When one
//! ends the element gets the event `transition_complete_<name>`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::value::{Table, TableRef, UserData, Value};
use crate::vm::{LuaError, Vm};

type Res<T> = Result<T, LuaError>;

/// What an element shows, and where (relative to its parent).
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left_anchor: bool,
    pub top_anchor: bool,
    pub right_anchor: bool,
    pub bottom_anchor: bool,
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
    pub alpha_multiplier: f32,
    pub x_rot: f32,
    pub y_rot: f32,
    pub z_rot: f32,
    pub scale: f32,
    /// A material name (`RegisterMaterial`), when it draws a picture.
    pub material: Option<String>,
    pub font: Option<String>,
    /// LUI.Alignment: 0 none, 1 left, 2 centre, 3 right (also 4 top,
    /// 5 middle, 6 bottom).
    pub alignment: i32,
    pub text: Option<String>,
    /// bo2zm M4: the material's shader vectors 0..3 (`setShaderVector`):
    /// what an engine shader reads (the globe's turn is vector 2).
    pub shader: [[f32; 4]; 4],
}

impl Default for State {
    fn default() -> Self {
        State {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left_anchor: false,
            top_anchor: false,
            right_anchor: false,
            bottom_anchor: false,
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            alpha: 1.0,
            alpha_multiplier: 1.0,
            x_rot: 0.0,
            y_rot: 0.0,
            z_rot: 0.0,
            scale: 1.0,
            material: None,
            font: None,
            alignment: 0,
            text: None,
            shader: [[0.0; 4]; 4],
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl State {
    /// Between two states; pictures, fonts and text switch at the end, the
    /// anchors at the start (the edges carry the motion).
    fn mix(&self, to: &State, t: f32) -> State {
        State {
            left: lerp(self.left, to.left, t),
            top: lerp(self.top, to.top, t),
            right: lerp(self.right, to.right, t),
            bottom: lerp(self.bottom, to.bottom, t),
            red: lerp(self.red, to.red, t),
            green: lerp(self.green, to.green, t),
            blue: lerp(self.blue, to.blue, t),
            alpha: lerp(self.alpha, to.alpha, t),
            alpha_multiplier: lerp(self.alpha_multiplier, to.alpha_multiplier, t),
            x_rot: lerp(self.x_rot, to.x_rot, t),
            y_rot: lerp(self.y_rot, to.y_rot, t),
            z_rot: lerp(self.z_rot, to.z_rot, t),
            scale: lerp(self.scale, to.scale, t),
            left_anchor: to.left_anchor,
            top_anchor: to.top_anchor,
            right_anchor: to.right_anchor,
            bottom_anchor: to.bottom_anchor,
            material: if t >= 1.0 {
                to.material.clone()
            } else {
                self.material.clone()
            },
            font: to.font.clone(),
            alignment: to.alignment,
            text: to.text.clone(),
            shader: std::array::from_fn(|i| {
                std::array::from_fn(|j| lerp(self.shader[i][j], to.shader[i][j], t))
            }),
        }
    }

    /// The fields a registered state table names, over this state.
    fn apply_table(&mut self, t: &Table) {
        let num = |k: &str| t.get_str(k).as_num();
        let flag = |k: &str| match t.get_str(k) {
            Value::Nil => None,
            v => Some(v.truthy()),
        };
        if let Some(v) = num("left") {
            self.left = v;
        }
        if let Some(v) = num("top") {
            self.top = v;
        }
        if let Some(v) = num("right") {
            self.right = v;
        }
        if let Some(v) = num("bottom") {
            self.bottom = v;
        }
        if let Some(v) = flag("leftAnchor") {
            self.left_anchor = v;
        }
        if let Some(v) = flag("topAnchor") {
            self.top_anchor = v;
        }
        if let Some(v) = flag("rightAnchor") {
            self.right_anchor = v;
        }
        if let Some(v) = flag("bottomAnchor") {
            self.bottom_anchor = v;
        }
        if let Some(v) = num("red") {
            self.red = v;
        }
        if let Some(v) = num("green") {
            self.green = v;
        }
        if let Some(v) = num("blue") {
            self.blue = v;
        }
        // The real engine keeps one fade value: a state carrying only
        // `alphaMultiplier` (fade_in = {alphaMultiplier = 1}) brings back an
        // element made at alpha 0, so both keys set the same alpha here.
        match (num("alpha"), num("alphaMultiplier")) {
            (Some(a), Some(m)) => self.alpha = a * m,
            (Some(v), None) | (None, Some(v)) => self.alpha = v,
            (None, None) => {}
        }
        if let Some(v) = num("zRot") {
            self.z_rot = v;
        }
        if let Some(v) = num("scale") {
            self.scale = v;
        }
        match t.get_str("material") {
            Value::Str(s) => self.material = Some(s.to_string()),
            Value::User(u) => self.material = material_name(&u),
            _ => {}
        }
        // A font is a name or (`CoD.fonts.Condensed`) a RegisterFont handle.
        match t.get_str("font") {
            Value::Str(s) => self.font = Some(s.to_string()),
            Value::User(f) if f.kind == "font" => {
                if let Some(name) = f.data.borrow().downcast_ref::<String>() {
                    self.font = Some(name.clone());
                }
            }
            _ => {}
        }
        if let Some(v) = num("alignment") {
            self.alignment = v as i32;
        }
    }
}

/// A screen rectangle (x0, y0, x1, y1) in the root's units.
pub type Rect = [f32; 4];

/// Where an element's edges fall inside its parent's rectangle.
pub fn place(s: &State, parent: Rect) -> Rect {
    let axis =
        |lo_anchor: bool, hi_anchor: bool, lo: f32, hi: f32, p0: f32, p1: f32| -> (f32, f32) {
            let mid = (p0 + p1) * 0.5;
            match (lo_anchor, hi_anchor) {
                (true, true) => (p0 + lo, p1 + hi),
                (true, false) => (p0 + lo, p0 + hi),
                (false, true) => (p1 + lo, p1 + hi),
                (false, false) => (mid + lo, mid + hi),
            }
        };
    let (x0, x1) = axis(
        s.left_anchor,
        s.right_anchor,
        s.left,
        s.right,
        parent[0],
        parent[2],
    );
    let (y0, y1) = axis(
        s.top_anchor,
        s.bottom_anchor,
        s.top,
        s.bottom,
        parent[1],
        parent[3],
    );
    // setScale: the element (and so its children) grown about its centre
    // (the map zooms in with it).
    if (s.scale - 1.0).abs() > 1e-4 {
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let (hw, hh) = ((x1 - x0) * 0.5 * s.scale, (y1 - y0) * 0.5 * s.scale);
        return [cx - hw, cy - hh, cx + hw, cy + hh];
    }
    [x0, y0, x1, y1]
}

#[derive(Clone, Debug)]
struct Anim {
    name: String,
    from: State,
    to: State,
    start_ms: f64,
    duration_ms: f64,
    ease_in: bool,
    ease_out: bool,
}

/// The engine's part of one element.
pub struct Element {
    pub id: usize,
    pub parent: Option<Weak<UserData>>,
    pub children: Vec<Rc<UserData>>,
    pub priority: f32,
    /// The state now shown (animated toward `anim.to` while one runs).
    pub state: State,
    anim: Option<Anim>,
    /// The end state the setters write while an animation is open.
    pending: Option<State>,
    states: HashMap<String, TableRef>,
    pub use_stencil: bool,
    /// A list's gap between its children (`setSpacing`).
    pub spacing: f32,
    /// Where the last layout put it (`getRect`, the mouse's hit test).
    pub last_rect: Option<Rect>,
    /// Holds the menu focus (`setFocus`, `isInFocus`).
    pub focused: bool,
    /// A dashes bar's (count, lit) and units from one dash to the next
    /// (`setupDashes`).
    pub dashes: (i32, i32),
    pub dash_pitch: f32,
    pub kind: &'static str,
    /// A `UITightText`: as wide as its words (a list gives it that slot).
    pub tight: bool,
    /// A streamed picture (`setupUIStreamedImage`): its scripts wait for
    /// `streamed_image_ready` after each picture they set.
    pub streamed: bool,
    /// It blurs what is behind it (`setBlur`).
    pub blur: bool,
    /// The element's own field table (its metatable's `__newindex`).
    pub fields: TableRef,
}

pub fn material_name(u: &UserData) -> Option<String> {
    if u.kind != "material" {
        return None;
    }
    u.data.borrow().downcast_ref::<String>().cloned()
}

/// The element behind a value, if it is one.
pub fn element(v: &Value) -> Option<Rc<UserData>> {
    match v {
        Value::User(u) if u.kind == "LUIElement" => Some(u.clone()),
        _ => None,
    }
}

fn with<R>(u: &UserData, f: impl FnOnce(&mut Element) -> R) -> R {
    let mut data = u.data.borrow_mut();
    let e = data
        .downcast_mut::<Element>()
        .expect("LUIElement userdata holds an Element");
    f(e)
}

/// LUI's clock and the elements' animations (the engine drives `tick`).
pub struct Lui {
    pub now_ms: f64,
    next_id: usize,
    pub natives: TableRef,
    /// Elements whose animation ended since the last tick: the
    /// animation's name and whether a new one cut it short.
    finished: Vec<(Rc<UserData>, String, bool)>,
    /// Streamed pictures set since the last frame (`streamed_image_ready`).
    streamed_ready: Vec<Rc<UserData>>,
    pub roots: Vec<Rc<UserData>>,
    /// The roots' rectangle in their units (0,0 top left).
    pub root_rect: Rect,
}

thread_local! {
    static LUI: RefCell<Option<Rc<RefCell<Lui>>>> = const { RefCell::new(None) };
}

/// bo2zm (debugging aid): the element with engine id `id` under `root` and
/// its ancestors, nearest first: (id, script `id` field, last rect).
pub fn chain_of(root: &Rc<UserData>, id: usize) -> Vec<(usize, String, Option<Rect>)> {
    let mut stack = vec![root.clone()];
    while let Some(u) = stack.pop() {
        if with(&u, |e| e.id) == id {
            let mut out = Vec::new();
            let mut at = Some(u);
            while let Some(x) = at {
                let (eid, name, rect, parent) = with(&x, |e| {
                    (
                        e.id,
                        e.fields
                            .borrow()
                            .get_str("id")
                            .as_str()
                            .unwrap_or("")
                            .to_owned(),
                        e.last_rect,
                        e.parent.as_ref().and_then(Weak::upgrade),
                    )
                });
                out.push((eid, name, rect));
                at = parent;
            }
            return out;
        }
        stack.extend(with(&u, |e| e.children.clone()));
    }
    Vec::new()
}

/// An element's engine id (debugging).
pub fn with_id(u: &Rc<UserData>) -> usize {
    with(u, |e| e.id)
}

/// Streamed pictures set since the last call (each gets
/// `streamed_image_ready`).
pub fn take_streamed_ready() -> Vec<Rc<UserData>> {
    std::mem::take(&mut lui().borrow_mut().streamed_ready)
}

/// The element natives (`setLeftRight`, `removeElement`, ...), by name.
pub fn natives() -> TableRef {
    lui().borrow().natives.clone()
}

fn lui() -> Rc<RefCell<Lui>> {
    LUI.with(|l| l.borrow().clone())
        .expect("LUI natives installed")
}

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or(Value::Nil)
}

fn this(a: &[Value], f: &str) -> Res<Rc<UserData>> {
    element(&arg(a, 0)).ok_or_else(|| LuaError::new(format!("{f}: not called on an LUI element")))
}

fn ease(t: f64, ease_in: bool, ease_out: bool) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match (ease_in, ease_out) {
        (true, true) => t * t * (3.0 - 2.0 * t),
        (true, false) => t * t,
        (false, true) => 1.0 - (1.0 - t) * (1.0 - t),
        _ => t,
    }
}

/// The state the setters change now: the open animation's end, the
/// running animation's end as well as the shown state (LUI writes the
/// latest animation's target: Nuketown's map fades in when its picture is
/// ready, during its zoom), or the shown state.
fn edit(e: &mut Element, f: impl Fn(&mut State)) {
    match (e.pending.as_mut(), e.anim.as_mut()) {
        (Some(p), _) => f(p),
        (None, Some(a)) => {
            f(&mut a.to);
            f(&mut e.state);
        }
        (None, None) => f(&mut e.state),
    }
}

thread_local! {
    /// Element setup methods the scripts called that the engine lacks.
    static MISSING_METHODS: RefCell<std::collections::BTreeSet<String>> = RefCell::default();
}

/// The engine widget setups the scripts asked for that are not built.
pub fn missing_methods() -> Vec<String> {
    MISSING_METHODS.with(|m| m.borrow().iter().cloned().collect())
}

/// A new element (`ConstructLUIElement`).
pub fn construct(vm: &mut Vm, kind: &'static str) -> Value {
    let l = lui();
    let (id, natives) = {
        let mut l = l.borrow_mut();
        l.next_id += 1;
        (l.next_id, l.natives.clone())
    };
    let fields = Table::new_ref();
    fields
        .borrow_mut()
        .set_str("m_eventHandlers", Value::Table(Table::new_ref()));
    fields
        .borrow_mut()
        .set_str("m_animationStates", Value::Table(Table::new_ref()));
    let meta = Table::new_ref();
    let f2 = fields.clone();
    let index = vm.native("LUIElement_index", move |vm, a| {
        let key = arg(&a, 1);
        let own = vm.index(&Value::Table(f2.clone()), &key)?;
        if !matches!(own, Value::Nil) {
            return Ok(vec![own]);
        }
        let native = natives.borrow().get(&key);
        // An engine widget setup we do not draw yet (`setupGlobe`, ...): a
        // do-nothing method, named in `missing_methods` (what to build next).
        if matches!(native, Value::Nil)
            && let Some(name) = key.as_str().filter(|k| k.starts_with("setup"))
        {
            MISSING_METHODS.with(|m| m.borrow_mut().insert(name.to_owned()));
            return Ok(vec![vm.native("stub_setup", |_, _| Ok(vec![]))]);
        }
        Ok(vec![native])
    });
    meta.borrow_mut().set_str("__index", index);
    meta.borrow_mut()
        .set_str("__newindex", Value::Table(fields.clone()));
    let el = Element {
        id,
        parent: None,
        children: Vec::new(),
        priority: 0.0,
        state: State::default(),
        anim: None,
        pending: None,
        states: HashMap::new(),
        use_stencil: false,
        spacing: 0.0,
        last_rect: None,
        focused: false,
        dashes: (0, 0),
        dash_pitch: 8.0,
        kind,
        tight: false,
        streamed: false,
        blur: false,
        fields,
    };
    Value::User(Rc::new(UserData {
        kind: "LUIElement",
        data: RefCell::new(Box::new(el)),
        meta: RefCell::new(Some(meta)),
    }))
}

fn add_child(parent: &Rc<UserData>, child: &Rc<UserData>, at: Option<(usize, bool)>) {
    // Out of its old parent first.
    if let Some(old) = with(child, |e| e.parent.as_ref().and_then(Weak::upgrade)) {
        with(&old, |e| e.children.retain(|c| !Rc::ptr_eq(c, child)));
    }
    with(child, |e| e.parent = Some(Rc::downgrade(parent)));
    with(parent, |e| match at {
        Some((i, after)) => {
            let i = if after { i + 1 } else { i };
            e.children.insert(i.min(e.children.len()), child.clone());
        }
        None => e.children.push(child.clone()),
    });
}

fn sibling_index(parent: &Rc<UserData>, sib: &Rc<UserData>) -> Option<usize> {
    with(parent, |e| {
        e.children.iter().position(|c| Rc::ptr_eq(c, sib))
    })
}

/// Advance every running animation to `now_ms`; returns the elements whose
/// animation ended (their `transition_complete_<name>` goes to the
/// scripts).
/// Animations that ended: the element, the animation, whether a new one cut
/// it short, and how long after its end this tick saw it (`lateness`: a
/// 0 ms timer moves by it).
pub fn tick(now_ms: f64) -> Vec<(Rc<UserData>, String, bool, f64)> {
    let l = lui();
    l.borrow_mut().now_ms = now_ms;
    let roots = l.borrow().roots.clone();
    let mut done = Vec::new();
    // Tree order (parents first, children first to last): two animations
    // ending in the same frame report in the order the game's UI does
    // (leaving Custom Games, the map's ends before the globe's, which
    // resets the map's "to the corner" mark the map's handler reads).
    let mut stack: Vec<_> = roots.into_iter().rev().collect();
    while let Some(u) = stack.pop() {
        let ended = with(&u, |e| {
            stack.extend(e.children.iter().rev().cloned());
            let a = e.anim.as_ref()?;
            let t = if a.duration_ms <= 0.0 {
                1.0
            } else {
                (now_ms - a.start_ms) / a.duration_ms
            };
            let k = ease(t, a.ease_in, a.ease_out) as f32;
            e.state = a.from.mix(&a.to, if t >= 1.0 { 1.0 } else { k });
            if t >= 1.0 {
                let a = e.anim.take()?;
                e.state = a.to;
                Some((a.name, (now_ms - a.start_ms - a.duration_ms).max(0.0)))
            } else {
                None
            }
        });
        if let Some((name, late)) = ended {
            done.push((u.clone(), name, false, late));
        }
    }
    let mut l = l.borrow_mut();
    done.extend(l.finished.drain(..).map(|(u, n, i)| (u, n, i, 0.0)));
    done
}

/// Every element in drawing order (parents before children, children by
/// priority), with its rectangle and its alpha including its parents'.
pub fn layout(
    root: &Rc<UserData>,
    rect: Rect,
    measure: &dyn Fn(&str, &str, f32) -> f32,
) -> Vec<(Rc<UserData>, Rect, f32)> {
    layout_clipped(root, rect, measure)
        .into_iter()
        .map(|(u, r, a, _)| (u, r, a))
        .collect()
}

/// `layout`, with each element's clip: the box of its nearest ancestors
/// that clip their children (`setUseStencil`), intersected; None = none.
pub fn layout_clipped(
    root: &Rc<UserData>,
    rect: Rect,
    measure: &dyn Fn(&str, &str, f32) -> f32,
) -> Vec<(Rc<UserData>, Rect, f32, Option<Rect>)> {
    let mut out = Vec::new();
    // `slot`: a list's child is placed along the list's axis by the list
    // (vertical?, start, end); across it by its own anchors.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        u: &Rc<UserData>,
        parent: Rect,
        alpha: f32,
        slot: Option<(bool, f32, f32)>,
        clip: Option<Rect>,
        measure: &dyn Fn(&str, &str, f32) -> f32,
        out: &mut Vec<(Rc<UserData>, Rect, f32, Option<Rect>)>,
    ) {
        let (r, a, mut kids, list, stencil) = with(u, |e| {
            let mut r = place(&e.state, parent);
            match slot {
                Some((true, y0, y1)) => (r[1], r[3]) = (y0, y1),
                Some((false, x0, x1)) => (r[0], r[2]) = (x0, x1),
                None => {}
            }
            let a = alpha * e.state.alpha * e.state.alpha_multiplier;
            e.last_rect = Some(r);
            let list = match e.kind {
                "vlist" => Some((true, e.spacing, e.state.alignment)),
                "hlist" => Some((false, e.spacing, e.state.alignment)),
                _ => None,
            };
            (r, a, e.children.clone(), list, e.use_stencil)
        });
        out.push((u.clone(), r, a, clip));
        // A stencil clips its children to its own box.
        let clip = if stencil {
            let c = clip.unwrap_or([f32::MIN, f32::MIN, f32::MAX, f32::MAX]);
            Some([
                c[0].max(r[0].min(r[2])),
                c[1].max(r[1].min(r[3])),
                c[2].min(r[0].max(r[2])),
                c[3].min(r[1].max(r[3])),
            ])
        } else {
            clip
        };
        kids.sort_by(|x, y| {
            let px = with(x, |e| e.priority);
            let py = with(y, |e| e.priority);
            px.partial_cmp(&py).unwrap_or(std::cmp::Ordering::Equal)
        });
        let Some((vertical, spacing, align)) = list else {
            for k in &kids {
                walk(k, r, a, None, clip, measure, out);
            }
            return;
        };
        // A list: its children one after another along its axis, each its
        // own size, `spacing` apart; the run starts at the list's start,
        // middle or end (LUI.Alignment top/left, middle/centre,
        // bottom/right).
        let sizes: Vec<f32> = kids
            .iter()
            .map(|k| {
                with(k, |e| {
                    if vertical {
                        (e.state.bottom - e.state.top).abs()
                    } else if let (true, Some(text)) = (e.tight, e.state.text.as_deref()) {
                        // Tight text: its words' width at its height.
                        let h = (e.state.bottom - e.state.top).abs();
                        measure(text, e.state.font.as_deref().unwrap_or(""), h)
                    } else {
                        (e.state.right - e.state.left).abs()
                    }
                })
            })
            .collect();
        // A child with no size along the axis takes no room and no spacing
        // (ButtonList's two key-repeat helpers: BO2's Options menu starts
        // Settings at the list's top, not two spacings down).
        let placed = sizes.iter().filter(|s| **s > 0.0).count();
        let total = sizes.iter().sum::<f32>() + spacing * placed.saturating_sub(1) as f32;
        let (lo, hi) = if vertical { (r[1], r[3]) } else { (r[0], r[2]) };
        let mut at = match align {
            2 | 5 => (lo + hi) * 0.5 - total * 0.5,
            3 | 6 => hi - total,
            _ => lo,
        };
        for (k, size) in kids.iter().zip(sizes) {
            walk(k, r, a, Some((vertical, at, at + size)), clip, measure, out);
            if size > 0.0 {
                at += size + spacing;
            }
        }
    }
    walk(root, rect, 1.0, None, None, measure, &mut out);
    out
}

fn reg(
    vm: &mut Vm,
    t: &TableRef,
    name: &'static str,
    f: impl Fn(&mut Vm, Vec<Value>) -> Res<Vec<Value>> + 'static,
) {
    // T6LUA_TRACE=1 prints every element call (debugging aid).
    let trace = std::env::var_os("T6LUA_TRACE").is_some();
    let v = vm.native(name, move |vm, a| {
        if trace {
            let id = a
                .first()
                .and_then(element)
                .map_or_else(|| "-".to_owned(), |u| with(&u, |e| e.id.to_string()));
            let rest: Vec<String> = a.iter().skip(1).map(|v| format!("{v:?}")).collect();
            println!("  call #{id} {name}({})", rest.join(", "));
        }
        f(vm, a)
    });
    t.borrow_mut().set_str(name, v);
}

/// Install `ConstructLUIElement`, `RegisterMaterial` and the element
/// natives into the VM.
#[allow(clippy::too_many_lines)]
pub fn install(vm: &mut Vm) {
    let natives = Table::new_ref();
    let l = Rc::new(RefCell::new(Lui {
        now_ms: 0.0,
        next_id: 0,
        natives: natives.clone(),
        finished: Vec::new(),
        streamed_ready: Vec::new(),
        roots: Vec::new(),
        root_rect: [0.0, 0.0, 1280.0, 720.0],
    }));
    LUI.with(|slot| *slot.borrow_mut() = Some(l));
    let n = &natives;

    reg(vm, n, "setLeftRight", |_, a| {
        let u = this(&a, "setLeftRight")?;
        let (la, ra) = (arg(&a, 1).truthy(), arg(&a, 2).truthy());
        let (l, r) = (
            arg(&a, 3).as_num().unwrap_or(0.0),
            arg(&a, 4).as_num().unwrap_or(0.0),
        );
        with(&u, |e| {
            edit(e, |s| {
                s.left_anchor = la;
                s.right_anchor = ra;
                s.left = l;
                s.right = r;
            });
        });
        Ok(vec![])
    });
    reg(vm, n, "setTopBottom", |_, a| {
        let u = this(&a, "setTopBottom")?;
        let (ta, ba) = (arg(&a, 1).truthy(), arg(&a, 2).truthy());
        let (t, b) = (
            arg(&a, 3).as_num().unwrap_or(0.0),
            arg(&a, 4).as_num().unwrap_or(0.0),
        );
        with(&u, |e| {
            edit(e, |s| {
                s.top_anchor = ta;
                s.bottom_anchor = ba;
                s.top = t;
                s.bottom = b;
            });
        });
        Ok(vec![])
    });
    reg(vm, n, "setAlpha", |_, a| {
        let u = this(&a, "setAlpha")?;
        let v = arg(&a, 1).as_num().unwrap_or(1.0);
        with(&u, |e| edit(e, |s| s.alpha = v));
        Ok(vec![])
    });
    reg(vm, n, "setRGB", |_, a| {
        let u = this(&a, "setRGB")?;
        let c = [1, 2, 3].map(|i| arg(&a, i).as_num().unwrap_or(1.0));
        with(&u, |e| {
            edit(e, |s| {
                s.red = c[0];
                s.green = c[1];
                s.blue = c[2];
            });
        });
        Ok(vec![])
    });
    reg(vm, n, "setScale", |_, a| {
        let u = this(&a, "setScale")?;
        let v = arg(&a, 1).as_num().unwrap_or(1.0);
        with(&u, |e| edit(e, |s| s.scale = v));
        Ok(vec![])
    });
    for (name, axis) in [("setXRot", 0usize), ("setYRot", 1), ("setZRot", 2)] {
        reg(vm, n, name, move |_, a| {
            let u = this(&a, name)?;
            let v = arg(&a, 1).as_num().unwrap_or(0.0);
            with(&u, |e| {
                edit(e, |s| match axis {
                    0 => s.x_rot = v,
                    1 => s.y_rot = v,
                    _ => s.z_rot = v,
                });
            });
            Ok(vec![])
        });
    }
    reg(vm, n, "setImage", |_, a| {
        let u = this(&a, "setImage")?;
        let m = match arg(&a, 1) {
            Value::User(m) => material_name(&m),
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        };
        let streamed = with(&u, |e| {
            edit(e, |s| s.material = m.clone());
            e.streamed
        });
        // Our pictures are resident: a streamed one is ready at once.
        if streamed {
            lui().borrow_mut().streamed_ready.push(u);
        }
        Ok(vec![])
    });
    for name in ["setText", "setTextInC"] {
        reg(vm, n, name, move |vm, a| {
            let u = this(&a, name)?;
            let t = match arg(&a, 1) {
                Value::Nil => None,
                v => Some(vm.tostring(&v)?),
            };
            with(&u, |e| edit(e, |s| s.text = t.clone()));
            Ok(vec![])
        });
    }
    reg(vm, n, "setFont", |_, a| {
        let u = this(&a, "setFont")?;
        let f = match arg(&a, 1) {
            Value::Str(s) => Some(s.to_string()),
            Value::User(f) => f.data.borrow().downcast_ref::<String>().cloned(),
            _ => None,
        };
        with(&u, |e| edit(e, |s| s.font = f.clone()));
        Ok(vec![])
    });
    reg(vm, n, "setAlignment", |_, a| {
        let u = this(&a, "setAlignment")?;
        let v = arg(&a, 1).as_num().unwrap_or(0.0) as i32;
        with(&u, |e| edit(e, |s| s.alignment = v));
        Ok(vec![])
    });
    reg(vm, n, "setSpacing", |_, a| {
        let u = this(&a, "setSpacing")?;
        let v = arg(&a, 1).as_num().unwrap_or(0.0);
        with(&u, |e| e.spacing = v);
        Ok(vec![])
    });
    reg(vm, n, "setPriority", |_, a| {
        let u = this(&a, "setPriority")?;
        let v = arg(&a, 1).as_num().unwrap_or(0.0);
        with(&u, |e| e.priority = v);
        Ok(vec![])
    });
    reg(vm, n, "setUseStencil", |_, a| {
        let u = this(&a, "setUseStencil")?;
        let v = arg(&a, 1).truthy();
        with(&u, |e| e.use_stencil = v);
        Ok(vec![])
    });
    // setShaderVector(index, x, y, z, w): one of the material's shader
    // vectors (animated like the rest); getShaderVector<n>() reads it now.
    reg(vm, n, "setShaderVector", |_, a| {
        let u = this(&a, "setShaderVector")?;
        let i = arg(&a, 1).as_num().map_or(0, |n| n as usize).min(3);
        let v: [f32; 4] = std::array::from_fn(|k| arg(&a, k + 2).as_num().unwrap_or(0.0));
        with(&u, |e| edit(e, |s| s.shader[i] = v));
        Ok(vec![])
    });
    for (name, i) in [
        ("getShaderVector0", 0),
        ("getShaderVector1", 1),
        ("getShaderVector2", 2),
        ("getShaderVector3", 3),
    ] {
        reg(vm, n, name, move |_, a| {
            let u = this(&a, name)?;
            let v = with(&u, |e| e.state.shader[i]);
            Ok(v.iter().map(|x| Value::Num(*x)).collect())
        });
    }
    for name in [
        "setLayoutCached",
        "setUseGameTime",
        "updateElementLayout",
        "layoutChildren",
        "setRoot",
    ] {
        reg(vm, n, name, |_, _| Ok(vec![]));
    }
    // setupDashes(count, filled, ...): a slider's bar, `count` dashes of
    // which `filled` are lit (the engine draws it).
    reg(vm, n, "setupDashes", |_, a| {
        let u = this(&a, "setupDashes")?;
        let count = arg(&a, 1).as_num().unwrap_or(0.0) as i32;
        let filled = arg(&a, 2).as_num().unwrap_or(0.0) as i32;
        // (count, lit, gap, dash width): 20 dashes 8 units apart make the
        // slider's 160-unit bar.
        let pitch = arg(&a, 3).as_num().unwrap_or(0.0) + arg(&a, 4).as_num().unwrap_or(8.0);
        with(&u, |e| {
            e.kind = "dashes";
            e.dashes = (count.max(0), filled.clamp(0, count.max(0)));
            e.dash_pitch = pitch.max(1.0);
        });
        Ok(vec![])
    });
    // setupVoiceMeter(count): Settings' Level Indicator, `count` square
    // dashes 14 apart lit by how loud the microphone is (none here: no
    // voice chat, so all unlit, as the real one sits with no one talking).
    reg(vm, n, "setupVoiceMeter", |_, a| {
        let u = this(&a, "setupVoiceMeter")?;
        let count = arg(&a, 1).as_num().unwrap_or(20.0) as i32;
        with(&u, |e| {
            e.kind = "meter";
            e.dashes = (count.max(0), 0);
            e.dash_pitch = 14.0;
        });
        Ok(vec![])
    });
    // Focus: the scripts move it (gain_focus / lose_focus) and ask it.
    reg(vm, n, "setFocus", |_, a| {
        let u = this(&a, "setFocus")?;
        let on = arg(&a, 1).truthy();
        with(&u, |e| e.focused = on);
        Ok(vec![])
    });
    reg(vm, n, "isInFocus", |_, a| {
        let u = this(&a, "isInFocus")?;
        Ok(vec![Value::Bool(with(&u, |e| e.focused))])
    });
    reg(vm, n, "registerAnimationState", |_, a| {
        let u = this(&a, "registerAnimationState")?;
        let name = arg(&a, 1).as_str().unwrap_or("").to_owned();
        if let Value::Table(t) = arg(&a, 2) {
            // The scripts also read them: `self.m_animationStates.<name>`.
            with(&u, |e| {
                if let Value::Table(m) = e.fields.borrow().get_str("m_animationStates") {
                    m.borrow_mut().set_str(&name, Value::Table(t.clone()));
                }
                e.states.insert(name, t)
            });
        }
        Ok(vec![])
    });
    reg(vm, n, "beginAnimation", |_, a| {
        let u = this(&a, "beginAnimation")?;
        let name = arg(&a, 1).as_str().unwrap_or("").to_owned();
        let ms = f64::from(arg(&a, 2).as_num().unwrap_or(0.0));
        let (ei, eo) = (arg(&a, 3).truthy(), arg(&a, 4).truthy());
        let now = lui().borrow().now_ms;
        let interrupted = with(&u, |e| {
            let old = e.anim.take().map(|a| a.name);
            let from = e.state.clone();
            e.pending = Some(from.clone());
            e.anim = Some(Anim {
                name,
                from: from.clone(),
                to: from,
                start_ms: now,
                duration_ms: ms,
                ease_in: ei,
                ease_out: eo,
            });
            old
        });
        if let Some(old) = interrupted {
            lui().borrow_mut().finished.push((u, old, true));
        }
        Ok(vec![])
    });
    reg(vm, n, "animateToState", |_, a| {
        let u = this(&a, "animateToState")?;
        let name = arg(&a, 1).as_str().unwrap_or("").to_owned();
        let ms = f64::from(arg(&a, 2).as_num().unwrap_or(0.0));
        let (ei, eo) = (arg(&a, 3).truthy(), arg(&a, 4).truthy());
        let now = lui().borrow().now_ms;
        with(&u, |e| {
            let Some(t) = e.states.get(&name).cloned() else {
                return;
            };
            let mut to = e.state.clone();
            to.apply_table(&t.borrow());
            // A list's gap can be a state field too (MFTabManager's
            // SetTabSpacing registers {spacing = 20}).
            if let Some(v) = t.borrow().get_str("spacing").as_num() {
                e.spacing = v;
            }
            e.pending = None;
            if ms <= 0.0 {
                e.state = to;
                e.anim = None;
            } else {
                e.anim = Some(Anim {
                    name: name.clone(),
                    from: e.state.clone(),
                    to,
                    start_ms: now,
                    duration_ms: ms,
                    ease_in: ei,
                    ease_out: eo,
                });
            }
        });
        Ok(vec![])
    });
    // completeAnimation: the running animation jumps to its end (the
    // values written since beginAnimation included), and, as LUI does it,
    // its `transition_complete_<name>` runs now, before the script's next
    // line, marked `interrupted`: a looping handler stops there (the menu
    // button's pulse_high/pulse_low loop ends when it loses focus; without
    // the mark every button once passed over kept pulsing grey), and the
    // globe's handlers skip it before its next move starts.
    reg(vm, n, "completeAnimation", |vm, a| {
        let u = this(&a, "completeAnimation")?;
        let name = with(&u, |e| {
            let pending = e.pending.take();
            let a = e.anim.take()?;
            e.state = pending.unwrap_or(a.to);
            Some(a.name)
        });
        if let Some(name) = name {
            let target = Value::User(u);
            let t = Table::new_ref();
            t.borrow_mut()
                .set_str("name", Value::str(&format!("transition_complete_{name}")));
            t.borrow_mut().set_str("controller", Value::Num(0.0));
            t.borrow_mut().set_str("lateness", Value::Num(0.0));
            t.borrow_mut().set_str("interrupted", Value::Bool(true));
            let pe = vm.index(&target, &Value::str("processEvent"))?;
            if !matches!(pe, Value::Nil) {
                vm.call(pe, vec![target, Value::Table(t)])?;
            }
        }
        Ok(vec![])
    });
    // The setters after beginAnimation write `pending`; it becomes the
    // animation's end when the next frame starts it running.
    reg(vm, n, "addElementToC", |_, a| {
        let (p, c) = (this(&a, "addElementToC")?, element(&arg(&a, 1)));
        if let Some(c) = c {
            add_child(&p, &c, None);
        }
        Ok(vec![])
    });
    for (name, after) in [("addElementBeforeInC", false), ("addElementAfterInC", true)] {
        reg(vm, n, name, move |_, a| {
            // self:addElementBeforeInC(sibling): self goes before sibling.
            let (me, sib) = (this(&a, name)?, element(&arg(&a, 1)));
            let Some(sib) = sib else { return Ok(vec![]) };
            let Some(parent) = with(&sib, |e| e.parent.as_ref().and_then(Weak::upgrade)) else {
                return Ok(vec![]);
            };
            let i = sibling_index(&parent, &sib).unwrap_or(0);
            add_child(&parent, &me, Some((i, after)));
            Ok(vec![])
        });
    }
    reg(vm, n, "removeFromParentInC", |_, a| {
        let u = this(&a, "removeFromParentInC")?;
        if let Some(p) = with(&u, |e| e.parent.take().and_then(|w| w.upgrade())) {
            with(&p, |e| e.children.retain(|c| !Rc::ptr_eq(c, &u)));
        }
        Ok(vec![])
    });
    // removeElement(child): the child out of this element.
    reg(vm, n, "removeElement", |_, a| {
        let p = this(&a, "removeElement")?;
        if let Some(c) = element(&arg(&a, 1)) {
            let mine = with(&c, |e| e.parent.as_ref().and_then(Weak::upgrade))
                .is_some_and(|q| Rc::ptr_eq(&q, &p));
            if mine {
                with(&c, |e| e.parent = None);
                with(&p, |e| e.children.retain(|x| !Rc::ptr_eq(x, &c)));
            }
        }
        Ok(vec![])
    });
    reg(vm, n, "removeAllChildren", |_, a| {
        let u = this(&a, "removeAllChildren")?;
        let kids = with(&u, |e| std::mem::take(&mut e.children));
        for k in kids {
            with(&k, |e| e.parent = None);
        }
        Ok(vec![])
    });
    reg(vm, n, "getParent", |_, a| {
        let u = this(&a, "getParent")?;
        Ok(vec![
            with(&u, |e| e.parent.as_ref().and_then(Weak::upgrade)).map_or(Value::Nil, Value::User),
        ])
    });
    reg(vm, n, "getFirstChild", |_, a| {
        let u = this(&a, "getFirstChild")?;
        Ok(vec![
            with(&u, |e| e.children.first().cloned()).map_or(Value::Nil, Value::User),
        ])
    });
    reg(vm, n, "getLastChild", |_, a| {
        let u = this(&a, "getLastChild")?;
        Ok(vec![
            with(&u, |e| e.children.last().cloned()).map_or(Value::Nil, Value::User),
        ])
    });
    reg(vm, n, "getNumChildren", |_, a| {
        let u = this(&a, "getNumChildren")?;
        Ok(vec![Value::Num(with(&u, |e| e.children.len()) as f32)])
    });
    reg(vm, n, "getNextSibling", |_, a| {
        let u = this(&a, "getNextSibling")?;
        let Some(p) = with(&u, |e| e.parent.as_ref().and_then(Weak::upgrade)) else {
            return Ok(vec![Value::Nil]);
        };
        let i = sibling_index(&p, &u);
        Ok(vec![
            i.and_then(|i| with(&p, |e| e.children.get(i + 1).cloned()))
                .map_or(Value::Nil, Value::User),
        ])
    });
    reg(vm, n, "getPreviousSibling", |_, a| {
        let u = this(&a, "getPreviousSibling")?;
        let Some(p) = with(&u, |e| e.parent.as_ref().and_then(Weak::upgrade)) else {
            return Ok(vec![Value::Nil]);
        };
        let i = sibling_index(&p, &u);
        Ok(vec![
            i.filter(|i| *i > 0)
                .and_then(|i| with(&p, |e| e.children.get(i - 1).cloned()))
                .map_or(Value::Nil, Value::User),
        ])
    });
    reg(vm, n, "getRect", |_, a| {
        // The rectangle in the root's units (left, top, right, bottom).
        let u = this(&a, "getRect")?;
        // As last drawn (lists place their children), else from the anchors.
        if let Some(r) = with(&u, |e| e.last_rect) {
            return Ok(r.iter().map(|v| Value::Num(*v)).collect());
        }
        let mut chain = vec![u.clone()];
        let mut at = u;
        while let Some(p) = with(&at, |e| e.parent.as_ref().and_then(Weak::upgrade)) {
            chain.push(p.clone());
            at = p;
        }
        let mut rect: Rect = lui().borrow().root_rect;
        for e in chain.iter().rev() {
            rect = with(e, |e| place(&e.state, rect));
        }
        Ok(rect.iter().map(|v| Value::Num(*v)).collect())
    });
    // setupUIImage / setupUIText / ...: the element's kind.
    for (name, kind) in [
        ("setupUIElement", "element"),
        ("setupUIImage", "image"),
        // bo2zm M4: the Zombies globe (its material's shader draws it).
        ("setupGlobe", "globe"),
        ("setupUIText", "text"),
        ("setupUITextUncached", "text"),
        ("setupUIHorizontalList", "hlist"),
        ("setupUIVerticalList", "vlist"),
        ("setupGameTimer", "timer"),
        ("setupGameTimerZombie", "timer"),
        // Engine-drawn widgets not drawn yet: their kind names them.
        ("setupLoadingBar", "loadingbar"),
        ("setupLoadingStatusText", "text"),
        ("setupSafeAreaBoundary", "element"),
        ("setupEntityContainer", "entity"),
        ("setupGameMessages", "messages"),
        ("setupObjectiveProgress", "progress"),
        ("setupHUDShaker", "element"),
        ("setupCinematicSubtitles", "subtitles"),
        ("setupTiles", "image"),
        ("setupEdgePointer", "pointer"),
        ("setupHorizontalCompass", "compass"),
        ("setupPlayerHealthEKG", "element"),
        ("setupVisorImage", "image"),
        ("setupImageViewer", "image"),
        ("setupPlayerEmblemServer", "image"),
        ("setupPlayerEmblemByXUID", "image"),
        ("setupLeagueEmblem", "image"),
    ] {
        reg(vm, n, name, move |_, a| {
            let u = this(&a, name)?;
            with(&u, |e| e.kind = kind);
            Ok(vec![])
        });
    }
    // setupVoipImage(clientNum): the engine's speaker beside a player's
    // name: the plain grey speaker (`voice_quiet`), as nobody talks here.
    reg(vm, n, "setupVoipImage", |_, a| {
        let u = this(&a, "setupVoipImage")?;
        with(&u, |e| {
            e.kind = "image";
            edit(e, |s| s.material = Some("voice_quiet".to_owned()));
        });
        Ok(vec![])
    });
    reg(vm, n, "setupUIStreamedImage", |_, a| {
        let u = this(&a, "setupUIStreamedImage")?;
        let has_picture = with(&u, |e| {
            e.kind = "image";
            e.streamed = true;
            e.pending.as_ref().unwrap_or(&e.state).material.is_some()
        });
        // The picture set before it (the map screen sets it first) is ready.
        if has_picture {
            lui().borrow_mut().streamed_ready.push(u);
        }
        Ok(vec![])
    });
    reg(vm, n, "setupUITightText", |_, a| {
        let u = this(&a, "setupUITightText")?;
        with(&u, |e| {
            e.kind = "text";
            e.tight = true;
        });
        Ok(vec![])
    });

    // setBlur(on): the element blurs what is drawn behind it (a front-end
    // popup over its menu).
    reg(vm, n, "setBlur", |_, a| {
        let u = this(&a, "setBlur")?;
        let on = arg(&a, 1).truthy();
        with(&u, |e| e.blur = on);
        Ok(vec![])
    });
    // Engine settings on widgets not drawn yet (no effect here).
    for name in [
        "setZoom",
        "setTileVertically",
        "setEntityContainerClamp",
        "setEntityContainerFadeWhenTargeted",
        "setEntityContainerStopUpdating",
        "setOwnerControllerIndex",
        "setUI3DWindow",
        "addCompass",
        "addCrosshairDistance",
    ] {
        reg(vm, n, name, |_, _| Ok(vec![]));
    }

    let construct_fn = vm.native("ConstructLUIElement", |vm, _| {
        Ok(vec![construct(vm, "element")])
    });
    vm.set_global("ConstructLUIElement", construct_fn);
    let register_material = vm.native("RegisterMaterial", |_, a| {
        let name = arg(&a, 0).as_str().unwrap_or("").to_owned();
        Ok(vec![Value::User(Rc::new(UserData {
            kind: "material",
            data: RefCell::new(Box::new(name)),
            meta: RefCell::new(None),
        }))])
    });
    vm.set_global("RegisterMaterial", register_material);
    let register_font = vm.native("RegisterFont", |_, a| {
        let name = arg(&a, 0).as_str().unwrap_or("").to_owned();
        Ok(vec![Value::User(Rc::new(UserData {
            kind: "font",
            data: RefCell::new(Box::new(name)),
            meta: RefCell::new(None),
        }))])
    });
    vm.set_global("RegisterFont", register_font);
    let pairs = vm.global("pairs");
    vm.set_global("hpairs", pairs);
    // ProjectRootCoordinate(rootName, x, y): the engine's mouse position in a
    // root's units (the host sends it in root units already).
    let project = vm.native("ProjectRootCoordinate", |_, a| {
        Ok(vec![arg(&a, 1), arg(&a, 2)])
    });
    vm.set_global("ProjectRootCoordinate", project);
}

/// Start the animations whose end the setters have written since their
/// `beginAnimation` (called once per frame before `tick`).
pub fn commit_pending() {
    let l = lui();
    let roots = l.borrow().roots.clone();
    let mut stack = roots;
    while let Some(u) = stack.pop() {
        with(&u, |e| {
            stack.extend(e.children.iter().cloned());
            if let Some(p) = e.pending.take()
                && let Some(a) = e.anim.as_mut()
            {
                a.to = p;
            }
        });
    }
}

/// Make an element a root (its tree is laid out and animated).
pub fn add_root(u: Rc<UserData>) {
    lui().borrow_mut().roots.push(u);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_place_edges() {
        let parent = [0.0, 0.0, 100.0, 50.0];
        let mut s = State {
            left_anchor: true,
            right_anchor: true,
            left: 10.0,
            right: -10.0,
            ..State::default()
        };
        assert_eq!(place(&s, parent)[0..1], [10.0]);
        assert_eq!(place(&s, parent)[2], 90.0);
        s.right_anchor = false;
        s.right = 30.0;
        assert_eq!(place(&s, parent)[2], 30.0);
        s.left_anchor = false;
        s.left = -5.0;
        s.right = 5.0;
        let r = place(&s, parent);
        assert_eq!((r[0], r[2]), (45.0, 55.0));
    }

    /// A vertical list stacks its children by their own heights, `spacing`
    /// apart, from its top (or centred, LUI.Alignment.Middle = 5).
    #[test]
    fn vertical_list_stacks_children() {
        let mut vm = Vm::new();
        install(&mut vm);
        let list = element(&construct(&mut vm, "vlist")).unwrap();
        with(&list, |e| {
            e.spacing = 2.0;
            e.state.left_anchor = true;
            e.state.right_anchor = true;
            e.state.top_anchor = true;
            e.state.bottom_anchor = true;
        });
        for h in [30.0, 30.0] {
            let c = element(&construct(&mut vm, "element")).unwrap();
            with(&c, |e| {
                e.state.left_anchor = true;
                e.state.right_anchor = true;
                e.state.top_anchor = true;
                e.state.bottom = h;
            });
            add_child(&list, &c, None);
        }
        let laid = layout(&list, [0.0, 0.0, 100.0, 100.0], &|_, _, _| 0.0);
        let tops: Vec<(f32, f32)> = laid.iter().skip(1).map(|(_, r, _)| (r[1], r[3])).collect();
        assert_eq!(tops, vec![(0.0, 30.0), (32.0, 62.0)]);
        with(&list, |e| e.state.alignment = 5);
        let laid = layout(&list, [0.0, 0.0, 100.0, 100.0], &|_, _, _| 0.0);
        assert_eq!(laid[1].1[1], 19.0);
    }
}

/// The names of the element natives installed (after `install`).
pub fn native_names() -> Vec<String> {
    let l = lui();
    let natives = l.borrow().natives.clone();
    let t = natives.borrow();
    let mut out = Vec::new();
    let mut k = Value::Nil;
    while let Some((key, _)) = t.next(&k) {
        out.push(key.to_string());
        k = key;
    }
    out
}

/// LUI's clock (the last `tick`'s time).
pub fn now_ms() -> f64 {
    lui().borrow().now_ms
}

/// Set the roots' rectangle (the host, on a resize).
pub fn set_root_rect(r: Rect) {
    lui().borrow_mut().root_rect = r;
}

/// The element tree under `u` as indented lines: id, kind, focus, the
/// fields that gate input (`m_inputDisabled`, `m_ownerController`) and
/// the element's `id` field (debugging aid).
pub fn tree(u: &Rc<UserData>, depth: usize, out: &mut Vec<String>) {
    let (line, kids) = with(u, |e| {
        let f = e.fields.borrow();
        let name = f.get_str("id");
        let off = f.get_str("m_inputDisabled");
        let owner = f.get_str("m_ownerController");
        (
            format!(
                "{:indent$}#{} {}{}{}{} {}",
                "",
                e.id,
                e.kind,
                if e.focused { " FOCUS" } else { "" },
                if off.truthy() { " input-off" } else { "" },
                if matches!(owner, Value::Nil) {
                    String::new()
                } else {
                    format!(" owner={owner}")
                },
                if matches!(name, Value::Nil) {
                    String::new()
                } else {
                    name.to_string()
                },
                indent = depth * 2
            ),
            e.children.clone(),
        )
    });
    out.push(line);
    for k in &kids {
        tree(k, depth + 1, out);
    }
}

/// The element with this id under `u` (debugging aid).
pub fn find(u: &Rc<UserData>, id: usize) -> Option<Rc<UserData>> {
    let (me, kids) = with(u, |e| (e.id == id, e.children.clone()));
    if me {
        return Some(u.clone());
    }
    kids.iter().find_map(|k| find(k, id))
}

/// The `id` fields of an element's children (a menu's is `Menu.<name>`).
/// bo2zm M4 (debugging aid): every menu under `u` (an element whose `id`
/// starts with `Menu.`), with its input state: `m_inputDisabled`,
/// `occludedBy` (a popup over it), `m_ownerController`,
/// `anyControllerAllowed`, and its focused child's id.
pub fn menu_report(u: &Rc<UserData>) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(u.clone(), 0usize)];
    while let Some((x, depth)) = stack.pop() {
        let (fields, kids) = with(&x, |e| (e.fields.clone(), e.children.clone()));
        let f = fields.borrow();
        if let Some(id) = f.get_str("id").as_str().filter(|i| i.starts_with("Menu.")) {
            let show = |k: &str| {
                let v = f.get_str(k);
                match v {
                    Value::Nil => "-".to_owned(),
                    Value::User(_) | Value::Table(_) => "set".to_owned(),
                    other => other.to_string(),
                }
            };
            out.push(format!(
                "{}{id}: inputDisabled {} occludedBy {} owner {} anyController {} disabled {}",
                "  ".repeat(depth),
                show("m_inputDisabled"),
                show("occludedBy"),
                show("m_ownerController"),
                show("anyControllerAllowed"),
                show("m_disableAllButtons")
            ));
        }
        for k in kids.into_iter().rev() {
            stack.push((k, depth + 1));
        }
    }
    out
}

pub fn child_ids(u: &Rc<UserData>) -> Vec<String> {
    let kids = with(u, |e| e.children.clone());
    kids.iter()
        .filter_map(|k| {
            with(k, |e| {
                e.fields.borrow().get_str("id").as_str().map(str::to_owned)
            })
        })
        .collect()
}

/// The engine ids of the element whose `id` field is `name` and of all
/// under it (a menu's elements), if it is in this tree.
pub fn ids_under_named(u: &Rc<UserData>, name: &str) -> Option<std::collections::HashSet<usize>> {
    let (named, kids) = with(u, |e| {
        (
            e.fields.borrow().get_str("id").as_str() == Some(name),
            e.children.clone(),
        )
    });
    if named {
        let mut out = std::collections::HashSet::new();
        let mut stack = vec![u.clone()];
        while let Some(x) = stack.pop() {
            with(&x, |e| {
                out.insert(e.id);
                stack.extend(e.children.iter().cloned());
            });
        }
        return Some(out);
    }
    kids.iter().find_map(|k| ids_under_named(k, name))
}

/// An element's id.
pub fn element_id(u: &Rc<UserData>) -> usize {
    with(u, |e| e.id)
}
