# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd onboard` to get started.

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest

## How to Work on a Slice (Native Compile + VFS/Remote-GC + full Rust port acceleration — spawned as part of Fresh Native 54=54 Mega Runner 1 on m934 sustain #5)

**User directive verbatim x2 everywhere**: "use more sub-agents to do more work and migrate more to rust" + "I don't care if it is going to take multiple years..." + "keep going until the entire codebase is ported to rust in the best way possible" + "proceed, do them all in parallel in the best way possible".

**Core mantra (x2 x2 in all charters/gov/headers)**: more sub-agents = more native-compile + vfs-native-cross + remote-gc + hygiene velocity + entire port accelerated.

**8-step proven methodology (shadow-first, additive, hybrid gRPC, fail-closed, Java FIRST, evidence 0%/54=54 gates, todo discipline with exactly 1 in_progress, varied parallel calls 100+ including polls/reads FIRST, sub-agents/schedulers/monitors/bg pilots for velocity)**:
1. Vision/plan/gov update (append to plan.md after relevant Remote/GC or prior block + PARITY new subsection + MIGRATION row + beads note/create child under m934/5ezk with full text + phrase + directive x2 x2 + abs paths).
2. Additive changes only (e.g. vfs-native-cross delta in native_compile.rs using DirectorySnapshot fp:1229 child_summaries + get_snapshot_delta watch:766; no behavior change, legacy preserved).
3. Rust implementation (native_compile.rs + problem_reporting.rs + build_event_stream.rs + file_fingerprint.rs + file_watch.rs for cross; reporters "native-compile"/"vfs-native-cross"/"build-events"/"problem-reporting"; BTree determinism).
4. Integration (reporters cross-slice, Java FIRST in 2 Java: RustBridgeCoreServices.java + RustSubstrateOptions.java with synthetics/exercise + ENABLE flags + exhaustive javadocs + phrase + directive).
5. Cargo + differential + corpus pilots (complete + --watch-fs + report-mismatches + full native + vfs-native-cross + remote/gc flags on native-heavy/trusted3/dogfood/manifest; target 0% on 4 reporters + 54=54 parity no mismatches on native toolchain/events + VFS delta invalidation. Extend differential for native + events/problem parity).
6. Measurements: 54=54 + 0% (evidence dirs like evidence-sustain5-native-54-54-*/ with corpus_summary 100% matches; cargo clean or E0063 hygiene fuel as momentum).
7. Docs/gov (headers in 4 .rs + 2 Java + this + plan/PARITY with all abs paths: /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/src/server/native_compile.rs etc + m934/5ezk + phrase + directive x2 x2; "How to Work on a Slice" reference).
8. Spawn more (sub-agents via scheduler_create/monitor/bg run_terminal + bd children + plan appends + this methodology recursively). "use more sub-agents..." forever until entire codebase ported to Rust best way possible. "I don't care if multiple years...". Proceed parallel.

**Absolute paths for this slice (4 .rs + watch/fp + 2 Java + plan + m934/5ezk)**: native_compile.rs, problem_reporting.rs, build_event_stream.rs, file_fingerprint.rs:1229 (DirectorySnapshot), file_watch.rs:766, RustBridgeCoreServices.java, RustSubstrateOptions.java, plan.md, gradle-fork-m934/5ezk.

**Evidence + 54=54 gate (sustain #5)**: ls build/evidence-sustain5-native* (100% parity confirmed in corpus_summary.json). Cargo E0063 hygiene fuel from richer slices. m934 OPEN. 0% on native/events/problem + VFS delta.

**Spawned**: As m934.1 child + edit here + scheduler/monitor/bg pilots during Mega Runner 1. More sub-agents = more velocity on entire port.

Follow exactly. No pause. All per user directive x2 x2. Go.
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

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
