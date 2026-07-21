## Bug: M1 optimization caused registry to delete unchanged models

### Root cause
When `scan_with_report` skips an unchanged file (filename+filesize match), it returns `Ok(None)` and the file is not added to the records list. Then `reconcile_models` builds its `seen` set from the records list only — skipped files are absent from `seen`, so the reconcile logic treats them as "vanished from disk" and deletes their DB rows. That's why the log shows "4 removed" when the files are still on disk.

The comment I wrote on `parse_one_or_skip` was wrong: it claimed skipped files would "still be in `seen`" but they never enter the records list that `seen` is built from.

### Fix
1. **`registry.rs`**: Change `scan_with_report` to return a 3-tuple `(records, all_filenames, report)` where `all_filenames` is a `HashSet<String>` of ALL discovered filenames (both parsed AND skipped). The skipped case now inserts the filename into `all_filenames` even though it doesn't produce a `ModelRecord`.

2. **`db/registry_ops.rs`**: Change `reconcile_models` to accept `all_filenames: &HashSet<String>` and use it to build the `seen` set (instead of deriving `seen` from the records list). This way skipped files are correctly treated as "still present" — their DB rows are preserved.

3. **`lib.rs` + `commands/registry.rs`**: Update both call sites to pass the `all_filenames` set to `reconcile_models`.

4. **`registry.rs` tests**: Update all test callers of `scan_with_report` to handle the new 3-tuple return.

### Verification
- `cargo check` + `cargo test` (64 tests must pass)
- The key behavioral test: models that haven't changed must NOT be removed from the registry