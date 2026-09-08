# Task: one canonical GC vocabulary across persistence, preview, and launch

Repo: Agora, a Minecraft mod launcher. Rust workspace; `crates/agora-core` is the shared library
used by the Tauri desktop app, the CLI, and an MCP server. You are in a git worktree on branch
`fix/gc-canonical`.

## The defect

The JVM garbage-collector setting has three different vocabularies and the write path rejects the
only values the UI actually sends, so explicit user selections are silently persisted as `auto`.

The UI (`desktop/src/lib/tauri.ts`, `desktop/src/pages/InstanceEditor.tsx`) sends exactly:
`auto`, `manual`, `low_latency`, `high_efficiency`.

The single write path, `InstanceService::update_jvm` in `crates/agora-core/src/instance_service.rs`
(around line 384):

```rust
let gc = match gc.trim().to_ascii_lowercase().as_str() {
    "auto" | "g1gc" | "zgc" | "shenandoah" | "manual" => gc.trim().to_ascii_lowercase(),
    _ => "auto".to_string(),          // low_latency and high_efficiency land here
};
```

So picking "ZGC · low latency" saves `auto`, and picking "G1GC · high efficiency" saves `auto`.
Meanwhile `shenandoah` is accepted by the writer and understood by nobody.

The two read paths use the UI vocabulary and disagree with the writer:

- `crates/agora-core/src/launch_service.rs` around line 343 (builds real launch arguments):
  `"zgc" | "low_latency" => LowLatency`, `"high_efficiency" => HighEfficiency`,
  `"manual" => Manual`, `_ => None` (None means Auto).
- `crates/agora-core/src/models.rs` around line 60 (`to_args_for_java`, the editor preview):
  `"auto" | "" | "g1gc" => None`, `"low_latency" | "zgc" => LowLatency`,
  `"high_efficiency" => HighEfficiency`, `"manual" => Manual`, `_ => None`.

`crate::gc::GcProfile` has exactly three variants: `LowLatency` (Generational ZGC),
`HighEfficiency` (Aikar's G1GC), `Manual` (raw user flags). "Auto" is represented as `None`.

## Required behaviour

Introduce one canonical selection type — a `GcSelection` enum with `Auto`, `LowLatency`,
`HighEfficiency`, `Manual` — and use it for persistence, preview, and launch resolution. Put it in
`crates/agora-core/src/gc.rs` next to `GcProfile`, and give it a method that resolves to
`Option<GcProfile>` so the existing `None == Auto` convention at the call sites is preserved.

Two distinct entry points, both producing the same canonical enum:

**Strict validation for new input** (used by `update_jvm`):

| Input | Result |
|---|---|
| `auto`, `manual`, `low_latency`, `high_efficiency` | accept, persist canonically |
| `g1gc` | accept as a legacy alias for `auto` |
| `zgc` | accept as a legacy alias for `low_latency` |
| `shenandoah` | **reject** — unsupported, no implemented meaning |
| anything else | **reject** |

Rejection must be a real error returned to the caller (a `LauncherError::Generic` with a clear
code and message), **not** a silent substitution. Today a bad value silently becomes `auto`; that
is the bug. The row must be left unchanged when validation fails.

**Compatibility decoding for persisted rows** (used by launch resolution and the preview):

| Stored value | Decodes to |
|---|---|
| the four canonical values | themselves |
| `g1gc` | `Auto` |
| `zgc` | `LowLatency` |
| `shenandoah` | `Auto` — this is the behaviour those instances have actually been getting |
| empty string | `Auto` |
| anything else unrecognised | `Auto`, preserving today's `_ => None` fallback |

`g1gc` maps to `Auto`, **not** to `HighEfficiency`. Both existing read paths already treat it as
Auto, and remapping it would silently change the launch flags of existing instances on upgrade.

Do **not** write a data migration and do **not** mutate rows just because they were read.
Normalize on read; write canonical values on successful updates. Correctness must hold without a
migration pass.

Matching is case-insensitive and trims surrounding whitespace, as today.

## Files you may change

- `crates/agora-core/src/gc.rs` — add `GcSelection` and the two entry points.
- `crates/agora-core/src/instance_service.rs` — `update_jvm` only.
- `crates/agora-core/src/launch_service.rs` — the `jvm_gc_profile` match only.
- `crates/agora-core/src/models.rs` — the `to_args_for_java` match only.

Do not touch anything else. In particular do not change the database schema, do not change
`GcProfile`, and do not touch the `desktop/` frontend — the UI vocabulary is already correct and
is what we are standardising on. If a CLI or MCP call site fails to compile because of your change,
fix that call site minimally and say so in your report.

## Tests you must add

Put them in the existing test modules of the files you touch. Each must fail before the change:

1. **Round-trip through the real write path.** For each of `low_latency`, `high_efficiency`,
   `manual`, `auto`: call `InstanceService::update_jvm`, read the row back from the database, and
   assert both the persisted string and the resolved `Option<GcProfile>` that launch would use.
   `instance_service.rs` already has a `fn context()` test helper that builds a `Ctx` and
   initialises the local-state DB — use it.
2. **Assert the resulting flags, not just the enum.** Auto can coincidentally select the same
   collector, which would hide a fallback bug. For `low_latency` and `high_efficiency`, assert the
   generated JVM argument string actually differs from what `auto` produces on the same Java
   version and memory size.
3. **Invalid input is rejected without changing the row.** `update_jvm` with `"shenandoah"` and
   with `"nonsense"` returns `Err`, and the stored `jvm_gc` is exactly what it was before.
4. **Legacy row fixtures decode correctly.** Write rows containing `g1gc`, `zgc`, `shenandoah`,
   `""`, and `"garbage"` directly into the DB, then assert what launch resolution and the preview
   each produce — `Auto`, `LowLatency`, `Auto`, `Auto`, `Auto` respectively.
5. **Launch and preview agree.** For every value in the compatibility table, assert
   `launch_service`'s resolution and `models.rs`'s preview resolution produce the same
   `Option<GcProfile>`. This is the property whose violation caused the original drift.

## Verify before you finish

```
cargo fmt --all --check
cargo clippy -p agora-core -p agora-cli --all-targets --all-features -- -D warnings
cargo test -p agora-core --lib
```

Clippy runs with `-D warnings`. Report anything you could not make pass.
