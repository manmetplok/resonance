//! Pre-generates all built-in wavetables at plugin build time and writes them
//! to `$OUT_DIR/wavetables.bin`. The runtime loads this blob via
//! `include_bytes!`, avoiding a multi-second additive-synthesis pass at every
//! plugin instantiation.

#[path = "src/dsp/wavetable_gen.rs"]
mod wavetable_gen;

fn main() {
    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR not set");
    let dest = std::path::Path::new(&out_dir).join("wavetables.bin");
    wavetable_gen::write_bundled(44_100.0, &dest);

    println!("cargo:rerun-if-changed=src/dsp/wavetable_gen.rs");
    println!("cargo:rerun-if-changed=build.rs");
}
