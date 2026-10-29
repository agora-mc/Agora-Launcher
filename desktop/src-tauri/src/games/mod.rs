//! The desktop command layer for multi-game support (MASTER_SPEC §26).
//!
//! One file per slice. Every command is implemented now; a command added later before its
//! body exists should fail with `GamesError::not_implemented` rather than return a fake
//! success (`null` or an empty list).

pub mod bases;
pub mod catalog;
pub mod content;
pub mod discovery;
pub mod dto;
pub mod error;
pub mod fomod;
pub mod import;
pub mod ini_saves;
pub mod instances;
pub mod plugins;
pub mod tools;
pub mod user_files;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stub_error_names_its_command_and_code() {
        let error = error::GamesError::not_implemented("games_example");
        assert_eq!(error.code, "not_implemented");
        assert_eq!(error.message, "games_example is not implemented yet.");
        assert_eq!(error.core_code, None);
    }
}
