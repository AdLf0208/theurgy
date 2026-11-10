// The harness readouts (`/theurgy info|recipe|recipes`, API 8).
fn observe_register_commands(commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    crate::game::theurgy_state::on_register_commands(commands);
}
