use prost_build::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let include_dirs = &["../proto"];
    let sources = &["../proto/ignition.proto"];

    let mut config = Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");

    tonic_build::configure()
        .build_client(false)
        .compile_protos_with_config(config, sources, include_dirs)
        .expect("failed to compile protobuf");

    Ok(())
}
