// `sqlx::migrate!` embeds `migrations/` at compile time; without this, a change to a migration
// alone can leave a binary with the old copy (found in plan 89 PR 3).
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
