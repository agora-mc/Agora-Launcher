//! A program that sleeps for ten minutes whatever its arguments are. The VFS tests run it as the
//! helper beside a copy of `agora_vfs.dll`, to see that a helper which hangs is ended after its timeout
//! and takes its target with it. Built with the crate, so `cargo test` finds it as
//! `CARGO_BIN_EXE_agora-vfs-sleeper`.

fn main() {
    std::thread::sleep(std::time::Duration::from_secs(600));
}
