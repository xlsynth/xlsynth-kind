#!/usr/bin/env bash
set -euo pipefail

usage() {
  local exit_code="${1:-2}"
  cat <<'EOF'
usage: scripts/compare_ir_observational_equiv.sh \
  --baseline-dir DIR \
  [--candidate-dir DIR] \
  [--timeout-seconds N] \
  [--show-progress] \
  [--out-dir DIR] \
  -- <ir_observational_equiv args...>

Runs `examples/ir_observational_equiv.rs` with the same arguments in both
xlsynth-kind2 worktrees and records each result and wall time.

Defaults:
  --candidate-dir      current working tree
  --timeout-seconds    0 (no timeout)
  --out-dir            /tmp/xk2_ir_observational_equiv_compare_<timestamp>

Outputs:
  <out-dir>/summary.tsv
  <out-dir>/<label>.log

The summary format is:
  label<TAB>repo_dir<TAB>exit_code<TAB>total_wall_s<TAB>final_result<TAB>final_wall_s<TAB>init_result<TAB>init_wall_s<TAB>step_result<TAB>step_wall_s
EOF
  exit "$exit_code"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

extract_field_after_colon() {
  local pattern="$1"
  local log_path="$2"
  grep -E "$pattern" "$log_path" | tail -n1 | cut -d: -f2- | sed 's/^ //' || true
}

baseline_dir=""
candidate_dir="$(pwd)"
timeout_seconds=0
show_progress=0
out_dir=""
proof_args=()

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
    --)
      shift
      proof_args=("$@")
      break
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
if [[ "${#proof_args[@]}" -eq 0 ]]; then
  echo "error: expected proof args after --" >&2
  usage
fi

if [[ -z "$out_dir" ]]; then
  out_dir="/tmp/xk2_ir_observational_equiv_compare_$(date +%Y%m%d_%H%M%S)"
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
printf 'label\trepo_dir\texit_code\ttotal_wall_s\tfinal_result\tfinal_wall_s\tinit_result\tinit_wall_s\tstep_result\tstep_wall_s\n' > "$summary_path"

run_one() {
  local label="$1"
  local repo_dir="$2"
  local log_path="$out_dir/${label}.log"
  local exit_code=0
  local total_wall=""
  local final_result=""
  local final_wall=""
  local init_result=""
  local init_wall=""
  local step_result=""
  local step_wall=""
  local -a cmd=(cargo run --example ir_observational_equiv --)

  if [[ "$show_progress" -eq 1 ]]; then
    cmd+=(--show-progress)
  fi
  cmd+=("${proof_args[@]}")

  {
    echo "label=$label"
    echo "repo_dir=$repo_dir"
    printf 'cmd='
    printf '%q ' "${cmd[@]}"
    echo
    if [[ "$timeout_seconds" != "0" ]]; then
      (cd "$repo_dir" && /usr/bin/time -f 'TOTAL_WALL_S=%e' timeout "${timeout_seconds}s" "${cmd[@]}")
    else
      (cd "$repo_dir" && /usr/bin/time -f 'TOTAL_WALL_S=%e' "${cmd[@]}")
    fi
  } >"$log_path" 2>&1 || exit_code=$?

  total_wall="$(grep '^TOTAL_WALL_S=' "$log_path" | tail -n1 | cut -d= -f2- || true)"
  final_result="$(extract_field_after_colon '^(Kind2 result|relation result|step obligation result):' "$log_path")"
  final_wall="$(extract_field_after_colon '^(Kind2 wall time|relation wall time|step obligation wall time):' "$log_path")"
  init_result="$(extract_field_after_colon '^init .* result:' "$log_path")"
  init_wall="$(extract_field_after_colon '^init .* wall time:' "$log_path")"
  step_result="$(extract_field_after_colon '^step obligation result:' "$log_path")"
  step_wall="$(extract_field_after_colon '^step obligation wall time:' "$log_path")"

  [[ -n "$total_wall" ]] || total_wall="<missing>"
  [[ -n "$final_result" ]] || final_result="<missing>"
  [[ -n "$final_wall" ]] || final_wall="<missing>"
  [[ -n "$init_result" ]] || init_result="<missing>"
  [[ -n "$init_wall" ]] || init_wall="<missing>"
  [[ -n "$step_result" ]] || step_result="<missing>"
  [[ -n "$step_wall" ]] || step_wall="<missing>"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$label" "$repo_dir" "$exit_code" "$total_wall" "$final_result" "$final_wall" \
    "$init_result" "$init_wall" "$step_result" "$step_wall" >> "$summary_path"
  printf '%s exit=%s total=%s init=%s/%s step=%s/%s\n' \
    "$label" "$exit_code" "$total_wall" "$init_result" "$init_wall" "$step_result" "$step_wall"
  return "$exit_code"
}

overall_exit=0
if ! run_one baseline "$baseline_dir"; then
  overall_exit=1
fi
if ! run_one candidate "$candidate_dir"; then
  overall_exit=1
fi

echo
echo "wrote summary: $summary_path"
echo "wrote logs under: $out_dir"

exit "$overall_exit"
