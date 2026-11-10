// Both apparatus tick on the server (API 15).
fn observe_block_entity_ticks(block: &str, ticks: &mut bool) {
    crate::game::theurgy_state::on_block_entity_ticks(block, ticks);
}
