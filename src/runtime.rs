// Theurgy's calcination process, as pure data and arithmetic.
//
// This module is both the standalone crate library and the game-side module
// (`crate::game::theurgy`), so it holds no game type: the jar's calcination
// recipes, the oven's progress rule and the brazier's burn rule. A tag
// ingredient asks its caller whether an item is a member, which the glue
// answers from the engine's merged item tags (`native_server_item_in_tag`).
//
// Read from Theurgy 1.78.0 (`CalcinationRecipe`, `CraftingBehaviour.tickServer`,
// `HeatConsumerBehaviour`, `PyromanticBrazierBlockEntity.tickServer`) and from
// the jar's `data/theurgy/recipe/calcination/*.json`. See `../../FINDINGS.md`.

/// `CalcinationRecipe.DEFAULT_TIME`: the time a recipe that names none takes.
pub const DEFAULT_TIME: u16 = 100;
/// `HeatConsumerBehaviour.CHECK_HEAT_TICK_INTERVAL`: the heat below is sampled
/// once every this many game ticks and cached.
pub const HEAT_CHECK_INTERVAL: u32 = 20;
/// One `ItemStackHandler(1)` slot's limit, which caps what the output may hold.
pub const SLOT_LIMIT: u32 = 64;

/// Whether `item` is a member of `tag`, as the engine's merged item tags answer it.
pub type InTag<'a> = &'a dyn Fn(&str, &str) -> bool;

/// A recipe's input: one item, or a tag by its id (the jar names tags such as
/// `#c:gems`, `#c:ores` and `#minecraft:logs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ingredient {
    Item(&'static str),
    Tag(&'static str),
}

impl Ingredient {
    /// Whether `item` is this ingredient (NeoForge's `Ingredient.test`).
    #[must_use]
    pub fn accepts(&self, item: &str, in_tag: InTag<'_>) -> bool {
        match self {
            Ingredient::Item(id) => *id == item,
            Ingredient::Tag(tag) => in_tag(item, tag),
        }
    }
}

/// One `theurgy:calcination` recipe: a sized ingredient and its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub input: Ingredient,
    pub count: u32,
    pub output: &'static str,
    pub output_count: u32,
    pub time: u16,
}

/// The first recipe the item matches, as `CalcinationCachedCheck` picks one.
#[must_use]
pub fn recipe_for(item: &str, in_tag: InTag<'_>) -> Option<&'static Recipe> {
    RECIPES
        .iter()
        .find(|recipe| recipe.input.accepts(item, in_tag))
}

/// Whether the whole result fits the output slot (`ItemHandlerHelper.insertItemStacked`).
#[must_use]
pub fn output_fits(recipe: &Recipe, output: Option<(&str, u32)>) -> bool {
    match output {
        None => recipe.output_count <= SLOT_LIMIT,
        Some((item, count)) => {
            item == recipe.output && count.saturating_add(recipe.output_count) <= SLOT_LIMIT
        }
    }
}

/// What one server tick of the oven did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// No progress this tick (cold, no input, no recipe, or the output is full).
    Stopped,
    /// Progress advanced but the recipe has not finished.
    Working,
    /// The recipe finished: consume `recipe.count` of the input and insert
    /// `recipe.output_count` of `recipe.output`.
    Craft(&'static Recipe),
}

/// The oven's persisted counters, as `CraftingBehaviour` saves them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Oven {
    pub progress: u16,
    pub total: u16,
}

impl Oven {
    /// A fresh oven: no progress, the default crafting time.
    #[must_use]
    pub fn new() -> Self {
        Oven {
            progress: 0,
            total: DEFAULT_TIME,
        }
    }

    /// The input slot's item changed (`onInputItemChanged`): the recipe's time
    /// becomes the total, or the default when the item has no recipe, and the
    /// progress starts over.
    pub fn input_changed(&mut self, item: &str, in_tag: InTag<'_>) {
        self.total = recipe_for(item, in_tag).map_or(DEFAULT_TIME, |recipe| recipe.time);
        self.progress = 0;
    }

