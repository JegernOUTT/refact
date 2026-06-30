#!/usr/bin/env bash
set -euo pipefail

ENGINE_VERSION_RE='^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'
NON_EAP_VERSION_RE='^[0-9]+\.[0-9]+\.[0-9]+-[0-9A-Za-z.-]+$'

fail() {
  echo "$1" >&2
  exit 1
}

git_output() {
  local out
  out="$(git "$@" 2>/dev/null || true)"
  printf '%s' "$out"
}

git_output_or_null() {
  local out
  out="$(git "$@" 2>/dev/null || true)"
  [[ -n "$out" ]] || return 1
  printf '%s' "$out"
}

repo_root() {
  git rev-parse --show-toplevel 2>/dev/null || pwd
}

read_base_version() {
  local root
  root="$(repo_root)"
  awk -F '=' '
    /^[[:space:]]*pluginVersion[[:space:]]*=/ {
      value = $2
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
      print value
      exit
    }
  ' "$root/plugins/intellij/gradle.properties"
}

compute_version() {
  local base_version="$1"
  local release_versions release_count release_tag_version branch number_of_commits commit_id version
  release_versions="$(git_output tag -l --points-at HEAD | sed -nE 's#^release/v([0-9]+\.[0-9]+\.[0-9]+)$#\1#p')"
  release_count="$(printf '%s\n' "$release_versions" | sed '/^$/d' | wc -l | tr -d ' ')"
  release_tag_version=""
  if [[ "$release_count" == "1" ]]; then
    release_tag_version="$(printf '%s\n' "$release_versions" | sed '/^$/d')"
  fi

  if [[ "${PUBLISH_EAP:-}" != "1" && "$release_tag_version" == "$base_version" ]]; then
    printf '%s\n' "$base_version"
    return
  fi

  if [[ -n "${GITHUB_REF_NAME:-}" ]]; then
    branch="$GITHUB_REF_NAME"
  else
    branch="$(git_output rev-parse --abbrev-ref HEAD)"
  fi
  if [[ -z "$branch" ]]; then
    branch="unknown"
  fi
  branch="${branch//\//-}"

  if [[ "$branch" == "main" ]]; then
    local last_tag
    if last_tag="$(git_output_or_null describe --tags --abbrev=0 --match 'v*' @^)"; then
      number_of_commits="$(git_output rev-list "${last_tag}..HEAD" --count)"
    else
      number_of_commits="$(git_output rev-list --count HEAD)"
    fi
  else
    if number_of_commits="$(git_output_or_null rev-list --count HEAD ^origin/main)"; then
      :
    else
      number_of_commits="$(git_output rev-list --count HEAD)"
    fi
  fi

  commit_id="$(git_output rev-parse --short=8 HEAD)"
  if [[ "${PUBLISH_EAP:-}" == "1" ]]; then
    version="$base_version.$number_of_commits-eap-$commit_id"
  else
    version="$base_version-$branch-$number_of_commits-$commit_id"
    if [[ ! "$version" =~ $ENGINE_VERSION_RE ]]; then
      fail "computed version does not match engine release version regex: $version"
    fi
  fi
  printf '%s\n' "$version"
}

self_test() {
  local tmp release_version non_release_version eap_version
  tmp="$(mktemp -d)"
  trap "rm -rf '$tmp'" EXIT
  git init -q -b main "$tmp"
  cd "$tmp"
  git config user.email refact-self-test@example.invalid
  git config user.name refact-self-test
  printf 'one\n' > file.txt
  git add file.txt
  git commit -q -m initial
  git tag release/v1.2.3
  git tag v1.2.3

  release_version="$(PUBLISH_EAP=0 compute_version 1.2.3)"
  [[ "$release_version" == "1.2.3" ]] || fail "release tag self-test failed: $release_version"

  printf 'two\n' >> file.txt
  git commit -q -am second
  git tag engine/v1.2.3-main-1-deadbeef

  printf 'three\n' >> file.txt
  git commit -q -am third
  git tag release/v1.2.4

  printf 'four\n' >> file.txt
  git commit -q -am fourth
  non_release_version="$(PUBLISH_EAP=0 compute_version 1.2.3)"
  [[ "$non_release_version" == 1.2.3-main-3-* ]] || fail "non-release self-test failed: $non_release_version"
  [[ "$non_release_version" =~ $NON_EAP_VERSION_RE ]] || fail "non-release format self-test failed: $non_release_version"

  eap_version="$(PUBLISH_EAP=1 compute_version 1.2.3)"
  [[ "$eap_version" == 1.2.3.3-eap-* ]] || fail "EAP count self-test failed: $eap_version"
  [[ "$eap_version" =~ ^1\.2\.3\.[0-9]+-eap-[0-9a-f]{8}$ ]] || fail "EAP self-test failed: $eap_version"
}

main() {
  if [[ "${1:-}" == "--self-test" ]]; then
    [[ $# -eq 1 ]] || fail "usage: compute_build_version.sh [base_version]"
    self_test
    return
  fi

  [[ $# -le 1 ]] || fail "usage: compute_build_version.sh [base_version]"
  local base_version
  base_version="${1:-}"
  if [[ -z "$base_version" ]]; then
    base_version="$(read_base_version)"
  fi
  [[ -n "$base_version" ]] || fail "could not determine base version"
  compute_version "$base_version"
}

main "$@"
