//! 1.12.2 block facts the movement code needs: identities, materials and per-state flags (from
//! `ghast-data`'s oracle tables) and collision boxes. Static shapes come straight from the table
//! (1.12 places every box in `double`); the actual-state shapes are computed here: fences, panes
//! and walls connect by the neighbour's face shape, stairs take their corner shape from adjacent
//! stairs, doors combine both halves, chests reach towards a chest of the same kind, chorus plants
//! grow arms towards chorus blocks.

use ghast_data::v1_12::{BlockState, Face, FaceShape, MaterialKind};

use crate::math::{Aabb, BoxOps};
use crate::world::BlockView;

/// Block ids used by identity in 1.12 movement code.
pub(crate) mod id {
    pub const BED: u8 = 26;
    pub const WEB: u8 = 30;
    pub const LADDER: u8 = 65;
    pub const PUMPKIN: u8 = 86;
    pub const SOUL_SAND: u8 = 88;
    pub const LIT_PUMPKIN: u8 = 91;
    pub const TRAPDOOR: u8 = 96;
    pub const MELON_BLOCK: u8 = 103;
    pub const VINE: u8 = 106;
    pub const END_STONE: u8 = 121;
    pub const SLIME: u8 = 165;
    pub const BARRIER: u8 = 166;
    pub const IRON_TRAPDOOR: u8 = 167;
    pub const CHORUS_PLANT: u8 = 199;
    pub const CHORUS_FLOWER: u8 = 200;
}

/// How a block's collision boxes are produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShapeKind {
    /// The static table shape from `ghast-data`.
    Table,
    /// Fences (wooden and nether brick): post plus arms.
    Fence,
    /// Glass panes and iron bars.
    Pane,
    /// Cobblestone walls.
    Wall,
    /// Any stairs block.
    Stairs,
    /// Any door block.
    Door,
    /// Fence gates: table boxes, but an identity for fences, walls and landings.
    Gate,
    /// Chest / trapped chest.
    Chest,
    /// Chorus plant.
    ChorusPlant,
}

pub(crate) const fn shape_kind(block: u8) -> ShapeKind {
    match block {
        85 | 113 | 188..=192 => ShapeKind::Fence,
        101 | 102 | 160 => ShapeKind::Pane,
        139 => ShapeKind::Wall,
        53 | 67 | 108 | 109 | 114 | 128 | 134 | 135 | 136 | 156 | 163 | 164 | 180 | 203 => {
            ShapeKind::Stairs
        }
        64 | 71 | 193..=197 => ShapeKind::Door,
        107 | 183..=187 => ShapeKind::Gate,
        54 | 146 => ShapeKind::Chest,
        id::CHORUS_PLANT => ShapeKind::ChorusPlant,
        _ => ShapeKind::Table,
    }
}

/// `BlockState.isNormalCube()` (the push-out-of-blocks test).
#[inline]
pub(crate) fn is_normal_cube(s: BlockState) -> bool {
    s.is_normal_cube()
}

#[inline]
fn material_kind(s: BlockState) -> MaterialKind {
    s.block().material().kind
}

/// `Material.blocksMovement()`.
#[inline]
pub(crate) fn blocks_movement(s: BlockState) -> bool {
    s.block().material().blocks_movement
}

/// `Material.isLiquid()` (water and lava).
#[inline]
pub(crate) fn is_liquid(s: BlockState) -> bool {
    s.block().material().liquid
}

/// Water material.
#[inline]
pub(crate) fn is_water(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Water
}

/// Lava material.
#[inline]
pub(crate) fn is_lava(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Lava
}

/// Air material.
#[inline]
pub(crate) fn is_air(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Air
}

/// Ice material (only the `ice` block).
#[inline]
pub(crate) fn is_ice_material(s: BlockState) -> bool {
    material_kind(s) == MaterialKind::Ice
}

