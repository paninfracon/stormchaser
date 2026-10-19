// Rebuild when a migration is added: sqlx::migrate! embeds the directory's files.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
