fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=stomprig.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    embed_resource::compile_for("stomprig.rc", ["stomprig"], embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the StompRig Windows icon");
}
