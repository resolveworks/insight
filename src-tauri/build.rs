fn main() {
    tauri_build::build();

    #[cfg(all(target_os = "windows", target_env = "msvc"))]
    {
        let manifest_path = std::path::Path::new("windows-app-manifest.xml")
            .canonicalize()
            .expect("windows-app-manifest.xml not found");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    }
}
