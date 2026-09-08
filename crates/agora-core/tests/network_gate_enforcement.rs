//! The outbound-request gate must be enforced at the request chokepoints, not
//! at each feature module's entry point.
//!
//! These live in their own integration binary because the gate is process-wide:
//! a unit test installing a denying gate would race every other test whose
//! `Ctx::for_testing` installs a permissive one.
//!
//! Each case asserts that the request is refused *by the gate* rather than by
//! URL validation or by the network. That distinction is the whole point — a
//! check placed after URL validation would already have resolved the hostname,
//! and one placed only in the GET helper would miss POST entirely, which is how
//! GitHub sign-in and the governance client stayed reachable under Lockdown.

use agora_core::http_client::{self, ClientCategory, HttpClients};
use agora_core::network_gate;
use std::sync::Arc;

fn deny_everything() -> HttpClients {
    network_gate::install(Arc::new(network_gate::DenyAll));
    HttpClients::for_testing(reqwest::Client::new())
}

/// Every category, refused before any network work — including the two that
/// have no per-endpoint toggle of their own.
#[tokio::test]
async fn the_gate_refuses_every_category_on_the_get_path() {
    let clients = deny_everything();
    for category in [
        ClientCategory::MojangMetadata,
        ClientCategory::MojangContent,
        ClientCategory::Loader,
        ClientCategory::Modrinth,
        ClientCategory::Modpack,
        ClientCategory::GitHub,
        ClientCategory::Microsoft,
        ClientCategory::Registry,
        ClientCategory::AiAssistant,
        ClientCategory::JavaRuntime,
        ClientCategory::PinnedArtifact,
        ClientCategory::ConsentedContent,
    ] {
        let error = http_client::checked_get_bytes(&clients, category, "https://github.com/x")
            .await
            .expect_err("a denied gate must refuse the request");
        assert!(
            format!("{error:?}").contains("ERR_NETWORK_GATE_MISSING"),
            "{category:?} was not refused by the gate: {error:?}"
        );
    }
}

/// The POST path has to be gated too. Gating only the GET helper was the
/// specific gap that left the governance GraphQL client reachable.
#[tokio::test]
async fn the_gate_refuses_posts_as_well_as_gets() {
    let clients = deny_everything();
    let error = http_client::checked_post_form(
        &clients,
        ClientCategory::GitHub,
        "https://github.com/login/device/code",
        &[("client_id", "irrelevant")],
        &[],
    )
    .await
    .expect_err("a denied gate must refuse a POST");
    assert!(
        format!("{error:?}").contains("ERR_NETWORK_GATE_MISSING"),
        "POST was not refused by the gate: {error:?}"
    );
}

/// The blocking helpers are a separate code path and were separately ungated.
#[test]
fn the_gate_refuses_blocking_requests() {
    let clients = deny_everything();
    let error = http_client::blocking_checked_get_bytes(
        &clients,
        ClientCategory::Registry,
        "https://api.github.com/repos/x/y",
    )
    .expect_err("a denied gate must refuse a blocking request");
    assert!(
        format!("{error:?}").contains("ERR_NETWORK_GATE_MISSING"),
        "blocking request was not refused by the gate: {error:?}"
    );
}

/// A refusal must come from the gate, not from URL validation — otherwise the
/// check is running after the hostname has already been resolved.
#[tokio::test]
async fn the_gate_runs_before_url_validation() {
    let clients = deny_everything();
    // A URL that URL validation would reject on its own (plain HTTP). If the
    // gate ran second, the error would name the scheme instead.
    let error =
        http_client::checked_get_bytes(&clients, ClientCategory::GitHub, "http://github.com/x")
            .await
            .expect_err("must be refused");
    assert!(
        format!("{error:?}").contains("ERR_NETWORK_GATE_MISSING"),
        "the gate did not run first: {error:?}"
    );
}
