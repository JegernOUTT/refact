#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$repo_root"

readonly KOTLIN_RESOLVER="plugins/intellij/src/main/kotlin/com/smallcloud/refactai/lsp/RefactBinaryResolver.kt"
readonly VSCODE_RESOLVER="plugins/vscode/src/refactBinaryResolver.ts"
readonly ENGINE_BUILD_WORKFLOW=".github/workflows/agent_engine_build.yml"
readonly TARGET_RE='[A-Za-z0-9_]+-[A-Za-z0-9_]+-[A-Za-z0-9_]+(-[A-Za-z0-9_]+)?'
readonly ALLOWLISTED_ADVERTISED_UNBUILT_TARGETS=("x86_64-apple-darwin")

allowlist_reason() {
  case "$1" in
    x86_64-apple-darwin) printf 'Intel macOS auto-download unsupported; set refactai.binaryPath or use a canonical engine/v* release' ;;
    *) printf 'advertised-but-not-plugin-built target' ;;
  esac
}

failures=()

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

record_failure() {
  failures+=("$1")
}

join_file() {
  local file="$1"
  if [[ ! -s "$file" ]]; then
    printf '<none>'
    return
  fi
  paste -sd, "$file" | sed 's/,/, /g'
}

extract_function_targets() {
  local file="$1"
  local start_pattern="$2"
  [[ -f "$file" ]] || { echo "missing file: $file" >&2; return 1; }
  awk -v start_pattern="$start_pattern" '
    $0 ~ start_pattern { inside = 1 }
    inside { print }
    inside && $0 ~ /^}/ { exit }
  ' "$file" | { grep -Eo "$TARGET_RE" || true; } | sort -u
}

extract_engine_matrix_targets() {
  [[ -f "$ENGINE_BUILD_WORKFLOW" ]] || { echo "missing file: $ENGINE_BUILD_WORKFLOW" >&2; return 1; }
  sed -nE "s/^[[:space:]]+target:[[:space:]]*($TARGET_RE)[[:space:]]*$/\1/p" "$ENGINE_BUILD_WORKFLOW" | sort -u
}

write_allowlist() {
  printf '%s\n' "${ALLOWLISTED_ADVERTISED_UNBUILT_TARGETS[@]}" | sort -u
}

require_non_empty() {
  local file="$1"
  local label="$2"
  if [[ ! -s "$file" ]]; then
    record_failure "$label target set is empty"
  fi
}

extract_function_targets "$KOTLIN_RESOLVER" "internal fun refactReleaseTarget" > "$tmp_dir/kotlin"
extract_function_targets "$VSCODE_RESOLVER" "export function refactReleaseTarget" > "$tmp_dir/vscode"
extract_engine_matrix_targets > "$tmp_dir/matrix"
write_allowlist > "$tmp_dir/allowlist"
sort -u "$tmp_dir/kotlin" "$tmp_dir/vscode" > "$tmp_dir/resolver_union"

require_non_empty "$tmp_dir/kotlin" "Kotlin resolver"
require_non_empty "$tmp_dir/vscode" "VS Code resolver"
require_non_empty "$tmp_dir/matrix" "engine build matrix"

if ! cmp -s "$tmp_dir/kotlin" "$tmp_dir/vscode"; then
  record_failure "Kotlin and VS Code resolver targets differ. Kotlin: $(join_file "$tmp_dir/kotlin"). VS Code: $(join_file "$tmp_dir/vscode")."
fi

comm -23 "$tmp_dir/resolver_union" "$tmp_dir/matrix" > "$tmp_dir/advertised_not_built"
comm -23 "$tmp_dir/advertised_not_built" "$tmp_dir/allowlist" > "$tmp_dir/unallowlisted_advertised_not_built"
comm -23 "$tmp_dir/matrix" "$tmp_dir/kotlin" > "$tmp_dir/matrix_not_kotlin"
comm -23 "$tmp_dir/matrix" "$tmp_dir/vscode" > "$tmp_dir/matrix_not_vscode"
comm -12 "$tmp_dir/advertised_not_built" "$tmp_dir/allowlist" > "$tmp_dir/allowlisted_gap"
comm -23 "$tmp_dir/allowlist" "$tmp_dir/advertised_not_built" > "$tmp_dir/stale_allowlist"

if [[ -s "$tmp_dir/unallowlisted_advertised_not_built" ]]; then
  record_failure "Resolver targets are advertised but not built by the engine matrix or allowlisted: $(join_file "$tmp_dir/unallowlisted_advertised_not_built")."
fi

if [[ -s "$tmp_dir/matrix_not_kotlin" ]]; then
  record_failure "Engine matrix targets are not advertised by the Kotlin resolver: $(join_file "$tmp_dir/matrix_not_kotlin")."
fi

if [[ -s "$tmp_dir/matrix_not_vscode" ]]; then
  record_failure "Engine matrix targets are not advertised by the VS Code resolver: $(join_file "$tmp_dir/matrix_not_vscode")."
fi

if [[ -s "$tmp_dir/stale_allowlist" ]]; then
  record_failure "Allowlisted targets are no longer advertised-but-not-built gaps; update the allowlist: $(join_file "$tmp_dir/stale_allowlist")."
fi

if (( ${#failures[@]} > 0 )); then
  echo "❌ Resolver target parity check failed:" >&2
  for failure in "${failures[@]}"; do
    printf '\n%s\n' "$failure" >&2
  done
  exit 1
fi

echo "✅ Resolver target parity check passed."
echo "Resolver targets: $(join_file "$tmp_dir/resolver_union")"
echo "Engine build matrix targets: $(join_file "$tmp_dir/matrix")"
if [[ -s "$tmp_dir/allowlisted_gap" ]]; then
  echo "Allowlisted advertised-but-not-plugin-built targets:"
  while IFS= read -r target; do
    echo "  - $target: $(allowlist_reason "$target")"
  done < "$tmp_dir/allowlisted_gap"
else
  echo "Allowlisted advertised-but-not-plugin-built targets: none"
fi
