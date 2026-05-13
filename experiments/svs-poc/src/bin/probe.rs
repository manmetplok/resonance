//! One-off: print the I/O signature of an ONNX file. Helps debug new voicebank exports.

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: probe <onnx-file>"))?;
    let session = ort::session::Session::builder()
        .map_err(|e| anyhow::anyhow!("builder: {e}"))?
        .commit_from_file(&path)
        .map_err(|e| anyhow::anyhow!("load {path}: {e}"))?;
    println!("=== INPUTS ===");
    for i in session.inputs() {
        println!("  {} : {:?}", i.name(), i.dtype());
    }
    println!("=== OUTPUTS ===");
    for o in session.outputs() {
        println!("  {} : {:?}", o.name(), o.dtype());
    }
    Ok(())
}
