//! Minecraft's content flows over core's provider interface: the built-in
//! Modrinth and Technic providers, browsing them alongside the curated
//! catalog, and turning a provider's plan into an install.

pub mod browse;
pub mod install;
pub mod modrinth;
pub mod technic;
