use std::env;
use std::fs;
use std::path::PathBuf;

/// Frozen Phase 1 HDL mailbox: 4 KiB control + 2 × 64 KiB slots.
const HDL_CONTROL: u64 = 4096;
const HDL_SLOT: u64 = 65536;
const HDL_BYTES: u64 = HDL_CONTROL + 2 * HDL_SLOT;
const HDL_OFFSET: u64 = 0x0140_0000;

fn assert_hdl_no_overlap(d1: bool) {
    assert_eq!(HDL_BYTES, 0x21000, "HDL mailbox is 132 KiB");
    if d1 {
        const DRAM: u64 = 0x4000_0000;
        const FB_META: u64 = 0x40FF_F000;
        const FB: u64 = 0x4100_0000;
        const FB_END: u64 = FB + 2048 * 480;
        const HDL: u64 = DRAM + HDL_OFFSET;
        const DRAM_END: u64 = 0x6000_0000;
        assert_eq!(HDL, 0x4140_0000);
        assert!(FB_META + 4096 <= FB, "D1 .fb_meta overlaps .fb");
        assert!(FB_END <= HDL, "D1 .fb overlaps .hdl");
        assert!(HDL + HDL_BYTES <= DRAM_END, "D1 .hdl leaves D1 DRAM");
        assert!(HDL + HDL_BYTES > FB_END, "D1 .hdl is empty or inverted");
    } else {
        const DRAM: u64 = 0x8000_0000;
        const FB_META: u64 = 0x80FF_F000;
        const FB: u64 = 0x8100_0000;
        const FB_END: u64 = FB + 4096 * 768;
        const HDL: u64 = DRAM + HDL_OFFSET;
        const DTB: u64 = 0x8200_0000;
        const HEAP: u64 = 0x8201_0000;
        assert_eq!(HDL, 0x8140_0000);
        assert!(FB_META + 4096 <= FB, "virt .fb_meta overlaps .fb");
        assert!(FB_END <= HDL, "virt .fb overlaps .hdl");
        assert!(HDL + HDL_BYTES <= DTB, "virt .hdl overlaps VM DTB");
        assert!(HDL + HDL_BYTES <= HEAP, "virt .hdl overlaps heap");
        assert_eq!(FB_END, 0x8130_0000, "virt .fb is 3 MiB");
    }
}

fn main() {
    println!("cargo:rerun-if-changed=link.x");
    println!("cargo:rerun-if-changed=d1.ld");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-check-cfg=cfg(machine_d1)");
    println!("cargo:rustc-check-cfg=cfg(machine_virt)");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let d1 = env::var("CARGO_FEATURE_D1").is_ok();
    assert_hdl_no_overlap(d1);
    let src = if d1 { "d1.ld" } else { "link.x" };
    fs::copy(src, out_dir.join("link.x")).expect("failed to copy linker script");
    if d1 {
        println!("cargo:rustc-cfg=machine_d1");
    } else {
        println!("cargo:rustc-cfg=machine_virt");
    }

    println!("cargo:rustc-link-search={}", out_dir.display());
    // Needed when building from the workspace root: package `.cargo/config.toml`
    // is not loaded, so rustc never gets `-Tlink.x` from there.
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rustc-link-arg=--relax");
    println!("cargo:rustc-link-arg=--no-relax-gp");
}
