//! Microsoft Detours' import-table injection (`crates/agora-vfs-inject`) makes `agora_vfs.dll` a
//! process's first import by *ordinal 1*, so the DLL must export something at ordinal 1.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:agora_vfs_ordinal1,@1");
    }
}