/// Fences, walls and fence gates: the blocks `Entity.move` looks through an air block for.
#[inline]
pub(crate) fn is_tall_support(s: BlockState) -> bool {
    matches!(
        shape_kind(s.block().id()),
        ShapeKind::Fence | ShapeKind::Wall | ShapeKind::Gate
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

/// Vanilla's base exclusion list for attaching (`Block.method_14309`: shulker boxes, leaves,
/// trapdoors, beacon, cauldron, glass, glowstone, ice, sea lantern, stained glass, pistons):
/// the fence exclusion list without barrier, pumpkins and melon.
#[inline]
pub(crate) fn attach_excluded(s: BlockState) -> bool {
    let b = s.block();
    b.fence_excluded()
        && !matches!(
            b.id(),
            id::BARRIER | id::PUMPKIN | id::LIT_PUMPKIN | id::MELON_BLOCK
        )
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
}

impl BoxSink<'_> {
    /// Adds a block-local box placed at the block position in `double` (vanilla
    /// `Box.offset(pos)`), if it intersects the query.
    #[inline]
    fn local(&mut self, x: i32, y: i32, z: i32, b: [f64; 6]) {
        let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
        let w = Aabb::new(
            b[0] + fx,
            b[1] + fy,
            b[2] + fz,
            b[3] + fx,
            b[4] + fy,
            b[5] + fz,
        );
        if self.query.intersects(&w) {
            self.out.push(w);
        }
    }

    #[inline]
    fn table(&mut self, x: i32, y: i32, z: i32, b: &Aabb) {
        self.local(
            x,
            y,
            z,
            [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z],
        );
    }
}

/// Adds the collision boxes of `s` at `(x, y, z)` that intersect the sink's query box.
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
            for b in s.shape() {
                sink.table(x, y, z, b);
            }
        }
        ShapeKind::Fence => fence_boxes(w, x, y, z, s, sink),
        ShapeKind::Pane => pane_boxes(w, x, y, z, sink),
        ShapeKind::Wall => wall_box(w, x, y, z, sink),
        ShapeKind::Stairs => stairs_boxes(w, x, y, z, s, sink),
        ShapeKind::Door => door_box(w, x, y, z, s, sink),
        ShapeKind::Chest => chest_box(w, x, y, z, s, sink),
        ShapeKind::ChorusPlant => chorus_boxes(w, x, y, z, sink),
    }
}

/// The face shape of the block at `(x, y, z)` on `face`, as vanilla's `getBlockFaceShape` sees it
/// (stairs from their actual corner shape, every other block from the table).
pub(crate) fn face_shape<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
    face: Face,
) -> FaceShape {
    if shape_kind(s.block().id()) != ShapeKind::Stairs {
        return s.face_shape(face);
    }
    let st = stairs_state(w, x, y, z, s);
    let d = match face {
        Face::Down => {
            return if st.top {
                FaceShape::Undefined
            } else {
                FaceShape::Solid
            };
        }
        Face::Up => {
            return if st.top {
                FaceShape::Solid
            } else {
                FaceShape::Undefined
            };
        }
        Face::North => (0, -1),
        Face::South => (0, 1),
        Face::West => (-1, 0),
        Face::East => (1, 0),
    };
    let solid = match st.shape {
        StairsShape::Straight => st.facing == d,
        StairsShape::InnerRight => st.facing == d || st.facing == ccw(d),
        StairsShape::InnerLeft => st.facing == d || st.facing == cw(d),
        StairsShape::OuterLeft | StairsShape::OuterRight => false,
    };
    if solid {
        FaceShape::Solid
    } else {
        FaceShape::Undefined
    }
}

/// The neighbours north, east, south, west with the face each one shows towards this block.
const SIDES: [(i32, i32, Face); 4] = [
    (0, -1, Face::South),
    (1, 0, Face::West),
    (0, 1, Face::North),
    (-1, 0, Face::East),
];

