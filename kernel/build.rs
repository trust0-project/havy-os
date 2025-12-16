use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=link.x");
    println!("cargo:rerun-if-changed=build.rs");
    
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    
    // Copy our complete link.x (with MEMORY, REGION_ALIAS, and SECTIONS)
    // riscv-rt's automatic `-Tlink.x` will find this via our search path
    let link_script = out_dir.join("link.x");
    fs::copy("link.x", &link_script).expect("failed to copy link.x");
    
    // Add output directory to search path FIRST so our link.x is found before riscv-rt's
    println!("cargo:rustc-link-search={}", out_dir.display());
}
