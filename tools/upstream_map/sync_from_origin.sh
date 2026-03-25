#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"

TARGET="${1:-origin/master}"
STABILIZATION_MODE="${2:-quick}"
if [[ "$STABILIZATION_MODE" != "quick" && "$STABILIZATION_MODE" != "full" ]]; then
  echo "Usage: $0 [target-ref] [quick|full]"
  exit 2
fi

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" == "HEAD" ]]; then
  echo "Detached HEAD is not supported. Checkout a branch first."
  exit 2
fi

CURRENT_HEAD="$(git rev-parse HEAD)"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
BACKUP_REF="refs/heads/backup/pre-sync-${BRANCH}-${TIMESTAMP}"
BEAD_ID=""

start_bead() {
  if ! command -v bd >/dev/null 2>&1; then
    return
  fi
  if [[ "${BD_TRACK:-1}" == "0" ]]; then
    return
  fi
  BEAD_ID="$(
    bd create \
      --title "upstream sync ${BRANCH} -> ${TARGET} (${TIMESTAMP})" \
      -t chore \
      -p 2 \
      --labels upstream-sync,automation \
      --silent 2>/dev/null || true
  )"
}

note_bead() {
  if [[ -n "$BEAD_ID" ]]; then
    bd note "$BEAD_ID" "$1" >/dev/null 2>&1 || true
  fi
}

close_bead_success() {
  if [[ -n "$BEAD_ID" ]]; then
    bd close "$BEAD_ID" --reason "completed" >/dev/null 2>&1 || true
  fi
}

start_bead

STASHED=0
if ! git diff --quiet || ! git diff --cached --quiet || [[ -n "$(git ls-files --others --exclude-standard)" ]]; then
  echo "Local changes detected, creating stash snapshot..."
  git stash push -u -m "pre-sync-${BRANCH}-${TIMESTAMP}" >/dev/null
  STASHED=1
  note_bead "Local dirty state detected and stashed before sync."
fi

cleanup_stash() {
  if [[ "$STASHED" -eq 1 ]]; then
    if ! git stash pop; then
      echo "Stash restore had conflicts. Resolve manually."
      note_bead "Stash restore failed with conflicts; manual resolution required."
      exit 1
    fi
  fi
}

rollback() {
  echo "Rolling back branch to pre-sync head..."
  note_bead "Rollback triggered. Resetting branch to pre-sync head ${CURRENT_HEAD}."
  git rebase --abort >/dev/null 2>&1 || true
  git reset --hard "$CURRENT_HEAD" >/dev/null
  cleanup_stash
  echo "Rollback complete."
}

echo "Creating backup ref: ${BACKUP_REF}"
git update-ref "$BACKUP_REF" "$CURRENT_HEAD"
note_bead "Created backup ref ${BACKUP_REF} at ${CURRENT_HEAD}."

echo "Fetching origin..."
git fetch origin --prune
note_bead "Fetched origin with prune."

echo "Rebasing ${BRANCH} onto ${TARGET}..."
if ! git rebase "$TARGET"; then
  rollback
  note_bead "Rebase failed for ${BRANCH} onto ${TARGET}."
  echo "Rebase failed."
  exit 1
fi
note_bead "Rebase completed successfully."

echo "Running upstream drift check..."
if ! ./tools/upstream_map/check_drift.sh; then
  rollback
  note_bead "Upstream drift check failed."
  echo "Drift checks failed."
  exit 1
fi
note_bead "Upstream drift check passed."

echo "Running strict stabilization (${STABILIZATION_MODE})..."
if ! ./tools/stabilization/run_strict_stabilization.sh "$STABILIZATION_MODE"; then
  rollback
  note_bead "Strict stabilization (${STABILIZATION_MODE}) failed."
  echo "Stabilization checks failed."
  exit 1
fi
note_bead "Strict stabilization (${STABILIZATION_MODE}) passed."

cleanup_stash

echo "Sync completed successfully."
echo "Backup ref retained at: ${BACKUP_REF}"
note_bead "Sync completed successfully."
close_bead_success
