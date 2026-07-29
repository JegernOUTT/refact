#!/usr/bin/env bash
if [ -n "${BASH_VERSION:-}" ]; then
    set -euo pipefail
else
    set -eu
fi

repo_url="https://github.com/JegernOUTT/refact"
api_url="https://api.github.com/repos/JegernOUTT/refact"
version="${VERSION:-}"
binary_path=""
modify_path=1
install_dir="${HOME:-}/.refact/bin"
modified_path_files=""
path_update_warning=0

usage() {
    cat <<'EOF'
Install Refact into ~/.refact/bin.

Usage:
  install.sh [--version <v>] [--binary <path>] [--no-modify-path]

Options:
  --version <v>       Install a specific Refact version. VERSION env is also honored.
  --binary <path>     Install from a local refact binary instead of GitHub Releases.
  --no-modify-path    Do not add ~/.refact/bin to shell startup files.
  -h, --help          Show this help.
EOF
}

info() {
    printf '%s\n' "$*"
}

err() {
    printf 'error: %s\n' "$*" >&2
}

fail() {
    err "$*"
    exit 1
}

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "missing required command: $1"
    fi
}

normalize_version() {
    value=$1
    case "$value" in
        engine/v*) value=${value#engine/v} ;;
        engine/*) value=${value#engine/} ;;
        release/v*) value=${value#release/v} ;;
        v*) value=${value#v} ;;
    esac
    if [ -z "$value" ]; then
        fail "version is empty"
    fi
    printf '%s\n' "$value"
}

latest_version() {
    need_cmd curl
    need_cmd tr
    releases_url="$api_url/releases?per_page=100"
    releases_json=$(curl -fsSL -H 'Accept: application/vnd.github+json' "$releases_url") || fail "could not fetch releases from $releases_url"
    tag=""
    for candidate in $(printf '%s\n' "$releases_json" | tr ',' '\n' | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'); do
        case "$candidate" in
            engine/v*)
                tag=$candidate
                break
                ;;
        esac
    done
    if [ -z "$tag" ]; then
        fail "could not find an engine/v* release in $releases_url"
    fi
    normalize_version "$tag"
}

resolve_version() {
    if [ -z "$version" ] || [ "$version" = "latest" ]; then
        latest_version
    else
        normalize_version "$version"
    fi
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            version=$2
            shift 2
            ;;
        --version=*)
            version=${1#--version=}
            shift
            ;;
        --binary)
            [ "$#" -ge 2 ] || fail "--binary requires a path"
            binary_path=$2
            shift 2
            ;;
        --binary=*)
            binary_path=${1#--binary=}
            shift
            ;;
        --no-modify-path)
            modify_path=0
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

if [ -z "${HOME:-}" ]; then
    fail "HOME is not set"
fi

os_name=$(uname -s 2>/dev/null || printf unknown)
arch_name=$(uname -m 2>/dev/null || printf unknown)

case "$os_name" in
    Darwin) os=darwin ;;
    Linux) os=linux ;;
    MINGW*|MSYS*|CYGWIN*) os=windows ;;
    *) fail "unsupported operating system: $os_name" ;;
esac

case "$arch_name" in
    x86_64|amd64) arch=x86_64 ;;
    arm64|aarch64) arch=aarch64 ;;
    i386|i686) arch=x86 ;;
    *) fail "unsupported CPU architecture: $arch_name" ;;
esac

case "$os:$arch" in
    linux:x86_64) target="x86_64-unknown-linux-gnu" ;;
    linux:aarch64) target="aarch64-unknown-linux-gnu" ;;
    darwin:x86_64) target="x86_64-apple-darwin" ;;
    darwin:aarch64) target="aarch64-apple-darwin" ;;
    windows:x86_64) target="x86_64-pc-windows-msvc" ;;
    windows:x86) target="i686-pc-windows-msvc" ;;
    windows:aarch64) target="aarch64-pc-windows-msvc" ;;
    *) fail "unsupported platform: $os_name $arch_name" ;;
esac

if [ "$os" = "windows" ]; then
    archive_ext=".zip"
    executable_name="refact.exe"
else
    archive_ext=".tar.gz"
    executable_name="refact"
fi

install_path="$install_dir/$executable_name"

tmp_dir=""
cleanup() {
    if [ -n "$tmp_dir" ] && [ -d "$tmp_dir" ]; then
        rm -rf "$tmp_dir"
    fi
}
trap cleanup EXIT INT TERM

compute_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print tolower($1)}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print tolower($1)}'
    else
        fail "missing required command: sha256sum or shasum"
    fi
}

verify_sha256() {
    archive=$1
    checksum=$2
    if [ ! -s "$checksum" ]; then
        fail "checksum file is empty: $checksum"
    fi
    IFS=' ' read -r expected _ < "$checksum" || fail "could not read checksum file: $checksum"
    expected=$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]' | tr -d '\r')
    actual=$(compute_sha256 "$archive")
    if [ "$expected" != "$actual" ]; then
        fail "sha256 mismatch for $(basename "$archive")"
    fi
}

install_binary() {
    source_path=$1
    if [ ! -f "$source_path" ]; then
        fail "binary not found: $source_path"
    fi
    mkdir -p "$install_dir"
    temp_target="$install_path.tmp.$$"
    cp "$source_path" "$temp_target"
    chmod 755 "$temp_target" 2>/dev/null || true
    mv "$temp_target" "$install_path"
}

block_begin_marker='# >>> Refact installer >>>'
block_end_marker='# <<< Refact installer <<<'

select_profile() {
    profile_shell=$(basename "${SHELL:-sh}")
    case "$profile_shell" in
        fish)
            printf '%s\n' "${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish"
            return 0
            ;;
        zsh)
            if [ "$os" = "darwin" ]; then
                printf '%s\n' "$HOME/.zprofile"
            else
                printf '%s\n' "$HOME/.zshrc"
            fi
            return 0
            ;;
        bash)
            if [ "$os" = "darwin" ]; then
                printf '%s\n' "$HOME/.bash_profile"
            else
                printf '%s\n' "$HOME/.bashrc"
            fi
            return 0
            ;;
        *)
            printf '%s\n' "$HOME/.profile"
            return 0
            ;;
    esac
}

upsert_path_block() {
    profile_file=$1
    body_line=$2
    profile_dir=$(dirname "$profile_file")
    mkdir -p "$profile_dir"

    begin_count=0
    end_count=0
    if [ -f "$profile_file" ]; then
        begin_count=$(awk -v marker="$block_begin_marker" '{ sub(/\r$/, "") } $0 == marker { count++ } END { print count + 0 }' "$profile_file")
        end_count=$(awk -v marker="$block_end_marker" '{ sub(/\r$/, "") } $0 == marker { count++ } END { print count + 0 }' "$profile_file")
    fi

    if [ "$begin_count" -ne 0 ] || [ "$end_count" -ne 0 ]; then
        if [ "$begin_count" -ne 1 ] || [ "$end_count" -ne 1 ]; then
            err "found malformed Refact installer markers in $profile_file; leaving it untouched"
            path_update_warning=1
            return 0
        fi
        begin_line=$(awk -v marker="$block_begin_marker" '{ sub(/\r$/, "") } $0 == marker { print NR; exit }' "$profile_file")
        end_line=$(awk -v marker="$block_end_marker" '{ sub(/\r$/, "") } $0 == marker { print NR; exit }' "$profile_file")
        if [ "$end_line" -le "$begin_line" ]; then
            err "found reversed Refact installer markers in $profile_file; leaving it untouched"
            path_update_warning=1
            return 0
        fi
    fi

    if [ "$begin_count" -ne "$end_count" ]; then
        err "found an unbalanced Refact installer block in $profile_file; leaving it untouched"
        path_update_warning=1
        return 0
    fi

    desired_block=$(printf '%s\n%s\n%s\n' "$block_begin_marker" "$body_line" "$block_end_marker")

    if [ "$begin_count" -eq 1 ]; then
        current_block=$(awk -v b="$block_begin_marker" -v e="$block_end_marker" '
            { sub(/\r$/, "") }
            $0 == b { capture = 1 }
            capture == 1 { print }
            capture == 1 && $0 == e { exit }
        ' "$profile_file")
        if [ "$current_block" = "$desired_block" ]; then
            return 0
        fi
        tmp_profile="$profile_file.tmp.$$"
        awk -v b="$block_begin_marker" -v e="$block_end_marker" -v line="$body_line" '
            { normalized = $0; sub(/\r$/, "", normalized) }
            normalized == b { skip = 1; print b; print line; print e; next }
            normalized == e { skip = 0; next }
            skip == 1 { next }
            { print }
        ' "$profile_file" > "$tmp_profile"
        mv "$tmp_profile" "$profile_file"
        modified_path_files="$modified_path_files${modified_path_files:+ }$profile_file"
        return 0
    fi

    if [ -f "$profile_file" ] && [ -s "$profile_file" ]; then
        printf '\n%s\n' "$desired_block" >> "$profile_file"
    else
        printf '%s\n' "$desired_block" >> "$profile_file"
    fi
    modified_path_files="$modified_path_files${modified_path_files:+ }$profile_file"
}

update_shell_path() {
    if [ "$modify_path" -eq 0 ]; then
        return 0
    fi

    export_line='export PATH="$HOME/.refact/bin:$PATH"'
    fish_line='fish_add_path "$HOME/.refact/bin"'
    profile_file=$(select_profile)
    profile_shell=$(basename "${SHELL:-sh}")

    case "$profile_shell" in
        fish) upsert_path_block "$profile_file" "$fish_line" ;;
        *) upsert_path_block "$profile_file" "$export_line" ;;
    esac
}

install_from_release() {
    resolved_version=$(resolve_version)
    archive_name="refact-$resolved_version-$target$archive_ext"
    release_base="$repo_url/releases/download/engine/v$resolved_version"
    archive_url="$release_base/$archive_name"
    checksum_url="$archive_url.sha256"

    if [ "$archive_ext" = ".zip" ]; then
        need_cmd unzip
    else
        need_cmd tar
    fi
    need_cmd curl
    need_cmd awk
    need_cmd tr

    tmp_dir=$(mktemp -d 2>/dev/null || mktemp -d -t refact-install)
    archive_path="$tmp_dir/$archive_name"
    checksum_path="$tmp_dir/$archive_name.sha256"
    extract_dir="$tmp_dir/extract"
    mkdir -p "$extract_dir"

    info "Downloading $archive_url"
    curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$archive_path" "$archive_url"
    curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$checksum_path" "$checksum_url"
    verify_sha256 "$archive_path" "$checksum_path"

    if [ "$archive_ext" = ".zip" ]; then
        unzip -q -o "$archive_path" -d "$extract_dir"
    else
        tar -xzf "$archive_path" -C "$extract_dir"
    fi

    binary_source=""
    if [ -f "$extract_dir/$executable_name" ]; then
        binary_source="$extract_dir/$executable_name"
    else
        for candidate in "$extract_dir"/*/"$executable_name" "$extract_dir"/*/*/"$executable_name"; do
            if [ -f "$candidate" ]; then
                binary_source=$candidate
                break
            fi
        done
    fi
    if [ -z "$binary_source" ]; then
        fail "archive did not contain $executable_name"
    fi

    install_binary "$binary_source"
}

if [ -n "$binary_path" ]; then
    install_binary "$binary_path"
else
    install_from_release
fi

update_shell_path

info "Refact installed successfully at $install_path"
if [ "$modify_path" -eq 0 ]; then
    info "PATH was not modified. Add $install_dir to PATH to run refact from anywhere."
elif [ "$path_update_warning" -eq 1 ]; then
    info "PATH was not changed because the selected shell profile contains malformed Refact installer markers."
elif [ -n "$modified_path_files" ]; then
    info "Added $install_dir to PATH in: $modified_path_files"
    info "Restart your terminal or source the updated shell file before running refact."
else
    info "$install_dir is already configured in PATH startup files."
fi
info "Open Refact with:"
info "  refact ui"
info "Update Refact with:"
info "  refact self-update"
