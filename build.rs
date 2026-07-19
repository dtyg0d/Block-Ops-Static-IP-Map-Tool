#[cfg(target_os = "windows")]
fn main() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/blockops_app_icon.ico");
    res.set("FileDescription", "BlockOps Static IP Manager");
    res.set("ProductName", "BlockOps Static IP Manager 2.1.1");
    res.set("CompanyName", "BlockOps Mining");
    res.compile().expect("failed to embed Windows resource");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
