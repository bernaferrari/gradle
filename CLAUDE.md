# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:ca08a54f -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd dolt push
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->


## Build & Test

```bash
cargo check                    # Check compilation
cargo test                     # Run all 558 tests (493 unit + 12 differential + 3 cross-language + 47 integration + 3 E2E)
cargo test --test integration_test  # Integration tests only (over Unix sockets)
cargo test --test e2e_lifecycle_test   # E2E lifecycle tests (event fan-out validation)
cargo clippy                   # Lint check (must be clean)
cargo bench                    # Run all 8 criterion benchmarks
```

## Architecture Overview

Strangler-fig migration of Gradle's execution substrate to Rust. Rust daemon (substrate/) communicates with JVM via gRPC over Unix domain sockets.

**Key files:**
- `substrate/src/main.rs` — Daemon binary, wires all 34 services
- `substrate/src/server/` — Service implementations (32 files)
- `substrate/proto/v1/substrate.proto` — gRPC service + message definitions
- `substrate/tests/` — Integration + E2E tests
- `substrate/benches/` — Criterion benchmarks
- `platforms/core-execution/rust-bridge/` — Java gRPC bridge clients
- `platforms/core-runtime/build-option/` — Feature flags (RustSubstrateOptions.java)

**Epic:** `gradle-fork-5bh` — Run `bd show gradle-fork-5bh` for full task breakdown.

**Completed phases (1-4, 7):** Core daemon, 33 service implementations, cross-service integration, testing/benchmarks, native DAG executor.
**Current phase (5):** Java bridge clients — run `bd ready` to see next tasks.

## Conventions & Patterns

- `bd` (beads) for ALL task tracking — run `bd prime` for workflow
- `Arc<DashMap<...>>` for services shared between dispatchers and tonic (DashMap::clone is a deep copy)
- Proto `int64`/`int32` map to Java `long`/`int`
- `BuildId`/`SessionId`/`TreeId`/`ProjectPath` scope newtypes in `scopes.rs`
- Integration tests use fully qualified proto clients: `foo_service_client::FooServiceClient::new(channel)`
- Shadow mode: off/shadow/on pattern per subsystem
- Feature flags via `-Dorg.gradle.rust.substrate.*` system properties
- See `~/.claude/projects/.../memory/MEMORY.md` for proto naming gotchas and known patterns
