// Their bodies (API 8).
fn observe_perform_command(
    game: &mut crate::game::Game<'_>,
    player: &crate::game::native_server::NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    crate::game::theurgy_state::on_perform_command(game, player, command, ran);
}
