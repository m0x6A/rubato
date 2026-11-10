//! Gherkin suite for the companion. Real tmux on a private server (`TMUX_TMPDIR`), real git in
//! temporary repositories, real tmux-ide, a stand-in `claude` script, and a test double of the
//! rubato-server end of the websocket. No mocks of tmux or git.
//!
//! One steps module per feature file (design D8); the World lives in `world.rs`.

mod logs;
mod steps;
mod world;

use cucumber::World;

#[tokio::main]
async fn main() {
    logs::install();
    world::RubatoWorld::cucumber()
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .run_and_exit("features")
        .await;
}