    /// One server tick (`CraftingBehaviour.tickServer(heated, has_input)`).
    ///
    /// `input` is the input slot's stack and `output` the output slot's, or
    /// nothing for an empty slot. Progress only advances while the oven is
    /// heated, the item matches a recipe, its stack holds the recipe's count
    /// and the whole result fits the output; every other tick resets the
    /// progress to 0, as upstream's `stopProcessing` does.
    pub fn tick(
        &mut self,
        heated: bool,
        input: Option<(&str, u32)>,
        output: Option<(&str, u32)>,
        in_tag: InTag<'_>,
    ) -> Step {
        let Some((item, held)) = input else {
            return self.stop();
        };
        let Some(recipe) = recipe_for(item, in_tag) else {
            return self.stop();
        };
        let craftable = held >= recipe.count && output_fits(recipe, output);
        if !heated || !craftable {
            return self.stop();
        }
        self.progress = self.progress.saturating_add(1);
        if self.progress < self.total {
            return Step::Working;
        }
        self.progress = 0;
        self.total = recipe.time;
        Step::Craft(recipe)
    }

    fn stop(&mut self) -> Step {
        self.progress = 0;
        Step::Stopped
    }
}

/// The brazier's burn (`PyromanticBrazierBlockEntity.tickServer`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Brazier {
    /// Ticks of heat left, saved as a short upstream.
    pub remaining: u16,
}

/// What one brazier tick did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Burn {
    /// Whether the brazier is lit after the tick (its `lit` state).
    pub lit: bool,
    /// Whether this tick consumed the fuel item in slot 0.
    pub consumed: bool,
    /// Whether the `lit` state changed and must be written.
    pub changed: bool,
}

impl Brazier {
    /// An empty, cold brazier.
    #[must_use]
    pub fn new() -> Self {
        Brazier { remaining: 0 }
    }

    /// Whether the brazier is lit (upstream's `isLit`).
    #[must_use]
    pub fn lit(&self) -> bool {
        self.remaining > 0
    }

    /// One server tick. `fuel_burn` is the burn time of the item in slot 0
    /// (`None` for an empty or non-fuel slot), in ticks.
    ///
    /// The brazier burns down first and only refuels once it has gone out, so a
    /// fuel item of `n` ticks heats for exactly `n` ticks.
    pub fn tick(&mut self, fuel_burn: Option<u32>) -> Burn {
        let was_lit = self.lit();
        if was_lit {
            self.remaining -= 1;
        }
        let mut consumed = false;
        if !self.lit()
            && let Some(burn) = fuel_burn
            && burn > 0
        {
            self.remaining = u16::try_from(burn).unwrap_or(u16::MAX);
            consumed = true;
        }
        Burn {
            lit: self.lit(),
            consumed,
            changed: self.lit() != was_lit,
        }
    }
}

/// Whether the oven's heat is sampled on this game tick (`gameTime % 20 == 0`).
#[must_use]
pub fn heat_sample_due(game_time: u64) -> bool {
    game_time.is_multiple_of(u64::from(HEAT_CHECK_INTERVAL))
}

// BEGIN GENERATED CALCINATION RECIPES (tools/gen_recipes.py)

