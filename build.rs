fn main() {
    println!("cargo:rerun-if-changed=assets/icons/aidraw.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/aidraw.ico")
            .set("ProductName", "AIDRAW")
            .set("FileDescription", "AIDRAW Image Studio")
            .compile()
            .expect("Failed to embed AIDRAW Windows icon");
    }
}
