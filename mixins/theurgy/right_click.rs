// An empty hand on the oven or the brazier takes from it (API 21).
fn observe_right_click(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    canceled: &mut bool,
) {
    crate::game::theurgy_state::on_right_click_block(game, pos, canceled);
}
