#!/usr/bin/env bash
# release.sh — bump version across manifests, commit, tag, and push.
#
# Wraps tools/bump_release_version.py and creates the right tag for the
# release type. Tag taxonomy (from .github/workflows):
#   build    -> tag v<version>            : ALL CI builds, no publish
#   plugins  -> tag release/v<version>    : VS Code + JetBrains publish
#   engine   -> tag engine/v<version>     : engine release
#
# Usage:
#   tools/dev/release.sh <version> <build|plugins|engine> [--push]
#   tools/dev/release.sh 8.0.5 build            # dry: bump+commit+tag, no push
#   tools/dev/release.sh 8.0.5 plugins --push   # full: + push commit & tag
#
# Without --push, nothing is pushed (review the tag first, then push manually).
set -euo pipefail

VERSION="${1:-}"
TYPE="${2:-}"
PUSH="${3:-}"

if [ -z "$VERSION" ] || [ -z "$TYPE" ]; then
  echo "usage: release.sh <version> <build|plugins|engine> [--push]" >&2
  exit 2
fi
if ! printf '%s' "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
  echo "invalid SemVer version: $VERSION" >&2
  exit 2
fi

case "$TYPE" in
  build)   TAG="v${VERSION}" ;;
  plugins) TAG="release/v${VERSION}" ;;
  engine)  TAG="engine/v${VERSION}" ;;
  *) echo "type must be one of: build | plugins | engine" >&2; exit 2 ;;
esac

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

# Releases must happen on main, not in a worktree branch (tags point at main).
branch="$(git branch --show-current 2>/dev/null || echo '')"
if [ "$branch" != "main" ]; then
  echo "⚠ You are on '$branch', not 'main'. Releases should be cut from main." >&2
  echo "  Merge your worktree to main first, then run release.sh on main." >&2
  exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
  echo "⚠ Working tree is dirty. Commit or stash before releasing." >&2
  exit 1
fi
if git rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null; then
  echo "⚠ Tag $TAG already exists." >&2
  exit 1
fi

echo "▶ Bumping version → $VERSION"
python3 tools/bump_release_version.py "$VERSION"
notes_template="docs/release-notes/TEMPLATE.md"
notes_file="docs/release-notes/v${VERSION}.md"
if [ ! -f "$notes_file" ]; then
  if [ -f "$notes_template" ]; then
    mkdir -p "$(dirname "$notes_file")"
    sed "s/vX\.Y\.Z/v${VERSION}/g" "$notes_template" > "$notes_file"
    echo "▶ Scaffolded $notes_file from $notes_template; edit the summary and highlights before pushing the release."
  else
    echo "⚠ $notes_template is missing; create $notes_file before pushing so the release has curated notes." >&2
  fi
fi
echo
echo "▶ Changed manifests:"
git diff --stat
echo
echo "▶ Committing version bump"
git add -A
git commit -m "chore(release): v${VERSION}"
echo
echo "▶ Creating tag $TAG"
git tag -a "$TAG" -m "Release v${VERSION} ($TYPE)"

if [ "$PUSH" = "--push" ]; then
  echo "▶ Pushing main + tag $TAG"
  git push origin main
  git push origin "$TAG"
  echo
  echo "✅ Released $TAG. Triggered workflows:"
  case "$TYPE" in
    build)   echo "   - all CI builds (engine, gui, vscode, intellij)" ;;
    plugins) echo "   - VS Code publish + JetBrains release" ;;
    engine)  echo "   - engine release" ;;
  esac
  echo "   Watch:  tools/dev/ci-status.sh \$(gh run list --repo \${GH_REPO:-JegernOUTT/refact} -L1 --json databaseId --jq '.[0].databaseId')"
else
  echo
  echo "✅ Prepared $TAG locally (NOT pushed)."
  echo "   Review, then:  git push origin main && git push origin $TAG"
  echo "   Or re-run with --push."
fi