/// Fence connections north, east, south, west: a solid face (of a block not on the fence
/// exclusion list), or a fence post of the same material, or a fence gate.
pub(crate) fn fence_links<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    me: BlockState,
) -> [bool; 4] {
    let material = me.block().material();
    SIDES.map(|(dx, dz, face)| {
        let (nx, nz) = (x + dx, z + dz);
        let n = state_at(w, nx, y, nz);
        let shape = face_shape(w, nx, y, nz, n, face);
        let pole = shape == FaceShape::MiddlePole
            && (n.block().material() == material || shape_kind(n.block().id()) == ShapeKind::Gate);
        !n.block().fence_excluded() && shape == FaceShape::Solid || pole
    })
}

/// Pane / iron bars connections north, east, south, west.
pub(crate) fn pane_links<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> [bool; 4] {
    SIDES.map(|(dx, dz, face)| {
        let (nx, nz) = (x + dx, z + dz);
        let n = state_at(w, nx, y, nz);
        let shape = face_shape(w, nx, y, nz, n, face);
        !n.block().pane_excluded() && shape == FaceShape::Solid
            || shape == FaceShape::MiddlePoleThin
    })
}

/// Wall connections north, east, south, west.
pub(crate) fn wall_links<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> [bool; 4] {
    SIDES.map(|(dx, dz, face)| {
        let (nx, nz) = (x + dx, z + dz);
        let n = state_at(w, nx, y, nz);
        let shape = face_shape(w, nx, y, nz, n, face);
        let pole = shape == FaceShape::MiddlePoleThick
            || shape == FaceShape::MiddlePole && shape_kind(n.block().id()) == ShapeKind::Gate;
        !n.block().wall_excluded() && shape == FaceShape::Solid || pole
    })
}

fn fence_boxes<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
    sink: &mut BoxSink<'_>,
) {
    let [n, e, so, we] = fence_links(w, x, y, z, s);
    // A 1.5 high post, plus one box per connected arm.
    sink.local(x, y, z, [0.375, 0.0, 0.375, 0.625, 1.5, 0.625]);
    if n {
        sink.local(x, y, z, [0.375, 0.0, 0.0, 0.625, 1.5, 0.375]);
    }
    if e {
        sink.local(x, y, z, [0.625, 0.0, 0.375, 1.0, 1.5, 0.625]);
    }
    if so {
        sink.local(x, y, z, [0.375, 0.0, 0.625, 0.625, 1.5, 1.0]);
    }
    if we {
        sink.local(x, y, z, [0.0, 0.0, 0.375, 0.375, 1.5, 0.625]);
    }
}

fn pane_boxes<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    sink: &mut BoxSink<'_>,
) {
    let [n, e, s, we] = pane_links(w, x, y, z);
    // A 2/16 post, plus an arm from the post's far edge to the block edge per connection.
    const LO: f64 = 0.4375;
    const HI: f64 = 0.5625;
    sink.local(x, y, z, [LO, 0.0, LO, HI, 1.0, HI]);
    if n {
        sink.local(x, y, z, [LO, 0.0, 0.0, HI, 1.0, HI]);
    }
    if s {
        sink.local(x, y, z, [LO, 0.0, LO, HI, 1.0, 1.0]);
    }
    if e {
        sink.local(x, y, z, [LO, 0.0, LO, 1.0, 1.0, HI]);
    }
    if we {
        sink.local(x, y, z, [0.0, 0.0, LO, HI, 1.0, HI]);
    }
}

fn wall_box<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    sink: &mut BoxSink<'_>,
) {
    let [n, e, s, we] = wall_links(w, x, y, z);
    // One 1.5 high box spanning the post and the connected sides; a straight run is the
    // thinner wall section without the post.
    let mut b = [
        if we { 0.0 } else { 0.25 },
        0.0,
        if n { 0.0 } else { 0.25 },
        if e { 1.0 } else { 0.75 },
        1.5,
        if s { 1.0 } else { 0.75 },
    ];
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
    const IN: f64 = 0.0625;
    const OUT: f64 = 0.9375;
    let mut b = [IN, 0.0, IN, OUT, 0.875, OUT];
    // The first chest of the same kind found (north, south, west, east) extends the box to that
    // edge.
    let me = s.block();
    let same = |dx: i32, dz: i32| state_at(w, x + dx, y, z + dz).block() == me;
    if same(0, -1) {
        b[2] = 0.0;
    } else if same(0, 1) {
        b[5] = 1.0;
    } else if same(-1, 0) {
        b[0] = 0.0;
    } else if same(1, 0) {
        b[3] = 1.0;
    }
    sink.local(x, y, z, b);
}

