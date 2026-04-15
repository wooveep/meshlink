fn main() {
    let proto_dir = "../../../proto";
    tonic_build::configure()
        .build_server(false)
        .compile_protos(
            &[
                "../../../proto/common.proto",
                "../../../proto/management.proto",
                "../../../proto/relay.proto",
                "../../../proto/signal.proto",
            ],
            &[proto_dir],
        )
        .expect("compile protobuf definitions");
}
