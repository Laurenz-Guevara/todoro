// Gives the Windows exe todoro's icon. Windows releases are built on Windows,
// so the resource compiler only runs there.
fn main() {
    println!("cargo:rerun-if-changed=assets/todoro.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/todoro.ico");
        res.compile().expect("couldn't embed the Windows icon");
    }
}
