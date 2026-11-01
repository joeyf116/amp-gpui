// Embeds the icon and version details in the exe. GPUI loads the window's icon
// from this same resource.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set("ProductName", "Amp GPUI")
            .set("FileDescription", "Amp GPUI")
            .compile()
            .expect("the Windows resource compiler should be available");
    }
}
