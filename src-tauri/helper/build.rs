fn main() {
    // Let the helper find the Steam API library sitting next to it (or in the
    // package's lib folder on Linux/macOS) without touching LD_LIBRARY_PATH.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN:$ORIGIN/../lib/IdleKit:$ORIGIN/../lib/idlekit");
    } else if target.contains("apple") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path:@executable_path/../Resources");
    }
}
