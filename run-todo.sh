#!/usr/bin/env bash
set -euo pipefail

TODO_DIR="todo"
RESULTS_DIR="results"

TOOL="${1:-claude}"

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Error: $cmd CLI is not available in PATH" >&2
    exit 1
  fi
}

case "$TOOL" in
  claude)
    require_command "claude"
    ;;
  codex)
    require_command "codex"
    ;;
  *)
    echo "Usage: $0 [claude|codex]" >&2
    exit 1
    ;;
esac

if ! command -v git >/dev/null 2>&1; then
  echo "Error: git is not available in PATH" >&2
  exit 1
fi

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Error: run-todo.sh must be executed inside a git repository" >&2
  exit 1
fi

run_prompt() {
  local prompt="$1"
  case "$TOOL" in
    claude)
      claude --dangerously-skip-permissions -p "$prompt"
      ;;
    codex)
      codex exec --dangerously-bypass-approvals-and-sandbox "$prompt"
      ;;
  esac
}

mkdir -p "$RESULTS_DIR"

shopt -s nullglob
task_files=("$TODO_DIR"/task-*.md)

if [ ${#task_files[@]} -eq 0 ]; then
  echo "No task-<number>.md files found in $TODO_DIR" >&2
  exit 0
fi

for task_file in "${task_files[@]}"; do
  filename=$(basename "$task_file")
  identifier="${filename#task-}"
  identifier="${identifier%.md}"
  result_file="$RESULTS_DIR/result-$identifier.md"

  printf 'Processing %s -> %s\n' "$filename" "$(basename "$result_file")"

  prompt_content=$(<"$task_file")

  run_prompt "$prompt_content" > "$result_file"

  git add -A

  if git diff --cached --quiet; then
    echo "No changes detected for $filename; skipping commit."
    continue
  fi

  git commit -m "task-$identifier"
  echo "Committed changes for task-$identifier."
done

echo "All tasks processed. Results saved in $RESULTS_DIR."
