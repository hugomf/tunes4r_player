# AGENTS.md — tunes4r_player

## Git Rules
- **Never** run `git checkout -- <file>` (or `git checkout <file>`) to discard uncommitted work. This destroys uncommitted changes without recovery.
- Always ask before using any destructive git operation: `git checkout` (any form), `git reset`, `git revert`, `git commit --amend`, `git push --force`, `git clean`, `git stash drop`.
- Only `git status`, `git diff`, `git log` are safe to use without asking.
- "Destructive" means anything that changes history or discards uncommitted work.
- If an edit tool fails repeatedly, do NOT reach for `git checkout` as a shortcut — stop and ask the user how to proceed.

## Verification
- Run `cargo test -p tunes4r-core --lib` (122 tests) and `cargo test --test ffi_contract` after Rust changes.
- Rebuild macOS dylib with `./scripts/build_rust.sh macos` when engine logic changes.
