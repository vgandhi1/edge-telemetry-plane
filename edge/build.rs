fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "../proto/detcp/v1/telemetry.proto";
    println!("cargo:rerun-if-changed={proto}");
    tonic_build::configure().compile_protos(&[proto], &["../proto"])?;
    Ok(())
}
