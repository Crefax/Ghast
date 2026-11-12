// Shared block API of the modern versions: included by `v26_1/mod.rs` and `v26_2/mod.rs`,
// whose `g` (generated tables), `BlockData`, `block_name` and `block_display_name` it uses.
/// Number of blocks (ids `0..BLOCK_COUNT`).
pub const BLOCK_COUNT: usize = g::blocks::BLOCKS.len();
/// Number of block states (ids `0..STATE_COUNT`).
pub const STATE_COUNT: usize = g::blocks::STATE_BLOCK.len();

impl Property {
    /// Looks a property up by name (`"facing"`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Property> {
        crate::common::lookup_name(
            g::blocks::PROPERTY_NAMES_TEXT,
            &g::blocks::PROPERTY_NAMES,
            name,
        )
    }
}

/// One property of a block with its possible values, in palette order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyDef {
    /// Which property.
    pub property: Property,
    /// Values in palette order (booleans are `["true", "false"]`, integers `"0"`, `"1"`, ...).
    pub values: &'static [&'static str],
}

/// The state-id layout of one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateLayout {
    /// First state id.
    pub min_state: u16,
    /// Number of states.
    pub state_count: u16,
    /// Default state id.
    pub default_state: u16,
    /// Indices into the property definition table, most significant first: `(start, len)` in
    /// the generated `LAYOUT_PROPERTIES` (plain data, no pointer).
    pub(crate) properties: (u16, u8),
}

impl StateLayout {
    /// Indices into the property definition table, most significant first.
    #[inline]
    pub(crate) fn property_ids(&self) -> &'static [u8] {
        let (start, len) = (
            usize::from(self.properties.0),
            usize::from(self.properties.1),
        );
        g::blocks::LAYOUT_PROPERTIES
            .get(start..start + len)
            .unwrap_or(&[])
    }
}


/// The collision voxel shape table (index 0 = empty; shared by the modern versions, so it
/// also holds the shapes of later versions).
#[must_use]
pub fn voxel_shapes() -> &'static [VoxelShapeRef] {
    &crate::generated::modern::VOXEL_SHAPES
}

/// Index of `Shapes.block()` in [`voxel_shapes`].
pub const FULL_CUBE_VOXEL: u16 = g::blocks::FULL_CUBE_VOXEL;
/// Index of scaffolding's bottom shape in [`voxel_shapes`].
pub const SCAFFOLDING_BOTTOM_VOXEL: u16 = g::blocks::SCAFFOLDING_BOTTOM_VOXEL;
/// Index of powder snow's falling shape (`box(0, 0, 0, 1, 0.9F, 1)`) in [`voxel_shapes`].
pub const POWDER_SNOW_FALLING_VOXEL: u16 = g::blocks::POWDER_SNOW_FALLING_VOXEL;


/// A 26.1 block type (registry id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Block(pub(crate) u16);

impl Block {
    /// The block with this registry id.
    #[must_use]
    pub const fn from_id(id: u16) -> Option<Block> {
        if (id as usize) < BLOCK_COUNT {
            Some(Block(id))
        } else {
            None
        }
    }

    /// Looks a block up by registry name (`"oak_stairs"`, no namespace).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Block> {
        lookup_name(g::blocks::BLOCK_NAMES_TEXT, &g::blocks::BLOCK_NAMES, name).map(Block)
    }

    /// Registry id.
    #[must_use]
    pub const fn id(self) -> u16 {
        self.0
    }

    /// Static data.
    #[inline]
    #[must_use]
    pub fn data(self) -> &'static BlockData {
        &g::blocks::BLOCKS[self.0 as usize]
    }

    /// Registry name.
    #[must_use]
    pub fn name(self) -> &'static str {
        block_name(self.0)
    }

    /// English display name.
    #[cfg(feature = "display-names")]
    #[must_use]
    pub fn display_name(self) -> &'static str {
        block_display_name(self.0)
    }

    /// `true` for `air`, `cave_air` and `void_air` (vanilla `BlockState.isAir`).
    #[must_use]
    pub const fn is_air(self) -> bool {
        matches!(self, Block::AIR | Block::CAVE_AIR | Block::VOID_AIR)
    }

    /// State-id layout.
    #[must_use]
    pub fn layout(self) -> &'static StateLayout {
        &g::blocks::LAYOUTS[self.0 as usize]
    }

    /// The default state.
    #[must_use]
    pub fn default_state(self) -> BlockState {
        BlockState(self.layout().default_state)
    }

    /// All states of this block in id order.
    pub fn states(self) -> impl ExactSizeIterator<Item = BlockState> {
        let l = self.layout();
        (l.min_state..l.min_state + l.state_count).map(BlockState)
    }

    /// The block's properties, most significant first.
    pub fn properties(self) -> impl ExactSizeIterator<Item = &'static PropertyDef> {
        self.layout()
            .property_ids()
            .iter()
            .map(|&i| &g::blocks::PROPERTY_DEFS[i as usize])
    }

    /// The default state with the given properties changed; `None` if a property or value does not
    /// exist for this block.
    #[must_use]
    pub fn state_with(self, props: &[(Property, &str)]) -> Option<BlockState> {
        props
            .iter()
            .try_fold(self.default_state(), |s, &(p, v)| s.with(p, v))
    }

    /// All blocks in id order.
    pub fn all() -> impl ExactSizeIterator<Item = Block> {
        (0..BLOCK_COUNT as u16).map(Block)
    }
}

/// A 26.1 block state (global palette id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockState(u16);

impl BlockState {
    /// `minecraft:air` (id 0).
    pub const AIR: BlockState = BlockState(0);

    /// Wraps a palette id; `None` if out of range.
    #[must_use]
    pub const fn from_id(id: u16) -> Option<BlockState> {
        if (id as usize) < STATE_COUNT {
            Some(BlockState(id))
        } else {
            None
        }
    }

