fn main() {
    // Sidecar binaries may keep cargo-packager's `-<target-triple>` suffix; expose the triple.
    println!("cargo:rustc-env=ASCII_PAINTER_TARGET={}", std::env::var("TARGET").expect("cargo sets TARGET"));
}
