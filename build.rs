fn main() {
    slint_build::compile("ui/main.slint").expect("failed to compile Slint UI");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.2.2".into());
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico")
            .set_icon_with_id("assets/model-file.ico", "2")
            .set("FileDescription", "3D Viewer")
            .set("ProductName", "3D Viewer")
            .set("CompanyName", "Gallery Inc.")
            .set("InternalName", "Gallery3DViewer.exe")
            .set("OriginalFilename", "Gallery3DViewer.exe")
            .set("FileVersion", &version)
            .set("ProductVersion", &version)
            .set("LegalCopyright", "Copyright (c) Gallery Inc.");
        res.compile().expect("failed to embed Windows resources");
    }
}
