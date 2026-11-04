//! 1.8.9 block facts the movement code needs: identities, material flags (from `ghast-data`),
//! and collision boxes. Static shapes, including the few that vanilla builds in `float` world
//! coordinates (cactus, cake, fence gates), come from `ghast-data`'s exact world boxes; the
//! neighbour-dependent shapes (fences, panes, walls, stairs, doors) are computed here.

use ghast_data::v1_8::BlockState;

use ghast_data::v1_8::MaterialKind;

use crate::math::{Aabb, BoxOps};
use crate::world::BlockView;

/// Block ids used by identity in 1.8 movement code.
pub(crate) mod id {
    pub const GLASS: u8 = 20;
    pub const WEB: u8 = 30;
    pub const LADDER: u8 = 65;
    pub const SOUL_SAND: u8 = 88;
    pub const STAINED_GLASS: u8 = 95;
    pub const VINE: u8 = 106;
    pub const NETHER_BRICK_FENCE: u8 = 113;
    pub const COBBLESTONE_WALL: u8 = 139;
    pub const CHEST: u8 = 54;
    pub const ANVIL: u8 = 145;
    pub const TRAPPED_CHEST: u8 = 146;
    pub const SLIME: u8 = 165;
    pub const BARRIER: u8 = 166;
    pub const PUMPKIN: u8 = 86;
    pub const LIT_PUMPKIN: u8 = 91;
    pub const MELON_BLOCK: u8 = 103;
}

/// How a block's collision boxes are produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShapeKind {
    /// The static table shape from `ghast-data`, placed with its exact world-coordinate rules.
    Table,
    /// Wooden fences (they connect to each other) — all share the wood material.
    WoodFence,
    /// Nether brick fence (rock material: does not connect to wooden fences).
    NetherFence,
    /// Glass panes and iron bars.
    Pane,
    /// Cobblestone wall.
    Wall,
    /// Any stairs block.
    Stairs,
    /// Any door block.
    Door,
    /// Fence gates: table boxes (in `float` world coordinates), but an identity for fences and
    /// walls.
    Gate,
    /// Anvil: shape by orientation once settled, full cube with constructor bounds.
    Anvil,
    /// Chest / trapped chest: reaches the block edge towards a chest of the same kind once
    /// settled, single-chest inset with constructor bounds.
    Chest,
}

/// Which bounds the history-dependent blocks (anvil, chest, trapped chest) collide with.
///
/// In 1.8.9 these blocks collide with whatever bounds the last shape update of *any* block of the
/// same type left behind (for example the crosshair ray trace); collision itself never recomputes
/// them. See `oracle/README.md`, "History-dependent shapes".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HistoryShapes {
    /// The shape the block's own update computes from its state and neighbours (anvil by facing,
    /// chest by double-chest connection). This is what Grim models and what a real client has had
    /// for nearly every block near it: the project default.
    #[default]
    Settled,
    /// The constructor bounds a fresh client keeps until a shape update (anvil full cube, chest
    /// single inset). This is what the oracle's movement harness sees.
    Constructor,
}

pub(crate) const fn shape_kind(block: u8) -> ShapeKind {
    match block {
        85 | 188..=192 => ShapeKind::WoodFence,
        id::NETHER_BRICK_FENCE => ShapeKind::NetherFence,
        101 | 102 | 160 => ShapeKind::Pane,
        id::COBBLESTONE_WALL => ShapeKind::Wall,
        53 | 67 | 108 | 109 | 114 | 128 | 134 | 135 | 136 | 156 | 163 | 164 | 180 => {
            ShapeKind::Stairs
        }
        64 | 71 | 193..=197 => ShapeKind::Door,
        107 | 183..=187 => ShapeKind::Gate,
        id::ANVIL => ShapeKind::Anvil,
        id::CHEST | id::TRAPPED_CHEST => ShapeKind::Chest,
        _ => ShapeKind::Table,
    }
}

/// `Block.isFullCube()`.
#[inline]
pub(crate) fn is_full_cube(s: BlockState) -> bool {
    s.block().full_cube()
}

