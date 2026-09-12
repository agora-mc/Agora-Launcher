# Distribution fixtures

`agora-plugin-update.json` files that must keep loading, and files that must keep being refused,
under distribution schema 1.

Kept separate from the manifest fixtures one directory up because they pin a separate contract.
How a plugin is published and what a plugin is allowed to do change on different schedules, and
an author can alter one without touching the other.

Enforced by `crates/agora-plugin-api/tests/compatibility.rs`. Adding a fixture after fixing a
contract bug is cheap and correct; **changing** one is a deliberate compatibility decision and
should be reviewed as exactly that.

The keys in these files are `0x01…` and `0x02…` repeated — obviously fake, and never used to
verify anything. What is under test is the shape of the file, not cryptography; that is covered
by `crates/agora-core/src/plugins/updates.rs` and `crates/agora/tests/signing_round_trip.rs`.
