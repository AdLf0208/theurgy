// One server tick: heat, progress and output for the oven; the burn for the
// brazier (API 17).
fn observe_block_entity_tick(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    entity: &mut horizon_client::containers::ModBlockEntity,
) {
    crate::game::theurgy_state::on_tick(game, pos, entity);
}