/// `Material.blocksMovement()`.
#[inline]
pub(crate) fn blocks_movement(s: BlockState) -> bool {
    s.block().material().blocks_movement
}

/// `Material.isSolid()`.
#[inline]
pub(crate) fn is_solid(s: BlockState) -> bool {
    s.block().material().solid
}

/// `Material.isFluid()` (water and lava).
#[inline]
pub(crate) fn is_fluid(s: BlockState) -> bool {
    s.block().material().fluid
}

#[inline]
fn material_kind(s: BlockState) -> MaterialKind {
    s.block().material().kind
}

/// Water material (still and flowing water).
#[inline]
pub(crate) fn is_water(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Water
}

/// Lava material (still and flowing lava).
#[inline]
pub(crate) fn is_lava(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Lava
}

/// Air material.
#[inline]
pub(crate) fn is_air(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Air
}

/// Ice material (only the `ice` block; packed ice has its own material).
#[inline]
pub(crate) fn is_ice_material(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Ice
}

/// Fences, walls and fence gates: the blocks `Entity.move` looks through an air block for.
#[inline]
pub(crate) fn is_tall_support(s: BlockState) -> bool {
    matches!(
        shape_kind(s.block().id()),
        ShapeKind::WoodFence | ShapeKind::NetherFence | ShapeKind::Wall | ShapeKind::Gate
    )
}

/// Ground friction of the block (vanilla `float`).
#[inline]
pub(crate) fn slipperiness(s: BlockState) -> f32 {
    s.block().physics().slipperiness
}

/// `true` inside vanilla's valid block range (`World.isValidPos`).
#[inline]
pub(crate) fn is_valid_pos(x: i32, y: i32, z: i32) -> bool {
    (-30_000_000..30_000_000).contains(&x)
        && (-30_000_000..30_000_000).contains(&z)
        && (0..256).contains(&y)
}

/// `World.getBlockState`: air outside the valid range, otherwise the world's block.
#[inline]
pub(crate) fn state_at<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> BlockState {
    if is_valid_pos(x, y, z) {
        w.block(x, y, z)
    } else {
        BlockState::AIR
    }
}

/// Boxes kept inline before spilling to the heap (a walking tick needs about ten).
const INLINE_BOXES: usize = 24;

/// A list of collision boxes that lives on the stack until it outgrows [`INLINE_BOXES`].
pub(crate) struct BoxList {
    inline: [Aabb; INLINE_BOXES],
    len: usize,
    heap: Vec<Aabb>,
}

impl BoxList {
    pub(crate) fn new() -> BoxList {
        BoxList {
            inline: [Aabb::FULL; INLINE_BOXES],
            len: 0,
            heap: Vec::new(),
        }
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.heap.clear();
    }

    #[inline]
    pub(crate) fn push(&mut self, b: Aabb) {
        if self.len < INLINE_BOXES {
            self.inline[self.len] = b;
        } else {
            if self.len == INLINE_BOXES {
                self.heap.extend_from_slice(&self.inline);
            }
            self.heap.push(b);
        }
        self.len += 1;
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[Aabb] {
        if self.len <= INLINE_BOXES {
            &self.inline[..self.len]
        } else {
            &self.heap
        }
    }
}

/// Collects collision boxes intersecting `query`.
pub(crate) struct BoxSink<'a> {
    pub query: Aabb,
    pub out: &'a mut BoxList,
    pub history: HistoryShapes,
}

impl BoxSink<'_> {
    /// Adds a box in block-local `f64` coordinates, placed at the block position in `double`
    /// (vanilla `Block.getCollisionBox`: `pos.getX() + boundingBoxMinX`, ...).
    #[inline]
    fn local(&mut self, x: i32, y: i32, z: i32, b: [f64; 6]) {
        let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
        self.add(Aabb::new(
            fx + b[0],
            fy + b[1],
            fz + b[2],
            fx + b[3],
            fy + b[4],
            fz + b[5],
        ));
    }

    #[inline]
    fn add(&mut self, b: Aabb) {
        if self.query.intersects(&b) {
            self.out.push(b);
        }
    }
}