/// Every `theurgy:calcination` recipe the jar ships, from
/// `data/theurgy/recipe/calcination/*.json` (86 files, all `time: 100`).
/// A tag ingredient keeps the tag the jar names; the engine's merged item tags
/// answer its membership (`native_server_item_in_tag`, API 63).
#[rustfmt::skip]
pub static RECIPES: &[Recipe] = &[
    Recipe { input: Ingredient::Item("minecraft:armadillo_scute"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_armadillo_scute.json
    Recipe { input: Ingredient::Item("minecraft:arrow"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_arrow.json
    Recipe { input: Ingredient::Item("minecraft:beef"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_beef.json
    Recipe { input: Ingredient::Item("minecraft:blaze_rod"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_blaze_rod.json
    Recipe { input: Ingredient::Item("minecraft:bone"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_bone.json
    Recipe { input: Ingredient::Item("minecraft:bone_meal"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_bone_meal.json
    Recipe { input: Ingredient::Item("minecraft:chicken"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_chicken.json
    Recipe { input: Ingredient::Item("minecraft:cod"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cod.json
    Recipe { input: Ingredient::Item("minecraft:cooked_beef"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cooked_beef.json
    Recipe { input: Ingredient::Item("minecraft:cooked_chicken"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cooked_chicken.json
    Recipe { input: Ingredient::Item("minecraft:cooked_cod"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cooked_cod.json
    Recipe { input: Ingredient::Item("minecraft:cooked_mutton"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cooked_mutton.json
    Recipe { input: Ingredient::Item("minecraft:cooked_porkchop"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cooked_porkchop.json
    Recipe { input: Ingredient::Item("minecraft:cooked_rabbit"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cooked_rabbit.json
    Recipe { input: Ingredient::Item("minecraft:cooked_salmon"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_cooked_salmon.json
    Recipe { input: Ingredient::Item("minecraft:dragon_egg"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 5, time: 100 }, // alchemical_salt_creature_from_dragon_egg.json
    Recipe { input: Ingredient::Item("minecraft:egg"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_egg.json
    Recipe { input: Ingredient::Item("minecraft:elytra"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_elytra.json
    Recipe { input: Ingredient::Item("minecraft:ender_pearl"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_ender_pearl.json
    Recipe { input: Ingredient::Item("minecraft:feather"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_feather.json
    Recipe { input: Ingredient::Item("minecraft:ghast_tear"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_ghast_tear.json
    Recipe { input: Ingredient::Item("minecraft:glow_ink_sac"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_glow_ink_sac.json
    Recipe { input: Ingredient::Item("minecraft:gunpowder"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_gunpowder.json
    Recipe { input: Ingredient::Item("minecraft:heart_of_the_sea"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 5, time: 100 }, // alchemical_salt_creature_from_heart_of_the_sea.json
    Recipe { input: Ingredient::Item("minecraft:ink_sac"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_ink_sac.json
    Recipe { input: Ingredient::Item("minecraft:leather"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_leather.json
    Recipe { input: Ingredient::Item("minecraft:magma_cream"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_magma_cream.json
    Recipe { input: Ingredient::Item("minecraft:mutton"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_mutton.json
    Recipe { input: Ingredient::Item("minecraft:nether_star"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 5, time: 100 }, // alchemical_salt_creature_from_nether_star.json
    Recipe { input: Ingredient::Item("minecraft:phantom_membrane"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_phantom_membrane.json
    Recipe { input: Ingredient::Item("theurgy:alchemical_salt_plant"), count: 2, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_plant_salt.json
    Recipe { input: Ingredient::Item("minecraft:porkchop"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_porkchop.json
    Recipe { input: Ingredient::Item("minecraft:prismarine_crystals"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_prismarine_crystals.json
    Recipe { input: Ingredient::Item("minecraft:prismarine_shard"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_prismarine_shard.json
    Recipe { input: Ingredient::Item("minecraft:pufferfish"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_pufferfish.json
    Recipe { input: Ingredient::Item("minecraft:rabbit"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_rabbit.json
    Recipe { input: Ingredient::Item("minecraft:rabbit_foot"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_rabbit_foot.json
    Recipe { input: Ingredient::Item("minecraft:rabbit_hide"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_rabbit_hide.json
    Recipe { input: Ingredient::Item("minecraft:rotten_flesh"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_rotten_flesh.json
    Recipe { input: Ingredient::Item("minecraft:salmon"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_salmon.json
    Recipe { input: Ingredient::Item("minecraft:shulker_shell"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_shulker_shell.json
    Recipe { input: Ingredient::Item("minecraft:skeleton_skull"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_skeleton_skull.json
    Recipe { input: Ingredient::Item("minecraft:slime_ball"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_slime_ball.json
    Recipe { input: Ingredient::Item("minecraft:spider_eye"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_spider_eye.json
    Recipe { input: Ingredient::Item("minecraft:string"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_string.json
    Recipe { input: Ingredient::Item("minecraft:tropical_fish"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_tropical_fish.json
    Recipe { input: Ingredient::Item("minecraft:turtle_scute"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 1, time: 100 }, // alchemical_salt_creature_from_turtle_scute.json
    Recipe { input: Ingredient::Item("minecraft:wither_skeleton_skull"), count: 1, output: "theurgy:alchemical_salt_creature", output_count: 3, time: 100 }, // alchemical_salt_creature_from_wither_skeleton_skull.json
    Recipe { input: Ingredient::Tag("c:gems"), count: 1, output: "theurgy:alchemical_salt_mineral", output_count: 2, time: 100 }, // alchemical_salt_mineral_from_gems.json
    Recipe { input: Ingredient::Tag("c:ingots"), count: 1, output: "theurgy:alchemical_salt_mineral", output_count: 2, time: 100 }, // alchemical_salt_mineral_from_ingots.json
    Recipe { input: Ingredient::Tag("c:ores"), count: 1, output: "theurgy:alchemical_salt_mineral", output_count: 3, time: 100 }, // alchemical_salt_mineral_from_ores.json
    Recipe { input: Ingredient::Tag("theurgy:other_minerals"), count: 1, output: "theurgy:alchemical_salt_mineral", output_count: 2, time: 100 }, // alchemical_salt_mineral_from_other_minerals.json
    Recipe { input: Ingredient::Tag("c:raw_materials"), count: 1, output: "theurgy:alchemical_salt_mineral", output_count: 1, time: 100 }, // alchemical_salt_mineral_from_raw_materials.json
    Recipe { input: Ingredient::Item("theurgy:alchemical_salt_strata"), count: 5, output: "theurgy:alchemical_salt_mineral", output_count: 1, time: 100 }, // alchemical_salt_mineral_from_strata_salt.json
    Recipe { input: Ingredient::Tag("c:crops"), count: 1, output: "theurgy:alchemical_salt_plant", output_count: 1, time: 100 }, // alchemical_salt_plant_from_crops.json
    Recipe { input: Ingredient::Tag("minecraft:leaves"), count: 1, output: "theurgy:alchemical_salt_plant", output_count: 1, time: 100 }, // alchemical_salt_plant_from_leaves.json
    Recipe { input: Ingredient::Tag("minecraft:logs"), count: 1, output: "theurgy:alchemical_salt_plant", output_count: 1, time: 100 }, // alchemical_salt_plant_from_logs.json
    Recipe { input: Ingredient::Tag("minecraft:saplings"), count: 1, output: "theurgy:alchemical_salt_plant", output_count: 1, time: 100 }, // alchemical_salt_plant_from_saplings.json
    Recipe { input: Ingredient::Item("minecraft:blackstone"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_blackstone.json
    Recipe { input: Ingredient::Item("minecraft:blue_ice"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 64, time: 100 }, // alchemical_salt_strata_from_blue_ice.json
    Recipe { input: Ingredient::Item("minecraft:clay"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 4, time: 100 }, // alchemical_salt_strata_from_clay.json
    Recipe { input: Ingredient::Item("minecraft:clay_ball"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_clay_ball.json
    Recipe { input: Ingredient::Tag("c:cobblestones"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_cobblestone.json
    Recipe { input: Ingredient::Tag("c:concretes"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_concrete.json
    Recipe { input: Ingredient::Item("minecraft:crimson_nylium"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_crimson_nylium.json
    Recipe { input: Ingredient::Item("minecraft:crying_obsidian"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 3, time: 100 }, // alchemical_salt_strata_from_crying_obsidian.json
    Recipe { input: Ingredient::Item("theurgy:crystallized_lava"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 8, time: 100 }, // alchemical_salt_strata_from_crystallized_lava.json
    Recipe { input: Ingredient::Item("theurgy:crystallized_water"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 4, time: 100 }, // alchemical_salt_strata_from_crystallized_water.json
    Recipe { input: Ingredient::Tag("minecraft:dirt"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_dirt.json
    Recipe { input: Ingredient::Item("minecraft:end_stone"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_end_stone.json
    Recipe { input: Ingredient::Item("minecraft:gravel"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_gravel.json
    Recipe { input: Ingredient::Item("minecraft:ice"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_ice.json
    Recipe { input: Ingredient::Item("minecraft:magma_block"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_magma_block.json
    Recipe { input: Ingredient::Item("minecraft:mycelium"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_mycelium.json
    Recipe { input: Ingredient::Tag("c:netherracks"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_netherrack.json
    Recipe { input: Ingredient::Item("minecraft:obsidian"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_obsidian.json
    Recipe { input: Ingredient::Item("minecraft:packed_ice"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 9, time: 100 }, // alchemical_salt_strata_from_packed_ice.json
    Recipe { input: Ingredient::Item("minecraft:purpur_block"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_purpur_block.json
    Recipe { input: Ingredient::Tag("minecraft:sand"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_sand.json
    Recipe { input: Ingredient::Tag("c:sandstone/blocks"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_sandstone.json
    Recipe { input: Ingredient::Item("minecraft:snowball"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_snowball.json
    Recipe { input: Ingredient::Item("minecraft:soul_sand"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_soul_sand.json
    Recipe { input: Ingredient::Item("minecraft:soul_soil"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_soul_soil.json
    Recipe { input: Ingredient::Tag("c:stones"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 1, time: 100 }, // alchemical_salt_strata_from_stone.json
    Recipe { input: Ingredient::Item("minecraft:terracotta"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_terracotta.json
    Recipe { input: Ingredient::Item("minecraft:warped_nylium"), count: 1, output: "theurgy:alchemical_salt_strata", output_count: 2, time: 100 }, // alchemical_salt_strata_from_warped_nylium.json
];
// END GENERATED CALCINATION RECIPES

/// Whether two stacks in the brazier's one slot are the same fuel, so an insert
/// may merge them. A different fuel is never converted into the held one (the
/// review finding); the glue also compares their components.
#[must_use]
pub fn fuel_merges(stored: &str, held: &str) -> bool {
    stored == held
}

/// How far a player without operator permission may be from an apparatus to read it.
pub const INSPECT_RANGE: f64 = 8.0;

/// `Commands.LEVEL_ADMINS`: the permission level that bypasses the reach check.
pub const LEVEL_ADMINS: u8 = 2;

/// Whether a caller may read an apparatus's contents: an operator, or a player
/// in the same level within [`INSPECT_RANGE`] of the block.
pub fn may_inspect(op: bool, same_level: bool, distance: f64) -> bool {
    op || (same_level && distance <= INSPECT_RANGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tools/tag_members.json`'s lists, cut to the members these tests use:
    /// the rule under test is membership, not the tag files.
    fn tags(tag: &str) -> &'static [&'static str] {
        match tag {
            "c:gems" => &[
                "minecraft:amethyst_shard",
                "minecraft:diamond",
                "minecraft:emerald",
                "minecraft:lapis_lazuli",
                "minecraft:prismarine_crystals",
                "minecraft:quartz",
            ],
            "c:ores" => &[
                "minecraft:coal_ore",
                "minecraft:iron_ore",
                "minecraft:diamond_ore",
            ],
            "c:ingots" => &[
                "minecraft:copper_ingot",
                "minecraft:gold_ingot",
                "minecraft:iron_ingot",
            ],
            "c:crops" => &["minecraft:carrot", "minecraft:potato", "minecraft:wheat"],
            "minecraft:logs" => &[
                "minecraft:oak_log",
                "minecraft:acacia_log",
                "minecraft:birch_log",
            ],
            "minecraft:sand" => &[
                "minecraft:sand",
                "minecraft:red_sand",
                "minecraft:suspicious_sand",
            ],
            "minecraft:leaves" => &["minecraft:oak_leaves", "minecraft:acacia_leaves"],
            _ => &[],
        }
    }

    fn in_tag(item: &str, tag: &str) -> bool {
        tags(tag).contains(&item)
    }

    /// `every_jar_recipe_is_carried`: the jar's recipes are the table, all 100 ticks.
    #[test]
    fn every_jar_recipe_is_carried() {
        assert_eq!(RECIPES.len(), 86, "the jar ships 86 calcination recipes");
        assert!(RECIPES.iter().all(|recipe| recipe.time == DEFAULT_TIME));
        assert!(
            RECIPES
                .iter()
                .all(|recipe| recipe.output.starts_with("theurgy:alchemical_salt"))
        );
        assert_eq!(
            RECIPES
                .iter()
                .filter(|r| matches!(r.input, Ingredient::Tag(_)))
                .count(),
            16,
            "16 recipes name a tag"
        );
    }

    /// `alchemical_salt_strata_from_blackstone.json`: 1 blackstone → 1 strata salt.
    #[test]
    fn blackstone_gives_one_strata_salt() {
        let recipe = recipe_for("minecraft:blackstone", &|_: &str, _: &str| false)
            .expect("blackstone calcines");
        assert_eq!(recipe.output, "theurgy:alchemical_salt_strata");
        assert_eq!(recipe.output_count, 1);
        assert_eq!(recipe.count, 1);
    }

    /// The jar's ratios: `packed_ice` 9, `blue_ice` 64, `clay` 4, `obsidian` 2.
    #[test]
    fn the_jar_ratios_are_kept() {
        let none = &|_: &str, _: &str| false;
        assert_eq!(
            recipe_for("minecraft:packed_ice", none)
                .unwrap()
                .output_count,
            9
        );
        assert_eq!(
            recipe_for("minecraft:blue_ice", none).unwrap().output_count,
            64
        );
        assert_eq!(recipe_for("minecraft:clay", none).unwrap().output_count, 4);
        assert_eq!(
            recipe_for("minecraft:obsidian", none).unwrap().output_count,
            2
        );
        assert_eq!(
            recipe_for("minecraft:dragon_egg", none)
                .unwrap()
                .output_count,
            5
        );
    }

    /// `…_from_gems.json` names `#c:gems`: a gem matches through the tag.
    #[test]
    fn tag_ingredients_match_their_members() {
        let rough = recipe_for("minecraft:stick", &in_tag);
        assert!(rough.is_none(), "a non-member matches no tag recipe");
        let gems = recipe_for("minecraft:diamond", &in_tag).expect("a gem calcines");
        assert_eq!(gems.output, "theurgy:alchemical_salt_mineral");
        assert_eq!(gems.output_count, 2);
        let ore = recipe_for("minecraft:iron_ore", &in_tag).unwrap();
        assert_eq!(ore.output_count, 3, "#c:ores");
        let log = recipe_for("minecraft:oak_log", &in_tag).unwrap();
        assert_eq!(log.output, "theurgy:alchemical_salt_plant");
        let sand = recipe_for("minecraft:sand", &in_tag).unwrap();
        assert_eq!(sand.output, "theurgy:alchemical_salt_strata");
    }

    /// A tag read that fails (an unknown tag) leaves the recipe inert.
    #[test]
    fn an_unreadable_tag_matches_nothing() {
        assert!(recipe_for("minecraft:diamond", &|_: &str, _: &str| false).is_none());
        assert!(in_tag("minecraft:diamond", "c:gems"));
        assert!(!in_tag("minecraft:diamond", "c:nonexistent"));
    }

    /// A recipe whose ingredient is a count of an item (`plant salt` ×2).
    #[test]
    fn a_counted_recipe_keeps_its_count() {
        let recipe = recipe_for("theurgy:alchemical_salt_plant", &|_: &str, _: &str| false)
            .expect("plant salt calcines");
        assert_eq!(recipe.count, 2);
        assert_eq!(recipe.output, "theurgy:alchemical_salt_creature");
    }

    /// An item no recipe names has no recipe.
    #[test]
    fn an_unknown_item_has_no_recipe() {
        assert!(recipe_for("minecraft:stick", &|_: &str, _: &str| false).is_none());
        assert!(recipe_for("theurgy:mercury_shard", &|_: &str, _: &str| false).is_none());
    }

    /// One craft takes `time` heated ticks and consumes the recipe's count.
    #[test]
    fn a_craft_takes_the_recipes_time() {
        let none = &|_: &str, _: &str| false;
        let mut oven = Oven::new();
        oven.input_changed("minecraft:blackstone", none);
        let input = Some(("minecraft:blackstone", 3));
        for _ in 0..DEFAULT_TIME - 1 {
            assert_eq!(oven.tick(true, input, None, none), Step::Working);
        }
        let finished = oven.tick(true, input, None, none);
        let Step::Craft(recipe) = finished else {
            panic!("the recipe should finish on the 100th heated tick, got {finished:?}");
        };
        assert_eq!(
            (recipe.output, recipe.output_count, recipe.count),
            ("theurgy:alchemical_salt_strata", 1, 1)
        );
        assert_eq!(oven.progress, 0, "progress starts over after a craft");
    }

    /// A cold tick at 99 throws the progress away (`stopProcessing`).
    #[test]
    fn heat_loss_resets_the_progress() {
        let none = &|_: &str, _: &str| false;
        let mut oven = Oven::new();
        oven.input_changed("minecraft:blackstone", none);
        let input = Some(("minecraft:blackstone", 3));
        for _ in 0..99 {
            assert_eq!(oven.tick(true, input, None, none), Step::Working);
        }
        assert_eq!(oven.tick(false, input, None, none), Step::Stopped);
        assert_eq!(oven.progress, 0);
        assert_eq!(
            oven.tick(true, input, None, none),
            Step::Working,
            "it starts again from 0"
        );
    }

    /// An unheated oven never progresses, and no input never progresses.
    #[test]
    fn nothing_happens_cold_or_empty() {
        let none = &|_: &str, _: &str| false;
        let mut oven = Oven::new();
        oven.input_changed("minecraft:blackstone", none);
        for _ in 0..200 {
            assert_eq!(
                oven.tick(false, Some(("minecraft:blackstone", 64)), None, none),
                Step::Stopped
            );
        }
        assert_eq!(oven.tick(true, None, None, none), Step::Stopped);
        assert_eq!(oven.progress, 0);
    }

    /// `SizedIngredient.test` needs the stack to hold the recipe's count.
    #[test]
    fn too_few_items_do_not_progress() {
        let none = &|_: &str, _: &str| false;
        let mut oven = Oven::new();
        oven.input_changed("theurgy:alchemical_salt_strata", none);
        for _ in 0..150 {
            let step = oven.tick(
                true,
                Some(("theurgy:alchemical_salt_strata", 4)),
                None,
                none,
            );
            assert_eq!(step, Step::Stopped, "5 strata salts are needed");
        }
        assert_eq!(
            oven.tick(
                true,
                Some(("theurgy:alchemical_salt_strata", 5)),
                None,
                none
            ),
            Step::Working
        );
    }

    /// The whole result must fit the output slot, or nothing is crafted.
    #[test]
    fn a_full_output_slot_stops_the_process() {
        let none = &|_: &str, _: &str| false;
        let mut oven = Oven::new();
        oven.input_changed("minecraft:packed_ice", none);
        let input = Some(("minecraft:packed_ice", 1));
        let full = Some(("theurgy:alchemical_salt_strata", 60));
        for _ in 0..150 {
            assert_eq!(
                oven.tick(true, input, full, none),
                Step::Stopped,
                "60 + 9 > 64"
            );
        }
        let room = Some(("theurgy:alchemical_salt_strata", 55));
        for _ in 0..DEFAULT_TIME - 1 {
            assert_eq!(oven.tick(true, input, room, none), Step::Working);
        }
        assert!(
            matches!(oven.tick(true, input, room, none), Step::Craft(_)),
            "55 + 9 = 64 fits"
        );
    }

    /// A different item in the output slot blocks a craft.
    #[test]
    fn another_item_in_the_output_blocks_a_craft() {
        let none = &|_: &str, _: &str| false;
        let mut oven = Oven::new();
        oven.input_changed("minecraft:blackstone", none);
        let step = oven.tick(
            true,
            Some(("minecraft:blackstone", 1)),
            Some(("minecraft:stone", 1)),
            none,
        );
        assert_eq!(step, Step::Stopped);
    }

    /// One coal heats for 1,600 ticks (`minecraft:coal`'s furnace burn time).
    #[test]
    fn one_coal_heats_for_sixteen_crafts() {
        let mut brazier = Brazier::new();
        let first = brazier.tick(Some(1600));
        assert!(first.consumed && first.lit && first.changed);
        let mut lit_ticks = 1;
        for _ in 0..5000 {
            let burn = brazier.tick(None);
            if !burn.lit {
                break;
            }
            lit_ticks += 1;
        }
        assert_eq!(lit_ticks, 1600, "one coal is 1,600 ticks of heat");
        assert_eq!(lit_ticks / u32::from(DEFAULT_TIME), 16, "16 crafts a coal");
    }

    /// A non-fuel is neither consumed nor burnt, and refuelling waits for the burn to end.
    #[test]
    fn fuel_rules() {
        let mut brazier = Brazier::new();
        let none = brazier.tick(None);
        assert!(!none.lit && !none.consumed);
        let not_fuel = brazier.tick(Some(0));
        assert!(!not_fuel.lit && !not_fuel.consumed);
        brazier.tick(Some(100));
        let while_burning = brazier.tick(Some(1600));
        assert!(
            !while_burning.consumed,
            "a lit brazier does not eat the next fuel"
        );
        assert!(!while_burning.changed);
    }

    /// `HeatConsumerBehaviour` samples the heat once every 20 ticks.
    #[test]
    fn heat_is_sampled_every_twenty_ticks() {
        assert!(heat_sample_due(0));
        assert!(!heat_sample_due(19));
        assert!(heat_sample_due(20));
        assert!(heat_sample_due(1600));
    }

    /// The whole process end to end: one coal, three blackstone, three salts.
    #[test]
    fn one_coal_calcines_three_blackstone() {
        let none = &|_: &str, _: &str| false;
        let mut brazier = Brazier::new();
        brazier.tick(Some(1600));
        let mut oven = Oven::new();
        oven.input_changed("minecraft:blackstone", none);
        let (mut input, mut output) = (3_u32, 0_u32);
        let mut crafts = 0;
        for _ in 0..1600 {
            let burn = brazier.tick(None);
            if !burn.lit {
                break;
            }
            let out = if output == 0 {
                None
            } else {
                Some(("theurgy:alchemical_salt_strata", output))
            };
            if let Step::Craft(recipe) =
                oven.tick(true, Some(("minecraft:blackstone", input)), out, none)
            {
                input -= recipe.count;
                output += recipe.output_count;
                crafts += 1;
            }
            if input == 0 {
                break;
            }
        }
        assert_eq!(crafts, 3, "three inputs, three crafts");
        assert_eq!(
            (input, output),
            (0, 3),
            "three blackstone became three strata salts"
        );
    }

    #[test]
    fn only_the_same_fuel_merges_in_the_brazier() {
        assert!(fuel_merges("minecraft:coal", "minecraft:coal"));
        assert!(
            !fuel_merges("minecraft:coal", "minecraft:charcoal"),
            "a different fuel is not converted into the held one"
        );
    }

    #[test]
    fn a_readout_needs_an_operator_reach_or_the_same_level() {
        assert!(may_inspect(true, false, f64::MAX), "an operator");
        assert!(may_inspect(false, true, 3.0), "a player in reach");
        assert!(!may_inspect(false, true, 8.5), "out of reach");
        assert!(!may_inspect(false, false, 1.0), "another level");
    }
}
