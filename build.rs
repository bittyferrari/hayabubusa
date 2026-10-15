fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // Embed the icon into the .exe so Explorer / taskbar show it.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile().expect("failed to embed Windows icon resource");
    }
}