/// Adds the collision boxes of `s` at `(x, y, z)` that intersect the sink's query box.
/// Neighbours are read from `w` (vanilla reads them with `getBlockState`, no border override).
pub(crate) fn add_collision_boxes<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
    sink: &mut BoxSink<'_>,
) {
    match shape_kind(s.block().id()) {
        ShapeKind::Table | ShapeKind::Gate => {
            for b in s.collision().world_boxes(x, y, z) {
                sink.add(b);
            }
        }
        ShapeKind::Anvil => match sink.history {
            // The settled shape is the table's (by facing).
            HistoryShapes::Settled => {
                for b in s.collision().world_boxes(x, y, z) {
                    sink.add(b);
                }
            }
            HistoryShapes::Constructor if s.is_registered() => {
                sink.local(x, y, z, [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
            }
            HistoryShapes::Constructor => {}
        },
        ShapeKind::Chest => chest_box(w, x, y, z, s, sink),
        kind @ (ShapeKind::WoodFence | ShapeKind::NetherFence) => {
            fence_boxes(w, x, y, z, kind, sink);
        }
        ShapeKind::Pane => pane_boxes(w, x, y, z, sink),
        ShapeKind::Wall => wall_box(w, x, y, z, sink),
        ShapeKind::Stairs => stairs_boxes(w, x, y, z, s, sink),
        ShapeKind::Door => door_box(w, x, y, z, s, sink),
    }
}

/// Neighbour states north, south, west, east (`-Z`, `+Z`, `-X`, `+X`).
#[inline]
fn horizontal_neighbours<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> [BlockState; 4] {
    [
        state_at(w, x, y, z - 1),
        state_at(w, x, y, z + 1),
        state_at(w, x - 1, y, z),
        state_at(w, x + 1, y, z),
    ]
}

/// Fence / wall targets that are not fences: gates and opaque normal-render blocks, except
/// barrier and the pumpkin material (pumpkin, lit pumpkin, melon).
#[inline]
pub(crate) fn connects_as_solid(n: BlockState) -> bool {
    let id = n.block().id();
    if id == id::BARRIER || matches!(id, id::PUMPKIN | id::LIT_PUMPKIN | id::MELON_BLOCK) {
        return false;
    }
    let v = n.block().vanilla();
    shape_kind(id) == ShapeKind::Gate || v.material.opaque && v.render_as_normal_block
}

/// Fence connections north, south, west, east.
pub(crate) fn fence_links<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    kind: ShapeKind,
) -> [bool; 4] {
    horizontal_neighbours(w, x, y, z)
        .map(|nb| shape_kind(nb.block().id()) == kind || connects_as_solid(nb))
}

/// Glass pane / iron bars connections north, south, west, east.
pub(crate) fn pane_links<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> [bool; 4] {
    horizontal_neighbours(w, x, y, z).map(|nb| {
        let id = nb.block().id();
        nb.block().vanilla().full_block
            || shape_kind(id) == ShapeKind::Pane
            || id == id::GLASS
            || id == id::STAINED_GLASS
    })
}

/// Cobblestone wall connections north, south, west, east.
pub(crate) fn wall_links<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> [bool; 4] {
    horizontal_neighbours(w, x, y, z)
        .map(|nb| nb.block().id() == id::COBBLESTONE_WALL || connects_as_solid(nb))
}

fn fence_boxes<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    kind: ShapeKind,
    sink: &mut BoxSink<'_>,
) {
    let [n, s, we, e] = fence_links(w, x, y, z, kind);
    // Post plus arms, 1.5 high: one box along Z when connected north/south, one along X when
    // connected west/east or when standing alone on the Z axis.
    if n || s {
        let z0 = if n { 0.0 } else { 0.375 };
        let z1 = if s { 1.0 } else { 0.625 };
        sink.local(x, y, z, [0.375, 0.0, z0, 0.625, 1.5, z1]);
    }
    if we || e || !(n || s) {
        let x0 = if we { 0.0 } else { 0.375 };
        let x1 = if e { 1.0 } else { 0.625 };
        sink.local(x, y, z, [x0, 0.0, 0.375, x1, 1.5, 0.625]);
    }
}

