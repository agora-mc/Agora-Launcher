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
            discovery::games_discover,
            discovery::games_add_folder,
            discovery::games_remove_folder,
            bases::games_base_list,
            bases::games_base_build,
            bases::games_base_verify,
            bases::games_base_remove,
            instances::games_instance_list,
            instances::games_instance_get,
            instances::games_instance_create,
            instances::games_instance_delete,
            instances::games_instance_check,
            instances::games_instance_set_deployment,
            instances::games_instance_launch,
            content::games_content_list,
            content::games_content_add,
            content::games_content_verify,
            content::games_content_remove,
            content::games_instance_content_list,
            content::games_instance_content_add,
            content::games_instance_content_remove,
            content::games_instance_content_set_enabled,
            content::games_instance_content_move,
            fomod::games_fomod_show,
            fomod::games_fomod_install,
            plugins::games_plugins_list,
            plugins::games_plugins_set_active,
            plugins::games_plugins_set_locked,
            plugins::games_plugins_move,
            plugins::games_plugins_sort,
            plugins::games_plugins_check,
            ini_saves::games_ini_get,
            ini_saves::games_ini_set,
            ini_saves::games_saves_get,
            ini_saves::games_saves_set,
            tools::games_tools_list,
            tools::games_tools_run,
            tools::games_tools_rollback,
            tools::games_tools_remove,
            tools::games_tools_diff,
            tools::games_tools_swap_costs,
            tools::games_tools_swap_status,
            tools::games_tools_swap_restore,
        );
        assert_eq!(
            checked, 43,
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
