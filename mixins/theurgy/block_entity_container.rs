// The oven's input/output slots and the brazier's fuel slot (API 16).
fn observe_block_entity_container(
    block: &str,
    container: &mut Option<horizon_client::containers::ModContainer>,
) {
    crate::game::theurgy_state::on_container(block, container);
}