fn chorus_boxes<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    sink: &mut BoxSink<'_>,
) {
    let chorus = |dx: i32, dy: i32, dz: i32| {
        let id = state_at(w, x + dx, y + dy, z + dz).block().id();
        id == id::CHORUS_PLANT || id == id::CHORUS_FLOWER || (dy == -1 && id == id::END_STONE)
    };
    const A: f64 = 0.1875;
    const B: f64 = 0.8125;
    sink.local(x, y, z, [A, A, A, B, B, B]);
    if chorus(-1, 0, 0) {
        sink.local(x, y, z, [0.0, A, A, A, B, B]);
    }
    if chorus(1, 0, 0) {
        sink.local(x, y, z, [B, A, A, 1.0, B, B]);
    }
    if chorus(0, 1, 0) {
        sink.local(x, y, z, [A, B, A, B, 1.0, B]);
    }
    if chorus(0, -1, 0) {
        sink.local(x, y, z, [A, 0.0, A, B, A, B]);
    }
    if chorus(0, 0, -1) {
        sink.local(x, y, z, [A, A, 0.0, B, B, A]);
    }
    if chorus(0, 0, 1) {
        sink.local(x, y, z, [A, A, B, B, B, 1.0]);
    }
}

/// Horizontal direction as a unit offset `(dx, dz)`.
type Dir = (i32, i32);

/// 90 degrees clockwise seen from above (north -> east).
#[inline]
const fn cw(d: Dir) -> Dir {
    (-d.1, d.0)
}

/// 90 degrees counter-clockwise seen from above (north -> west).
#[inline]
const fn ccw(d: Dir) -> Dir {
    (d.1, -d.0)
}

/// Stairs facing from metadata bits 0-1: east, west, south, north.
const STAIRS_FACING: [Dir; 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StairsShape {
    Straight,
    InnerLeft,
    InnerRight,
    OuterLeft,
    OuterRight,
}

struct Stairs {
    facing: Dir,
    top: bool,
    shape: StairsShape,
}

/// The actual stairs shape: an outer corner when a perpendicular stairs (same half) is behind and
/// this stairs does not continue on its far side, else an inner corner for one in front.
fn stairs_state<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
) -> Stairs {
    let meta = s.meta();
    let top = meta & 4 != 0;
    let facing = STAIRS_FACING[(meta & 3) as usize];
    let stairs_at = |d: Dir| -> Option<(Dir, bool)> {
        let n = state_at(w, x + d.0, y, z + d.1);
        (shape_kind(n.block().id()) == ShapeKind::Stairs)
            .then(|| (STAIRS_FACING[(n.meta() & 3) as usize], n.meta() & 4 != 0))
    };
    // A different stairs (or no stairs) at an offset: not the same facing and half.
    let different = |d: Dir| stairs_at(d) != Some((facing, top));
    let perpendicular = |d: Dir| (d.0 == 0) != (facing.0 == 0);
    let mut shape = StairsShape::Straight;
    if let Some((g, h)) = stairs_at(facing)
        && h == top
        && perpendicular(g)
        && different((-g.0, -g.1))
    {
        shape = if g == ccw(facing) {
            StairsShape::OuterLeft
        } else {
            StairsShape::OuterRight
        };
    } else if let Some((g, h)) = stairs_at((-facing.0, -facing.1))
        && h == top
        && perpendicular(g)
        && different(g)
    {
        shape = if g == ccw(facing) {
            StairsShape::InnerLeft
        } else {
            StairsShape::InnerRight
        };
    }
    Stairs { facing, top, shape }
}

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
    let st = stairs_state(w, x, y, z, s);
    let (slab_y, step_y) = if st.top {
        ((0.5, 1.0), (0.0, 0.5))
    } else {
        ((0.0, 0.5), (0.5, 1.0))
    };
    sink.local(x, y, z, [0.0, slab_y.0, 0.0, 1.0, slab_y.1, 1.0]);
    let mut put = |xs: (f64, f64), zs: (f64, f64)| {
        sink.local(x, y, z, [xs.0, step_y.0, zs.0, xs.1, step_y.1, zs.1]);
    };
    let f = st.facing;
    // The half step on the facing side (straight and inner corners).
    if matches!(
        st.shape,
        StairsShape::Straight | StairsShape::InnerLeft | StairsShape::InnerRight
    ) {
        if f.0 != 0 {
            put(half_toward(f.0), (0.0, 1.0));
        } else {
            put((0.0, 1.0), half_toward(f.1));
        }
    }
    // The quarter step of a corner: the quarter between `d` and its counter-clockwise neighbour.
    let d = match st.shape {
        StairsShape::Straight => return,
        StairsShape::OuterLeft => f,
        StairsShape::OuterRight => cw(f),
        StairsShape::InnerRight => (-f.0, -f.1),
        StairsShape::InnerLeft => ccw(f),
    };
    let e = ccw(d);
    let (xd, zd) = if d.0 != 0 { (d.0, e.1) } else { (e.0, d.1) };
    put(half_toward(xd), half_toward(zd));
}

