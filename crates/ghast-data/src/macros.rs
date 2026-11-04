//! Version-module boilerplate shared by every version (items, entities, biomes, ...).

/// Expands to the item type and the lookup functions over the generated tables of the version
/// module it is invoked in (which must alias its generated module as `g`).
macro_rules! common_api {
    () => {
        /// An item type of this version (id; 1.8 damage / metadata is carried separately in item stacks).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct Item(pub(crate) u16);

        impl Item {
            /// The item with this id, if it exists in this version.
            #[must_use]
            pub fn from_id(id: u16) -> Option<Item> {
                lookup_id(&g::items::ITEMS, id, |i| i.id).map(|_| Item(id))
            }

            /// Looks an item up by registry name.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Item> {
                lookup_name(&g::items::ITEM_NAMES, name).map(Item)
            }

            /// Item id.
            #[must_use]
            pub const fn id(self) -> u16 {
                self.0
            }

            /// Static information.
            #[must_use]
            pub fn info(self) -> &'static ItemInfo {
                // Constructed only from valid ids.
                lookup_id(&g::items::ITEMS, self.0, |i| i.id).unwrap_or(&g::items::ITEMS[0])
            }

            /// All items in id order.
            pub fn all() -> impl ExactSizeIterator<Item = Item> {
                g::items::ITEMS.iter().map(|i| Item(i.id))
            }
        }

        /// All entity types, sorted by (kind, id). In 1.8 mob ids (spawn-mob packet) and object ids
        /// (spawn-object packet) are separate id spaces.
        #[must_use]
        pub fn entities() -> &'static [EntityInfo] {
            &g::entities::ENTITIES
        }

        /// The entity type with network id `id` in the given id space.
        #[must_use]
        pub fn entity(kind: EntityKind, id: u16) -> Option<&'static EntityInfo> {
            lookup_id(&g::entities::ENTITIES, (kind as u8, id), |e| {
                (e.kind as u8, e.id)
            })
        }

        /// Entity type by minecraft-data name (1.8 uses legacy names such as `"PigZombie"`).
        #[must_use]
        pub fn entity_by_name(name: &str) -> Option<&'static EntityInfo> {
            lookup_name(&g::entities::ENTITY_NAMES, name)
                .map(|i| &g::entities::ENTITIES[i as usize])
        }

        /// Vanilla entity sizes measured by the oracle, sorted by name.
        #[must_use]
        pub fn vanilla_entity_sizes() -> &'static [crate::common::VanillaEntitySize] {
            &g::entities::VANILLA_ENTITY_SIZES
        }

        /// Vanilla entity sizes by name (1.8: legacy registry name such as `"Zombie"`; 1.16:
        /// registry path such as `"zombie"` or `"player"`).
        #[must_use]
        pub fn vanilla_entity_size(
            name: &str,
        ) -> Option<&'static crate::common::VanillaEntitySize> {
            g::entities::VANILLA_ENTITY_SIZES
                .binary_search_by(|e| e.name.cmp(name))
                .ok()
                .map(|i| &g::entities::VANILLA_ENTITY_SIZES[i])
        }

        /// All biomes by id.
        #[must_use]
        pub fn biomes() -> &'static [BiomeInfo] {
            &g::biomes::BIOMES
        }

        /// Biome by id.
        #[must_use]
        pub fn biome(id: u16) -> Option<&'static BiomeInfo> {
            lookup_id(&g::biomes::BIOMES, id, |b| b.id)
        }

        /// Biome by name.
        #[must_use]
        pub fn biome_by_name(name: &str) -> Option<&'static BiomeInfo> {
            lookup_name(&g::biomes::BIOME_NAMES, name).and_then(biome)
        }

        /// All enchantments by id.
        #[must_use]
        pub fn enchantments() -> &'static [EnchantmentInfo] {
            &g::enchantments::ENCHANTMENTS
        }

        /// Enchantment by id (1.8: the NBT `id` short).
        #[must_use]
        pub fn enchantment(id: u16) -> Option<&'static EnchantmentInfo> {
            lookup_id(&g::enchantments::ENCHANTMENTS, id, |e| e.id)
        }

        /// Enchantment by name.
        #[must_use]
        pub fn enchantment_by_name(name: &str) -> Option<&'static EnchantmentInfo> {
            lookup_name(&g::enchantments::ENCHANTMENT_NAMES, name).and_then(enchantment)
        }

        /// All foods, sorted by item id.
        #[must_use]
        pub fn foods() -> &'static [FoodInfo] {
            &g::foods::FOODS
        }

        /// Food data for an item id.
        #[must_use]
        pub fn food(item: u16) -> Option<&'static FoodInfo> {
            lookup_id(&g::foods::FOODS, item, |f| f.item)
        }

        /// All status effects by id.
        #[must_use]
        pub fn effects() -> &'static [EffectInfo] {
            &g::effects::EFFECTS
        }

        /// Status effect by id.
        #[must_use]
        pub fn effect(id: u8) -> Option<&'static EffectInfo> {
            lookup_id(&g::effects::EFFECTS, id, |e| e.id)
        }

        /// Status effect by name (`"Speed"`).
        #[must_use]
        pub fn effect_by_name(name: &str) -> Option<&'static EffectInfo> {
            lookup_name(&g::effects::EFFECT_NAMES, name).and_then(effect)
        }

        /// Tool speed multiplier of `tool` (item id) against `material`; `1.0` when the tool gives no
        /// bonus. Input for vanilla dig-time computation together with [`BlockInfo::hardness`] and
        /// [`BlockInfo::can_harvest_with`].
        #[must_use]
        pub fn tool_multiplier(material: DigMaterial, tool: u16) -> f32 {
            crate::common::tool_multiplier(&g::materials::TOOL_MULTIPLIERS, material, tool)
        }
    };
}

pub(crate) use common_api;
