# Aerocosm Engineering Standards

<!-- TODO: one paragraph on what aerocosm is and what correctness matters most. -->

## Project Structure

Cargo workspace:

```
aerocosm/                              (workspace root)
├── Cargo.toml                         [workspace] manifest — all dependency versions here
├── docs/plan/                         Approved plans
└── crates/
    ├── aerocosm-core/                 (library — deterministic simulation, no I/O, no UI)
    │   ├── benches/core.rs            Criterion micro-benchmarks
    │   └── src/
    │       ├── lib.rs                 Module declarations + re-exports
    │       ├── error.rs               CoreError (thiserror)
    │       ├── id.rs                  Id<T> typed table indices
    │       ├── time/                  SimTime, SimDuration, CivilDateTime (integer ms, UTC)
    │       ├── units.rs               Meters, Feet, NauticalMiles, Knots, Degrees, …
    │       ├── geo/                   LatLon, great-circle maths
    │       ├── rng.rs                 RngSeed, SimRng — per-subsystem ChaCha8 streams
    │       ├── pacing.rs              WallClock trait, SimSpeed, Pacer
    │       └── sim.rs                 Simulation, fixed 1 s tick loop
    └── aerocosm/                      (binary — headless CLI runner, tracing setup)
        └── src/
            ├── main.rs                Entry point
            ├── cli.rs                 clap argument definitions
            └── run.rs                 `run` command (batch mode)
```

**Determinism:** the core never reads real time or global randomness. Time comes from
`SimTime`/`WallClock`, randomness from a `SimRng` stream of the run's `RngSeed`. Same seed +
same inputs ⇒ same world.

**Dependency boundary:** core ← binaries. Core has no UI or binary-specific dependencies. Never reverse.

## Planning Process

**Planning is interactive, not autonomous.** When asked to plan a step or feature:

1. Research the codebase and external dependencies. Present findings.
2. **Ask questions** before finalizing the plan — surface ambiguities, trade-offs, API changes, and design decisions that need the user's input.
3. Only finalize and save the plan after the user has reviewed and approved the approach.
4. Save the final plan to `docs/plan/` before implementation begins.

Do not silently make architectural decisions. If the implementation plan document conflicts with the current codebase state (e.g., outdated dependency versions, changed APIs), flag the discrepancy and ask how to proceed.

## Quick Reference

```bash
cargo nextest run --workspace                 # All workspace tests (use nextest, not cargo test)
cargo nextest run -p aerocosm-core            # Core-only tests
cargo clippy --workspace --all-targets        # Zero warnings required (includes tests)
cargo check --workspace --all-targets         # Type-check everything including tests
cargo fmt --all -- --check                    # Format check
cargo run -p aerocosm -- run --duration 24h   # Headless batch run
cargo bench -p aerocosm-core --bench core --profile bench-unwind   # Benchmarks (see below)
```

**Always use `cargo nextest run` instead of `cargo test`.** Nextest is the project's test runner.

**Benchmarks need `--profile bench-unwind`.** The release profile uses `panic = "immediate-abort"`
with `build-std`; bench targets must unwind, and the plain bench profile then builds `core` twice
(E0152).

**Fresh cloud containers** lack the tools: install nextest with
`curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C ~/.cargo/bin`.

**`--all-targets` is mandatory** for `clippy` and `check`. Lints must pass in tests and examples — not just lib/bin targets. Test code follows the same quality standards as production code.

## Test-Driven Development

**Strict red-green-refactor. No exceptions.**

1. **Red** — Write a failing test first. Run it. Confirm it fails for the right reason.
2. **Green** — Write the minimum code to make the test pass. Nothing more.
3. **Refactor** — Clean up while all tests stay green.

### Test Requirements

| Context | Requirement |
|---------|------------|
| Every public function | At least one unit test |
| Data processing logic | Property-based tests (`proptest`, 1000+ cases) |
| Async flows | Integration test with `#[tokio::test]` + `tokio::time::timeout` |
| `thiserror` enum variants | Test verifying each variant's Display output |

