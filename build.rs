fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // clap builds the complete command tree on the main thread. The debug
        // binary crosses Windows' 1 MiB default stack as the auditable
        // lifecycle commands are added, although the optimized binary does not.
        // Reserve enough virtual stack for debug and release to behave alike.
        println!("cargo:rustc-link-arg-bin=alpine=/STACK:8388608");
    }
}
