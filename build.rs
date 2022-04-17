use std::process::Command;

fn main() {
    let git_hash_short_vec = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .expect("Cannot retrieve short git version")
        .stdout;
    let git_hash_short =
        std::str::from_utf8(&git_hash_short_vec).expect("Cannot retrieve short git hash");
    let cargo_pkg_version = env!("CARGO_PKG_VERSION");

    println!("cargo:rustc-env=APP_VERSION={cargo_pkg_version}-{git_hash_short}");

    let git_hash_vec = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("Cannot retrieve git version")
        .stdout;
    let git_hash = std::str::from_utf8(&git_hash_vec).expect("Cannot retrieve git hash");

    println!("cargo:rustc-env=GIT_HASH={git_hash}");
}
