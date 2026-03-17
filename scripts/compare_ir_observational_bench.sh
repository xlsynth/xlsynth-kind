#!/usr/bin/env bash
set -euo pipefail

usage() {
  local exit_code="${1:-2}"
  cat <<'EOF'
usage: scripts/compare_ir_observational_bench.sh \
  --baseline-dir DIR \
  [--candidate-dir DIR] \
  [--samples slug1,slug2,...] \
  [--timeout-seconds N] \
  [--show-progress] \
  [--out-dir DIR]

Compares checked-in IR observational benchmarks across two xlsynth-kind2
worktrees or revisions. The script runs `examples/ir_observational_bench.rs`
for each requested sample, captures per-run logs, and writes a concise summary.

Defaults:
  --candidate-dir      current working tree
  --samples            batch_control,batch_tag
  --timeout-seconds    0 (no timeout)
  --out-dir            /tmp/xk2_ir_observational_compare_<timestamp>

Outputs:
  <out-dir>/summary.tsv
  <out-dir>/<label>_<slug>.log

The summary format is:
  label<TAB>repo_dir<TAB>slug<TAB>exit_code<TAB>total_wall_s<TAB>csv_row
EOF
  exit "$exit_code"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

baseline_dir=""
candidate_dir="$(pwd)"
samples_csv="batch_control,batch_tag"
timeout_seconds=0
show_progress=0
out_dir=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --baseline-dir)
      baseline_dir="${2:-}"
      shift 2
      ;;
    --candidate-dir)
      candidate_dir="${2:-}"
      shift 2
      ;;
    --samples)
      samples_csv="${2:-}"
      shift 2
      ;;
    --timeout-seconds)
      timeout_seconds="${2:-}"
      shift 2
      ;;
    --show-progress)
      show_progress=1
      shift
      ;;
    --out-dir)
      out_dir="${2:-}"
      shift 2
      ;;
    --help|-h)
      usage 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage
      ;;
  esac
done

if [[ -z "$baseline_dir" ]]; then
  echo "error: --baseline-dir is required" >&2
  usage
fi

if [[ -z "$out_dir" ]]; then
  out_dir="/tmp/xk2_ir_observational_compare_$(date +%Y%m%d_%H%M%S)"
fi

require_cmd cargo
require_cmd grep
require_cmd sed
require_cmd /usr/bin/time
if [[ "$timeout_seconds" != "0" ]]; then
  require_cmd timeout
fi

mkdir -p "$out_dir"
summary_path="$out_dir/summary.tsv"
printf 'label\trepo_dir\tslug\texit_code\ttotal_wall_s\tcsv_row\n' > "$summary_path"

IFS=',' read -r -a samples <<< "$samples_csv"
if [[ "${#samples[@]}" -eq 0 ]]; then
  echo "error: expected at least one sample in --samples" >&2
  exit 1
fi

run_one() {
  local label="$1"
  local repo_dir="$2"
  local slug="$3"
  local log_path="$out_dir/${label}_${slug}.log"
  local csv_row=""
  local total_wall=""
  local exit_code=0
  local -a cmd=(cargo run --example ir_observational_bench -- --format csv)

  if [[ "$show_progress" -eq 1 ]]; then
    cmd+=(--show-progress)
  fi
  cmd+=("$slug")

  {
    echo "label=$label"
    echo "repo_dir=$repo_dir"
    echo "slug=$slug"
    echo "cmd=${cmd[*]}"
    if [[ "$timeout_seconds" != "0" ]]; then
      (cd "$repo_dir" && /usr/bin/time -f 'TOTAL_WALL_S=%e' timeout "${timeout_seconds}s" "${cmd[@]}")
    else
      (cd "$repo_dir" && /usr/bin/time -f 'TOTAL_WALL_S=%e' "${cmd[@]}")
    fi
  } >"$log_path" 2>&1 || exit_code=$?

  total_wall="$(grep 'TOTAL_WALL_S=' "$log_path" | tail -n1 | cut -d= -f2- || true)"
  csv_row="$(grep -E "^${slug}," "$log_path" | tail -n1 || true)"

  if [[ -z "$total_wall" ]]; then
    total_wall="<missing>"
  fi
  if [[ -z "$csv_row" ]]; then
    csv_row="<missing>"
  fi

  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$label" "$repo_dir" "$slug" "$exit_code" "$total_wall" "$csv_row" >> "$summary_path"
  printf '%s %-24s exit=%s wall=%s\n' "$label" "$slug" "$exit_code" "$total_wall"
  return "$exit_code"
}

overall_exit=0
for slug in "${samples[@]}"; do
  if ! run_one baseline "$baseline_dir" "$slug"; then
    overall_exit=1
  fi
  if ! run_one candidate "$candidate_dir" "$slug"; then
    overall_exit=1
  fi
done

echo
echo "wrote summary: $summary_path"
echo "wrote logs under: $out_dir"

exit "$overall_exit"
