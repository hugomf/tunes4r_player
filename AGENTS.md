# AGENTS.md — tunes4r_player

## Git Rules
- Always ask before using any destructive git operation: `git checkout`, `git reset`, `git revert`, `git commit --amend`, `git push --force`, `git clean`, `git stash drop`.
- Only `git status`, `git diff`, `git log` are safe to use without asking.
- "Destructive" means anything that changes history or discards uncommitted work.

## Verification
- Run `cargo test -p tunes4r-core --lib` (122 tests) and `cargo test --test ffi_contract` after Rust changes.
- Rebuild macOS dylib with `./scripts/build_rust.sh macos` when engine logic changes.
