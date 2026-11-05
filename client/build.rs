fn main() {
    // Same look on Windows, macOS and Linux; follows the system light/dark theme.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint build failed");

    // Windows: embed the icon and version info in the .exe (works when
    // cross-compiling from Linux too, using mingw's windres).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "Paddock")
            .set("FileDescription", "Paddock - versões de CAD e software")
            .set("CompanyName", "Paddock");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows resources: {e}");
        }
    }
}
