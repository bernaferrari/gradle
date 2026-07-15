# Agent instructions (substrate)

Contributor notes for work under `substrate/`. Product contract:
[`docs/rust-substrate-preview.md`](../docs/rust-substrate-preview.md).  
Roadmap: [`plan.md`](plan.md). Parity: [`PARITY.md`](PARITY.md).

## Defaults

- Prefer **small, evidence-backed vertical slices** over broad scaffolding.  
- **Shadow first**, then authoritative; **fail closed** on unsupported shapes.  
- Do **not** change Rust engine semantics for pure hygiene/doc cleanups.  
- Do **not** append session diaries, agent IDs, or absolute machine paths to
  `plan.md` / `PARITY.md` / `MIGRATION.md` / source headers.  
- Skip full Gradle test suites and formatters unless the task requires them.  
- Leave commits to the human/parent agent unless explicitly told to commit.

## Issue tracking (bd)

This tree may use **bd** (beads). If present:

```bash
bd ready
bd show <id>
bd update <id> --claim
bd close <id>
```

Use `bd` for task tracking when the project is set up for it. Run `bd prime`
for session workflow details when available.

## Non-interactive shell

Avoid interactive prompts:

```bash
cp -f src dst
mv -f src dst
rm -f file
rm -rf dir
```

Prefer `ssh`/`scp` BatchMode, `apt-get -y`, `HOMEBREW_NO_AUTO_UPDATE=1` when those tools appear.

## Working a slice

1. Read the preview contract and existing module tests before editing.  
2. Keep changes additive at the IR/proto/bridge boundary when possible.  
3. Wire Java bridge flags only when the Rust path is real enough to exercise.  
4. Prove with focused `cargo test` and, for ownership claims, corpus/dogfood
   authoritative runs (zero JVM forwards + required parity).  
5. Update `PARITY.md` only for durable status; point to preview doc for matrix
   detail.  
6. Do not spam docs with process mantras or multi-agent orchestration text.

## Validation (minimum)

```bash
cargo test -p gradle-substrate-daemon --lib
# when touching kernel / executors / warm path:
cargo test -p gradle-substrate-daemon execution_kernel --lib
```

Broader gates: see `PARITY.md` (corpus, dogfood, demo, stabilization).

## Session completion

When ending a session that used bd and the project expects remote sync:

1. File follow-up issues for leftover work.  
2. Run the quality gates that match what you changed.  
3. Close or update issues.  
4. Push only if the human/parent workflow requires it for this branch
   (`bd dolt push` / `git push` as applicable). Many cleanup agents are
   instructed **not** to push—follow the task contract.  
5. Hand off with paths and commands, not wall-of-text headers.

## Docs map

| Doc | Role |
| --- | --- |
| `docs/rust-substrate-preview.md` | Supported/unsupported matrix, gates |
| `docs/rust-substrate-turbopack-plan.md` | Long-term strategy |
| `docs/rust-substrate-dogfood.md` | Dogfood evidence |
| `PARITY.md` | Short parity + validate commands |
| `plan.md` | Short roadmap |
| `MIGRATION.md` | Ownership / migrate-a-slice |
| `WARM_PATH_ROADMAP.md` | Warm runbuild priorities |
| `README.md` | Architecture overview |
