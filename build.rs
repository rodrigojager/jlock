// The target triple jlock is built for, so that `jlock update` downloads the matching archive.
fn main() {
    println!(
        "cargo:rustc-env=BUILD_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap()
    );
}