fn pane_boxes<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    sink: &mut BoxSink<'_>,
) {
    let [n, s, we, e] = pane_links(w, x, y, z);
    let any = n || s || we || e;
    // The bar along X: both halves when connected on both sides or on no side at all, one half
    // towards a single X connection, nothing when only Z connections exist.
    let (x_lo, x_hi) = (0.4375, 0.5625);
    if we && e || !any {
        sink.local(x, y, z, [0.0, 0.0, x_lo, 1.0, 1.0, x_hi]);
    } else if we {
        sink.local(x, y, z, [0.0, 0.0, x_lo, 0.5, 1.0, x_hi]);
    } else if e {
        sink.local(x, y, z, [0.5, 0.0, x_lo, 1.0, 1.0, x_hi]);
    }
    // The same rule for the bar along Z.
    if n && s || !any {
        sink.local(x, y, z, [x_lo, 0.0, 0.0, x_hi, 1.0, 1.0]);
    } else if n {
        sink.local(x, y, z, [x_lo, 0.0, 0.0, x_hi, 1.0, 0.5]);
    } else if s {
        sink.local(x, y, z, [x_lo, 0.0, 0.5, x_hi, 1.0, 1.0]);
    }
}

fn wall_box<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    sink: &mut BoxSink<'_>,
) {
    let [n, s, we, e] = wall_links(w, x, y, z);
    let mut b = [
        if we { 0.0 } else { 0.25 },
        0.0,
        if n { 0.0 } else { 0.25 },
        if e { 1.0 } else { 0.75 },
        1.5,
        if s { 1.0 } else { 0.75 },
    ];
    // A straight run narrows to the thinner wall section (the post is omitted).
    if n && s && !we && !e {
        b[0] = 0.3125;
        b[3] = 0.6875;
    } else if we && e && !n && !s {
        b[2] = 0.3125;
        b[5] = 0.6875;
    }
    sink.local(x, y, z, b);
}

fn chest_box<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
    sink: &mut BoxSink<'_>,
) {
    if !s.is_registered() {
        return;
    }
    const IN: f64 = 0.0625;
    const OUT: f64 = 0.9375;
    let mut b = [IN, 0.0, IN, OUT, 0.875, OUT];
    if sink.history == HistoryShapes::Settled {
        // The first chest of the same kind found (north, south, west, east) extends the box to
        // that edge.
        let me = s.block();
        let [n, so, we, e] = horizontal_neighbours(w, x, y, z).map(|nb| nb.block() == me);
        if n {
            b[2] = 0.0;
        } else if so {
            b[5] = 1.0;
        } else if we {
            b[0] = 0.0;
        } else if e {
            b[3] = 1.0;
        }
    }
    sink.local(x, y, z, b);
}

/// Horizontal direction as a unit offset `(dx, dz)`.
type Dir = (i32, i32);

