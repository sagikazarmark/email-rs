# AGENTS.md

## Conventions

- Structure Rust source files with the primary public trait or struct first, then impls, then supporting types, then internal helpers, then tests.
- Prefer `foo.rs` + `foo/` over `foo/mod.rs`.
- Keep dependency entries in `Cargo.toml` alphabetized within each table.