/// Door direction from the lower half's metadata bits 0-1: east, south, west, north.
const DOOR_FACING: [Dir; 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// A door state's properties as the palette decodes them: the half's own metadata, and for the
/// properties it does not carry the last state registered for that raw value (the last value of
/// each property: facing east, closed, right hinge).
struct DoorProps {
    facing: Dir,
    open: bool,
    right_hinge: bool,
}

fn door_props(s: BlockState) -> DoorProps {
    let meta = s.meta();
    if meta & 8 != 0 {
        DoorProps {
            facing: (1, 0),
            open: false,
            right_hinge: meta & 1 != 0,
        }
    } else {
        DoorProps {
            facing: DOOR_FACING[(meta & 3) as usize],
            open: meta & 4 != 0,
            right_hinge: true,
        }
    }
}

fn door_box<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
    sink: &mut BoxSink<'_>,
) {
    // The actual state: a lower half takes the hinge from the block above, an upper half the
    // facing and open flag from the block below, when that block is the same door (whatever
    // half it is).
    let upper = s.meta() & 8 != 0;
    let mut props = door_props(s);
    let other = state_at(w, x, if upper { y - 1 } else { y + 1 }, z);
    if other.block() == s.block() {
        let o = door_props(other);
        if upper {
            props.facing = o.facing;
            props.open = o.open;
        } else {
            props.right_hinge = o.right_hinge;
        }
    }
    let DoorProps {
        facing,
        open,
        right_hinge,
    } = props;
    const T: f64 = 0.1875;
    // A closed door is a 3/16 slab on the side opposite its facing; an open one swings to the
    // side given by the hinge.
    let side: Dir = if !open {
        (-facing.0, -facing.1)
    } else if right_hinge {
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

/// Every collision box of the block at `(x, y, z)` (tests and tools).
pub(crate) fn boxes_of<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
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
    };
    add_collision_boxes(w, x, y, z, state_at(w, x, y, z), &mut sink);
    out.as_slice().to_vec()
}

/// The ladder facing a ladder state points to (`(dx, dz)`), as vanilla decodes the metadata.
#[inline]
pub(crate) fn ladder_facing(meta: u8) -> Dir {
    match meta {
        3 => (0, 1),
        4 => (-1, 0),
        5 => (1, 0),
        _ => (0, -1),
    }
}

/// The trapdoor facing of a trapdoor state (`(dx, dz)`).
#[inline]
pub(crate) fn trapdoor_facing(meta: u8) -> Dir {
    match meta & 3 {
        0 => (0, -1),
        1 => (0, 1),
        2 => (-1, 0),
        _ => (1, 0),
    }
}
