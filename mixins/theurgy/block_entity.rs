// The oven and the brazier get block entities (API 15).
fn observe_block_entity(
    block: &str,
    entity: &mut Option<(String, horizon_world::nbt::Compound)>,
) {
    crate::game::theurgy_state::on_block_entity(block, entity);
}
