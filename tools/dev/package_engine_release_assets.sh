#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  package_engine_release_assets.sh --version <version> --input-dir <dir> --output-dir <dir>
  package_engine_release_assets.sh --self-test
EOF
}

sha256_value() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    python3 - "$file" <<'PY'
import hashlib
import sys

with open(sys.argv[1], "rb") as handle:
    print(hashlib.sha256(handle.read()).hexdigest())
PY
  fi
}

write_checksum() {
  local archive="$1"
  local checksum="$2"
  local hash
  hash="$(sha256_value "$archive")"
  printf '%s  %s\n' "$hash" "$(basename "$archive")" > "$checksum"
}

make_zip() {
  local source="$1"
  local binary="$2"
  local archive="$3"
  if command -v zip >/dev/null 2>&1; then
    (cd "$source" && zip -q -9 -X "$archive" "$binary")
  else
    python3 - "$source/$binary" "$binary" "$archive" <<'PY'
import sys
import zipfile

source, arcname, archive = sys.argv[1:]
with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as handle:
    handle.write(source, arcname)
PY
  fi
}

package_dist_dir() {
  local dist_dir="$1"
  local version="$2"
  local output_dir="$3"
  local name target binary archive ext

  name="$(basename "$dist_dir")"
  target="${name#dist-}"
  [[ "$name" == dist-* && -n "$target" ]] || fail "invalid dist directory: $dist_dir"
  [[ "$target" =~ ^[A-Za-z0-9_.-]+$ ]] || fail "invalid target: $target"

  if [[ "$target" == *msvc ]]; then
    binary="refact.exe"
    ext="zip"
  else
    binary="refact"
    ext="tar.gz"
  fi

  [[ -f "$dist_dir/$binary" ]] || fail "missing $binary in $dist_dir"
  archive="$output_dir/refact-${version}-${target}.${ext}"
  rm -f "$archive" "$archive.sha256"

  if [[ "$ext" == "zip" ]]; then
    make_zip "$dist_dir" "$binary" "$archive"
  else
    tar -czf "$archive" -C "$dist_dir" "$binary"
  fi

  [[ -f "$archive" ]] || fail "failed to create $archive"
  write_checksum "$archive" "$archive.sha256"
  printf '%s\n' "$archive"
  printf '%s\n' "$archive.sha256"
}

find_dist_dirs() {
  local input_dir="$1"
  if [[ "$(basename "$input_dir")" == dist-* ]]; then
    printf '%s\n' "$input_dir"
    return
  fi
  find "$input_dir" -mindepth 1 -maxdepth 1 -type d -name 'dist-*' -print | sort
}

package_assets() {
  local version="$1"
  local input_dir="$2"
  local output_dir="$3"
  local count=0

  [[ "$version" =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]] || fail "invalid version: $version"
  [[ -d "$input_dir" ]] || fail "input directory does not exist: $input_dir"
  mkdir -p "$output_dir"
  output_dir="$(cd "$output_dir" && pwd -P)"

  while IFS= read -r dist_dir; do
    [[ -n "$dist_dir" ]] || continue
    package_dist_dir "$dist_dir" "$version" "$output_dir"
    count=$((count + 1))
  done < <(find_dist_dirs "$input_dir")

  [[ "$count" -gt 0 ]] || fail "no dist-* directories found in $input_dir"
}

assert_sha256() {
  local archive="$1"
  local checksum="$2"
  local expected actual
  expected="$(sha256_value "$archive")"
  actual="$(awk '{print $1}' "$checksum")"
  [[ "$actual" =~ ^[0-9a-fA-F]{64}$ ]] || fail "checksum does not contain a SHA-256 digest: $checksum"
  [[ "${actual,,}" == "$expected" ]] || fail "checksum mismatch for $archive"
}

assert_tar_root() {
  local archive="$1"
  local listing
  listing="$(tar -tzf "$archive")"
  [[ "$listing" == "refact" ]] || fail "unexpected tar layout: $listing"
}

assert_zip_root() {
  local archive="$1"
  python3 - "$archive" <<'PY'
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as handle:
    names = handle.namelist()
if names != ["refact.exe"]:
    raise SystemExit(f"unexpected zip layout: {names!r}")
PY
}

self_test() {
  local tmp input output linux_archive windows_archive
  tmp="$(mktemp -d)"
  trap "rm -rf '$tmp'" EXIT
  input="$tmp/input"
  output="$tmp/output"
  mkdir -p "$input/dist-x86_64-unknown-linux-gnu" "$input/dist-x86_64-pc-windows-msvc"
  printf 'linux refact\n' > "$input/dist-x86_64-unknown-linux-gnu/refact"
  printf 'linux lsp\n' > "$input/dist-x86_64-unknown-linux-gnu/refact-lsp"
  printf 'windows refact\n' > "$input/dist-x86_64-pc-windows-msvc/refact.exe"
  printf 'windows lsp\n' > "$input/dist-x86_64-pc-windows-msvc/refact-lsp.exe"

  package_assets "9.9.9-test" "$input" "$output" >/dev/null

  linux_archive="$output/refact-9.9.9-test-x86_64-unknown-linux-gnu.tar.gz"
  windows_archive="$output/refact-9.9.9-test-x86_64-pc-windows-msvc.zip"
  [[ -f "$linux_archive" ]] || fail "missing $linux_archive"
  [[ -f "$linux_archive.sha256" ]] || fail "missing $linux_archive.sha256"
  [[ -f "$windows_archive" ]] || fail "missing $windows_archive"
  [[ -f "$windows_archive.sha256" ]] || fail "missing $windows_archive.sha256"
  assert_tar_root "$linux_archive"
  assert_zip_root "$windows_archive"
  assert_sha256 "$linux_archive" "$linux_archive.sha256"
  assert_sha256 "$windows_archive" "$windows_archive.sha256"
}

main() {
  local version="" input_dir="" output_dir=""

  if [[ "${1:-}" == "--self-test" ]]; then
    [[ "$#" -eq 1 ]] || fail "usage: package_engine_release_assets.sh --self-test"
    self_test
    return
  fi

  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      --version)
        [[ "$#" -ge 2 ]] || fail "missing value for --version"
        version="$2"
        shift 2
        ;;
      --input-dir)
        [[ "$#" -ge 2 ]] || fail "missing value for --input-dir"
        input_dir="$2"
        shift 2
        ;;
      --output-dir)
        [[ "$#" -ge 2 ]] || fail "missing value for --output-dir"
        output_dir="$2"
        shift 2
        ;;
      -h|--help)
        usage
        return
        ;;
      *)
        fail "unknown argument: $1"
        ;;
    esac
  done

  [[ -n "$version" ]] || fail "missing --version"
  [[ -n "$input_dir" ]] || fail "missing --input-dir"
  [[ -n "$output_dir" ]] || fail "missing --output-dir"
  package_assets "$version" "$input_dir" "$output_dir"
}

main "$@"
