# AGENTS.md — tunes4r_player

## Git Rules
- **Never** run any form of `git checkout`, `git restore`, or `git checkout -- <file>` to discard, restore, or revert uncommitted work. This destroys uncommitted changes without recovery.
- **Never** use git to restore lost code. Lost code must be re-implemented manually.
- Always ask before using any destructive git operation: `git checkout` (any form), `git restore`, `git reset`, `git revert`, `git commit --amend`, `git push --force`, `git clean`, `git stash drop`.
- Only `git status`, `git diff`, `git log` are safe to use without asking.
- "Destructive" means anything that changes history or discards uncommitted work.
- If an edit tool fails repeatedly, do NOT reach for `git checkout` as a shortcut — stop and ask the user how to proceed.

## Verification
- Run `cargo test -p tunes4r-core --lib` (122 tests) and `cargo test --test ffi_contract` after Rust changes.
- Rebuild macOS dylib with `./scripts/build_rust.sh macos` when engine logic changes.
