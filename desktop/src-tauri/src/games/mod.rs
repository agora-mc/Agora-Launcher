//! The desktop command layer for multi-game support (MASTER_SPEC §26).
//!
//! One file per slice, so each slice works in its own file and never edits `lib.rs` or the
//! handler list. Every command a slice has not built yet is registered now as a stub that
//! fails with `not_implemented`; the tests below prove that none of them returns a fake
//! success (`null` or an empty list). `games_list` is the one command that is real.

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
    use super::error::NOT_IMPLEMENTED;
    use super::*;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime builds")
            .block_on(future)
    }

    /// Every stub, by the module it lives in. A stub added without an entry here is not
    /// tested, so keep this list in step with the modules.
    macro_rules! assert_every_stub_fails {
        ($($module:ident :: $command:ident),* $(,)?) => {{
            let mut checked = 0usize;
            $(
                let result = block_on($module::$command());
                let error = match result {
                    Ok(value) => panic!(
                        "{} succeeded with {value}; a stub must fail",
                        stringify!($command)
                    ),
                    Err(error) => error,
                };
                assert_eq!(error.code, NOT_IMPLEMENTED, "{}", stringify!($command));
                assert!(
                    error.message.contains(stringify!($command)),
                    "{} must name itself, got {:?}",
                    stringify!($command),
                    error.message
                );
                checked += 1;
            )*
            checked
        }};
    }

    #[test]
    fn every_stub_fails_loudly_with_not_implemented() {
        let checked = assert_every_stub_fails!(
            tools::games_tools_list,
            tools::games_tools_run,
            tools::games_tools_rollback,
            tools::games_tools_remove,
            tools::games_tools_diff,
            tools::games_tools_swap_costs,
            tools::games_tools_swap_status,
            tools::games_tools_swap_restore,
            import::games_import_mo2_scan,
            import::games_import_mo2,
            catalog::games_catalog_list,
            catalog::games_catalog_install,
            user_files::games_user_files_status,
            user_files::games_user_files_restore,
        );
        assert_eq!(
            "the stub list drifted from the brief's command list"
        );
    }

    #[test]
    fn a_stub_error_names_its_command_and_code() {
        let error = error::GamesError::not_implemented("games_example");
        assert_eq!(error.code, "not_implemented");
        assert_eq!(error.message, "games_example is not implemented yet.");
        assert_eq!(error.core_code, None);
    }
}
