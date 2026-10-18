fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../nettime.ico")
            .set_manifest_file("../desktop.manifest")
            .compile()
            .expect("Windows icon/DPI resources");
    }
}
