# Task: make instance cloning link-safe and give it a safe destination contract

Repo: Agora, a Minecraft mod launcher. Rust workspace; `crates/agora-core` is the shared library.
You are in a git worktree on branch `fix/clone-link-safety`.

## Files you may change

- `crates/agora-core/src/clone.rs` — this is the whole job. It is a self-contained module.
- `crates/agora-core/src/error.rs` — ONLY if you need to add an error variant, and only additively.

Do not touch any other file. Do not refactor anything outside `clone.rs`. There is exactly one
production caller path (instance cloning from the desktop app and CLI, both via
`clone_instance`); keep the existing `pub fn clone_instance(src_dir, dest_dir, prefs) ->
Result<String, String>` signature and the existing `ClonePrefs` field names so callers still
compile. If you believe the signature must change, stop and say so instead of changing it.

## The two defects

**1. Cloning follows symlinks and Windows reparse points.** `copy_entry` decides what to do with
`src.is_dir()` / `src.is_file()`, and `clone_instance` skips missing children with
`src_child.exists()`. All three of those follow links, and `fs::read_dir` follows a link to a
directory. So with the *default* prefs (`use_sym_links: false`, `use_hard_links: false`, every
`copy_*: true`):

- a symlink or directory junction inside `mods/` pointing at `C:\Users\me\Documents` is deep-copied
  into the clone — content escapes the instance;
- a link pointing at an ancestor of the instance recurses forever — `copy_entry` is recursive, so
  this is a stack overflow or a full disk;
- a link to a file outside the instance is silently materialised as a real copy.

**2. The destination is deleted unconditionally.** `clone_instance` starts with
`if dest_dir.exists() { fs::remove_dir_all(dest_dir)?; }` before it has copied a single byte. If
the destination equals the source, or is an ancestor of it, this destroys the source. If it is an
existing unrelated directory, it is gone before any work succeeds.

## Required behaviour

**Link policy: refuse, do not skip and do not recreate.** When the selected content contains a
symlink or an unsupported filesystem redirection, fail the clone with an error that names the
offending path(s) and explains the options (exclude that content via the `copy_*` prefs, or use a
link-based clone mode). Silently skipping produces a clone missing saves or configs that looks
successful; silently recreating the link violates the independence the default prefs promise.

- Classify entries with `symlink_metadata()`, never `is_dir()`/`is_file()`/`exists()`, before
  deciding to traverse. Apply the policy to the source root, the top-level mapped directories, the
  two copied manifest files, and every nested entry.
- `exists()` must not be used to decide an entry is absent: it hides dangling links and conflates
  some I/O errors with absence. Distinguish "not present" from "error" and fail on the latter.
- On Windows, a reparse point is **not** synonymous with a symlink. Junctions are path
  redirections and get the link policy; cloud-storage placeholders (OneDrive/Dropbox) are also
  reparse points and are not directory escapes. For now: classify what you can via
  `std::os::windows::fs::MetadataExt::file_attributes()` / `FileTypeExt`, and refuse an
  *unrecognised* reparse point with a clear message rather than claiming it is a malicious symlink.
- An unsupported entry type (fifo, socket, device) or an access failure must be an explicit error,
  not a silent skip.
- When `use_sym_links` or `use_hard_links` is set, the user has explicitly asked for sharing, so
  link creation stays allowed — but a *failed* link attempt must not silently fall through into a
  copy path that then follows an unsafe source link.

**Destination contract:**

- Reject a destination that already exists (including a dangling link) instead of deleting it.
- Reject `dest == src`, and reject either being an ancestor of the other. Use resolved filesystem
  relationships (`std::fs::canonicalize` on the existing parts), not string-prefix comparison —
  `D:\a\bc` must not be treated as inside `D:\a\b`.
- Copy into a uniquely-named staging directory that this operation owns, then publish to the final
  destination with a rename that fails if the destination appeared meanwhile.
- On any failure, remove only the staging directory this operation created. Never remove the
  destination.

**Traversal hardening:** convert `copy_entry`'s recursion into an iterative traversal with an
explicit work stack so a deep tree cannot exhaust the stack. Add a depth cap and an entry-count
cap as defence in depth; exceeding a cap fails the clone rather than producing a truncated
"success".

## Tests you must add (in the existing `#[cfg(test)] mod tests` in `clone.rs`)

Every one of these must fail before your change and pass after:

1. A symlinked directory inside `mods/` pointing outside the instance → clone returns `Err`, the
   error names the path, and the destination does not exist afterwards.
2. A symlinked *file* inside `config/` pointing outside the instance → `Err`; the outside file's
   content is not present anywhere under the destination.
3. A symlink pointing at an ancestor of the source (a cycle) → returns `Err` in bounded time. This
   must not hang or overflow the stack.
4. A dangling symlink among the selected content → `Err`, not a silent skip.
5. `dest_dir` already exists and contains a file → `Err`, **and that file is still there with its
   original contents afterwards**.
6. `dest_dir == src_dir` → `Err`, and the source is completely intact afterwards.
7. `dest_dir` is a subdirectory of `src_dir` → `Err`, source intact.
8. A mid-copy failure leaves no staging directory behind and does not create the destination. (Use
   whatever injection point is cleanest — an unreadable entry, or a small internal test hook.)
9. Windows junction handling, `#[cfg(windows)]`: create a real junction (you can shell out to
   `cmd /c mklink /J`) inside `resourcepacks/` and assert it is refused. Symlink creation on
   Windows may need developer mode — for the *symlink* tests, if creating the symlink itself
   fails, skip the test body rather than failing it, but do NOT skip the junction test.

Keep the existing tests in that module passing. `test_clone_all_dirs_exist`, `test_clone_no_mods`,
`test_clone_hardlinks`, `test_clone_source_not_a_dir`, and `test_clone_default_prefs_all_true`
must all still pass — note that some of them may pass a destination that does not exist yet, which
is the supported case.

## Verify before you finish

```
cargo fmt --all --check
cargo clippy -p agora-core --all-targets --all-features -- -D warnings
cargo test -p agora-core --lib clone
```

Clippy runs with `-D warnings`, so warnings are failures. Report anything you could not make pass.
