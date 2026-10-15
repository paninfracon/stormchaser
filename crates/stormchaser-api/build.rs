use std::error::Error;
use vergen_gitcl::{Build, Cargo, Emitter, Gitcl, Rustc, Sysinfo};

fn main() -> Result<(), Box<dyn Error>> {
    // Outside a git checkout (a source tarball) vergen sets no git values, and the
    // binary's version string, which reads them with env!(), would not compile. Values in
    // the environment take their place; a published crate keeps the commit vergen reads
    // from .cargo_vcs_info.json.
    let in_git = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|out| out.status.success());
    if !in_git {
        std::env::set_var("VERGEN_GIT_BRANCH", "unknown");
        if !std::path::Path::new(".cargo_vcs_info.json").exists() {
            std::env::set_var("VERGEN_GIT_SHA", "unknown");
        }
    }

    Emitter::default()
        .add_instructions(&Build::all_build())?
        .add_instructions(&Cargo::all_cargo())?
        .add_instructions(&Gitcl::all_git())?
        .add_instructions(&Rustc::all_rustc())?
        .add_instructions(&Sysinfo::all_sysinfo())?
        .emit()?;
    Ok(())
}