### Test Conventions

- **Naming:** `test_<unit>_<scenario>` (e.g., `test_parser_rejects_empty_input`)
- **Location:** `#[cfg(test)] mod tests` inline in the source file. Integration tests in `crates/<crate>/tests/`.
- **Assertions:** `assert_eq!`, `assert!`, `prop_assert!`
- **Errors:** Tests return `anyhow::Result<()>` with `.context()` for diagnostics.
- **Quality:** Test code follows the same lint rules as production code. No `.unwrap()`, `.expect()`, or `panic!()` in tests — use `?` with `anyhow::Result` or `anyhow::bail!`.
- **Proptest config:** `#![proptest_config(ProptestConfig::with_cases(1000))]`
- **Time and side effects:** Abstract behind traits (e.g. a `Clock`) and use mocks in tests instead of real time or real I/O.

## Performance Rules

### Mandatory

- **Pre-allocate when size is known.** `String::with_capacity()`, `Vec::with_capacity()`.
- **`rust_decimal` or integer types where precision matters.** Use `f64` only where approximate values are acceptable.
- **No allocation in tight loops.**

### Avoid

- `clone()` when a borrow suffices
- `Box<dyn Trait>` when monomorphic generics work
- `String` for fixed-vocabulary identifiers — use enums or newtypes
- `Arc<Mutex<T>>` when channels or `RwLock` suffice

## Error Handling

- **Public APIs:** Return `anyhow::Result<T>`. Add `.context("msg")` or `.with_context(|| format!(...))` (lazy) on every `?`.
- **Domain errors:** `thiserror` enums for recoverable, matchable errors (`CoreError`).
- **Forbidden everywhere (including tests):** `.unwrap()`, `.expect()`, `panic!()`, `todo!()` — enforced by lints with `--all-targets`.
- **Fallible constructors:** Return `Result<Self>` not `Self`. Validate inputs at construction.

## Coding Standards

### Lints (Cargo.toml, applied to all targets)

```
unsafe_code       = "forbid"
unwrap_used       = "deny"
expect_used       = "deny"
panic             = "deny"
todo              = "deny"
print_stdout      = "warn"
print_stderr      = "warn"
clippy::pedantic  = "warn" (priority -1)
```

Every crate opts in with `[lints] workspace = true`. Run `cargo clippy --all-targets` to verify.

### Formatting (`rustfmt.toml`, nightly)

- `max_width = 100`
- `imports_granularity = "Crate"`
- `group_imports = "StdExternalCrate"`
- `wrap_comments = true`
- `format_code_in_doc_comments = true`
- Edition 2024

### Import Order

```rust
use std::...;

use anyhow::...;       // external crates
use serde::...;

use crate::...;        // local
use super::...;
```

One blank line between groups.

### Visibility

Minimum necessary. `pub(super)` or `pub(crate)` for internal types. Private fields by default.

### Module Organization

- One responsibility per file.
- Split at ~300 lines. Convert to directory module (`mod.rs` + submodules) at ~500 lines.
- Directory modules: `mod.rs` is the coordinator (struct, trait impl, re-exports). Submodules own specific concerns.

### Comments

- `///` doc comments on all public items.
- `// SAFETY:` for correctness reasoning (cancellation safety, invariant guarantees, why a conversion is sound).
- No trivial comments restating what code does. Comments explain *why*.

### Type Safety

- Domain types over raw primitives in public APIs.
- Trait abstractions for time, I/O and notifications so they can be mocked. Accept trait bounds, not concrete types.

### Dependencies

**Rule 1: Always Disable Default Features.** Always explicitly set `default-features = false` when declaring dependencies. Only enable the specific features required.

**Rule 2: Minimum Features Enabled.** Strictly limit feature opt-ins to the bare minimum required. Never use blanket features like `features = ["full"]`.

**Rule 3: Deps live in the narrowest crate.** Binary-only dependencies stay out of `aerocosm-core`. All versions are defined in the workspace root `[workspace.dependencies]`.

