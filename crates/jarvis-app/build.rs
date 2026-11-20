fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/windows/jarvis.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icon = manifest.join("../../packaging/windows/jarvis.ico");
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(icon.to_str().expect("icon path is UTF-8"))
        .set("ProductName", "Jarvis")
        .set("FileDescription", "Jarvis for Windows")
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set("LegalCopyright", "Apache-2.0")
        .set("OriginalFilename", "jarvis.exe");
    if let Err(error) = resource.compile() {
        panic!("could not compile Windows resources: {error}");
    }
}
