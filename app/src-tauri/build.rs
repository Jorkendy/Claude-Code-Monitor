fn main() {
    println!("cargo:rerun-if-changed=icons/tray/tray@2x.png");
    tauri_build::build()
}
