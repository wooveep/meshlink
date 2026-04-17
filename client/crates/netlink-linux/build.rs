use std::env;
use std::path::PathBuf;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let vendor_dir = manifest_dir.join("vendor/wireguard-tools-1.0.20250521/embeddable-wg-library");

    println!(
        "cargo:rerun-if-changed={}",
        vendor_dir.join("wireguard.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        vendor_dir.join("wireguard.h").display()
    );

    cc::Build::new()
        .file(vendor_dir.join("wireguard.c"))
        .include(&vendor_dir)
        .flag("-Wno-unused-function")
        .compile("meshlink_wireguard_uapi");
}