**General:**
- New dependencies require justification: what problem, why this crate, what alternatives were considered.
- Prefer zero-cost abstractions over convenience crates.
- No exact (`=`) version pins: every dependency goes through the upgrade workflow below.

### Upgrading Dependencies

On a feature branch (`chore/upgrade-deps`), never directly on `dev`:

```bash
cargo upgrade --dry-run -vv                   # Every dependency: current, compatible and latest version
cargo upgrade                                 # Compatible upgrades (Cargo.toml and Cargo.lock)
cargo upgrade --incompatible                  # Also major / 0.x-minor upgrades: read their changelogs first
```

Then run the full check (fmt, clippy, check, nextest, as in the Pre-Merge Checklist).

## Tracing & Observability

- **`#[instrument]`** on all public `async fn`. Use `skip(self)` or `skip_all` with explicit `fields(...)`.
- **Field syntax:** `%value` for Display, `?value` for Debug, bare primitives for Copy types.
- **Levels:** `error!` = failures requiring attention. `warn!` = degraded states. `info!` = lifecycle events. `debug!` = protocol detail.
- **Default filter:** `info,aerocosm=debug,aerocosm_core=debug`.
- **Override:** `RUST_LOG` env var via `EnvFilter::try_from_default_env()`.

## Git Workflow

No CI, no hooks. All verification is the developer's responsibility before merging.

### Branch Structure

```
main          Stable release branch. Always clean: compiles, all tests pass, zero warnings.
  └── dev     Integration branch. Features merge here first via PR. Periodically merged to main.
       ├── feature/<name>   New functionality
       ├── fix/<name>       Bug fixes
       ├── refactor/<name>  Code restructuring
       ├── test/<name>      Test additions/improvements
       └── docs/<name>      Documentation changes
```

### Branch Lifecycle

1. **Create** a feature branch from `dev`:
   ```bash
   git checkout dev
   git pull origin dev
   git checkout -b feature/<name>
   ```

2. **Work** on the branch. Make atomic commits (each compiles, passes tests, zero warnings).

3. **Push** the branch and open a PR targeting `dev`:
   ```bash
   git push -u origin feature/<name>
   ```

4. **Verify** locally before merging (no CI — you must run these):
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets
   cargo nextest run --workspace
   ```

5. **Merge** the PR into `dev` via merge commit (no squash — preserve commit history).

6. **Delete** the feature branch after merge.

7. **Promote** `dev` to `main` when a stable milestone is reached:
   ```bash
   git checkout main
   git merge dev
   git push origin main
   ```

### Branch Rules

- **Never commit directly to `main` or `dev`.** All changes go through feature branches + PRs.
- **Never force-push** to `main` or `dev`.
- **No WIP commits** on `dev` or `main`. Feature branches may have WIP commits but clean them up before PR.
- **New commits over amends** — preserve history.
- **Delete merged branches** to keep the branch list clean.

### Commit Messages

```
<type>: <imperative summary, max 72 chars>

<optional body: explains WHY, not what. Motivation, trade-offs, context.>

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`

### Pull Requests

All PRs target `dev` unless it is a `dev` → `main` promotion.

#### PR Format

```
Title: <type>: <short description> (under 72 chars)

## Summary
- Bullet points of what changed and why

## Test Plan
- [ ] Tests written first (red-green-refactor)
- [ ] cargo fmt --all -- --check passes
- [ ] cargo nextest run passes
- [ ] cargo clippy --all-targets clean
- [ ] cargo check --all-targets clean
```

#### Pre-Merge Checklist

Run locally before every merge — there is no CI to catch failures:

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets` — zero warnings
- [ ] `cargo nextest run --workspace` — all tests pass
- [ ] `cargo check --workspace --all-targets` — all targets type-check
- [ ] Tests written first (TDD evidence in commit history)
- [ ] No `.unwrap()`, `.expect()`, `panic!()`, `todo!()`
- [ ] Error paths have `.context()`
- [ ] New public items have doc comments
- [ ] Minimum visibility applied