    /// Palette id.
    #[inline]
    #[must_use]
    pub const fn id(self) -> u16 {
        self.0
    }

    /// The block type.
    #[inline]
    #[must_use]
    pub fn block(self) -> Block {
        Block(g::blocks::STATE_BLOCK[self.0 as usize])
    }

    /// `true` for `air`, `cave_air` and `void_air`.
    #[must_use]
    pub fn is_air(self) -> bool {
        self.phys().has(state_flags::AIR)
    }

    /// The packed movement record (one table read).
    #[inline]
    #[must_use]
    pub fn phys(self) -> StatePhys {
        StatePhys(g::blocks::STATE_PHYS[self.0 as usize])
    }

    /// The context-free collision voxel shape (unshifted for offset blocks).
    #[must_use]
    pub fn voxel_shape(self) -> &'static VoxelShapeRef {
        &crate::generated::modern::VOXEL_SHAPES[self.phys().shape() as usize]
    }

    /// The crosshair outline shape (`getShape` / `getOutlineShape` with the player's context,
    /// empty hand). For states with [`BlockState::outline_offset`] this is the unshifted shape: the
    /// block moves it by its per-position model offset.
    #[must_use]
    pub fn outline_shape(self) -> &'static SelectionShape {
        &crate::generated::modern::SELECTION_SHAPES[g::selection::STATE_OUTLINE[self.0 as usize] as usize]
    }

    /// The outline while the player holds this block's own item (scaffolding and, where it
    /// exists, the light block show a full cube then); otherwise [`BlockState::outline_shape`].
    #[must_use]
    pub fn outline_shape_holding_own_item(self) -> &'static SelectionShape {
        let t = &g::selection::OUTLINE_HOLDING_OWN_ITEM;
        match t.binary_search_by_key(&self.0, |e| e.0) {
            Ok(i) => &crate::generated::modern::SELECTION_SHAPES[t[i].1 as usize],
            Err(_) => self.outline_shape(),
        }
    }

    /// `true` when the outline is moved by the block's model offset (flowers, bamboo, ...).
    #[must_use]
    pub fn outline_offset(self) -> bool {
        g::selection::STATE_OUTLINE_OFFSET[self.0 as usize / 64] & (1 << (self.0 % 64)) != 0
    }

    /// The interaction shape (`getInteractionShape`): when the ray hits it nearer than the outline,
    /// the hit keeps the outline's position but takes this shape's face (cauldrons, composters,
    /// hoppers, scaffolding).
    #[must_use]
    pub fn interaction_shape(self) -> &'static SelectionShape {
        &crate::generated::modern::SELECTION_SHAPES[g::selection::STATE_INTERACTION[self.0 as usize] as usize]
    }

    /// Index of `prop`'s value in [`PropertyDef::values`], if the block has that property.
    #[must_use]
    pub fn get_index(self, prop: Property) -> Option<usize> {
        let layout = self.block().layout();
        let mut rem = usize::from(self.0 - layout.min_state);
        for &d in layout.property_ids().iter().rev() {
            let def = &g::blocks::PROPERTY_DEFS[d as usize];
            let n = def.values.len();
            if def.property == prop {
                return Some(rem % n);
            }
            rem /= n;
        }
        None
    }

    /// The value of `prop` (`"north"`, `"true"`, `"3"`), if the block has that property.
    #[must_use]
    pub fn get(self, prop: Property) -> Option<&'static str> {
        let def = self.block().properties().find(|d| d.property == prop)?;
        self.get_index(prop).map(|i| def.values[i])
    }

    /// This state with `prop` set to `value`; `None` if the block lacks the property or value.
    #[must_use]
    pub fn with(self, prop: Property, value: &str) -> Option<BlockState> {
        let layout = self.block().layout();
        let mut stride = 1usize;
        for &d in layout.property_ids().iter().rev() {
            let def = &g::blocks::PROPERTY_DEFS[d as usize];
            let n = def.values.len();
            if def.property == prop {
                let new = def.values.iter().position(|v| *v == value)?;
                let base = usize::from(self.0 - layout.min_state);
                let old = base / stride % n;
                let id = base - old * stride + new * stride + usize::from(layout.min_state);
                return Some(BlockState(id as u16));
            }
            stride *= n;
        }
        None
    }

    /// `(property, value)` pairs of this state, most significant property first.
    pub fn properties(self) -> impl Iterator<Item = (Property, &'static str)> {
        let block = self.block();
        block
            .properties()
            .filter_map(move |d| self.get(d.property).map(|v| (d.property, v)))
    }

    /// Parses `minecraft:name[prop=value,...]` (the namespace is optional).
    #[must_use]
    pub fn parse(s: &str) -> Option<BlockState> {
        let s = s.strip_prefix("minecraft:").unwrap_or(s);
        let (name, props) = match s.split_once('[') {
            Some((n, rest)) => (n, rest.strip_suffix(']')?),
            None => (s, ""),
        };
        let mut st = Block::from_name(name)?.default_state();
        for kv in props.split(',').filter(|p| !p.is_empty()) {
            let (k, v) = kv.split_once('=')?;
            st = st.with(Property::from_name(k)?, v)?;
        }
        Some(st)
    }
}

/// `minecraft:oak_stairs[facing=north,half=bottom,...]`, the vanilla block-state string form.
impl fmt::Display for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "minecraft:{}", self.block().name())?;
        let mut first = true;
        for (p, v) in self.properties() {
            f.write_str(if first { "[" } else { "," })?;
            write!(f, "{}={v}", p.name())?;
            first = false;
        }
        if !first {
            f.write_str("]")?;
        }
        Ok(())
    }
}