/// 1.8 stairs facing from metadata bits 0-1: east, west, south, north.
const STAIRS_FACING: [Dir; 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// The half (`[lo, hi)` in block units) of an axis on the side of a unit offset component.
#[inline]
fn half_toward(d: i32) -> (f64, f64) {
    if d > 0 { (0.5, 1.0) } else { (0.0, 0.5) }
}

fn stairs_boxes<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
    sink: &mut BoxSink<'_>,
) {
    let meta = s.meta();
    let top = meta & 4 != 0;
    let facing = STAIRS_FACING[(meta & 3) as usize];
    let (slab_y, step_y) = if top {
        ((0.5, 1.0), (0.0, 0.5))
    } else {
        ((0.0, 0.5), (0.5, 1.0))
    };
    sink.local(x, y, z, [0.0, slab_y.0, 0.0, 1.0, slab_y.1, 1.0]);

    // A stairs neighbour at `offset` with the same half; returns its facing.
    let stairs_at = |dx: i32, dz: i32| -> Option<Dir> {
        let n = state_at(w, x + dx, y, z + dz);
        (shape_kind(n.block().id()) == ShapeKind::Stairs && (n.meta() & 4 != 0) == top)
            .then(|| STAIRS_FACING[(n.meta() & 3) as usize])
    };
    // "Same stairs" (same half and facing as this block) at an offset.
    let same_at = |dx: i32, dz: i32| -> bool {
        let n = state_at(w, x + dx, y, z + dz);
        shape_kind(n.block().id()) == ShapeKind::Stairs && n.meta() & 7 == meta & 7
    };
    let perpendicular = |d: Dir| (d.0 == 0) != (facing.0 == 0);
    // A quarter box spanning the half of each axis towards `a` and `b` (one on X, one on Z).
    let quarter = |a: Dir, b: Dir| {
        let (xd, zd) = if a.0 != 0 { (a.0, b.1) } else { (b.0, a.1) };
        (half_toward(xd), half_toward(zd))
    };
    let mut put = |xs: (f64, f64), zs: (f64, f64), ys: (f64, f64)| {
        sink.local(x, y, z, [xs.0, ys.0, zs.0, xs.1, ys.1, zs.1]);
    };

    // The step: the half on the facing side, shrunk to a quarter at an outer corner (a
    // perpendicular stairs behind it, not continued on the opposite side).
    let behind = stairs_at(facing.0, facing.1);
    if let Some(g) = behind.filter(|&g| perpendicular(g) && !same_at(-g.0, -g.1)) {
        let (xs, zs) = quarter(facing, g);
        put(xs, zs, step_y);
        return;
    }
    let (xs, zs) = if facing.0 != 0 {
        (half_toward(facing.0), (0.0, 1.0))
    } else {
        ((0.0, 1.0), half_toward(facing.1))
    };
    put(xs, zs, step_y);

    // Inner corner: a perpendicular stairs in front adds the quarter on the front side towards
    // its facing, unless this stairs continues on that side.
    let front = stairs_at(-facing.0, -facing.1);
    if let Some(g) = front.filter(|&g| perpendicular(g) && !same_at(g.0, g.1)) {
        let (xs, zs) = quarter((-facing.0, -facing.1), g);
        put(xs, zs, step_y);
    }
}

/// 1.8 door direction from the lower half's metadata bits 0-1: east, south, west, north.
const DOOR_FACING: [Dir; 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

fn door_box<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
    sink: &mut BoxSink<'_>,
) {
    // Each half reads the other half's metadata whatever block is there: facing and open from
    // the lower half, hinge from the upper half.
    let meta = s.meta();
    let upper = meta & 8 != 0;
    let (lower_meta, upper_meta) = if upper {
        (state_at(w, x, y - 1, z).meta(), meta)
    } else {
        (meta, state_at(w, x, y + 1, z).meta())
    };
    let facing = DOOR_FACING[(lower_meta & 3) as usize];
    let open = lower_meta & 4 != 0;
    let hinge_left = upper_meta & 1 != 0;
    const T: f64 = 0.1875;
    // A closed door is a 3/16 slab on the side opposite its facing; an open one swings to the
    // side given by the hinge (90 degrees clockwise or counter-clockwise from the facing).
    let side: Dir = if !open {
        (-facing.0, -facing.1)
    } else if hinge_left {
        (-facing.1, facing.0)
    } else {
        (facing.1, -facing.0)
    };
    let slab = |d: i32| if d > 0 { (1.0 - T, 1.0) } else { (0.0, T) };
    let (xs, zs) = if side.0 != 0 {
        (slab(side.0), (0.0, 1.0))
    } else {
        ((0.0, 1.0), slab(side.1))
    };
    sink.local(x, y, z, [xs.0, 0.0, zs.0, xs.1, 1.0, zs.1]);
}

/// Test helper: every collision box of the block at `(x, y, z)`.
#[cfg(test)]
pub(crate) fn boxes_of<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    history: HistoryShapes,
) -> Vec<Aabb> {
    let mut out = BoxList::new();
    let query = crate::math::sorted_box(
        f64::from(x) - 2.0,
        f64::from(y) - 2.0,
        f64::from(z) - 2.0,
        f64::from(x) + 3.0,
        f64::from(y) + 3.0,
        f64::from(z) + 3.0,
    );
    let mut sink = BoxSink {
        query,
        out: &mut out,
        history,
    };
    add_collision_boxes(w, x, y, z, state_at(w, x, y, z), &mut sink);
    out.as_slice().to_vec()
}
