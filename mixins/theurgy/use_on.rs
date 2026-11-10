// A player's right click on the oven or the brazier (API 13).
fn observe_use_on(
    game: &mut crate::game::Game<'_>,
    stack: &horizon_client::items::ItemStack,
    pos: [i32; 3],
    claimed: &mut bool,
) {
    crate::game::theurgy_state::on_use_item_on(game, stack, pos, claimed);
}
