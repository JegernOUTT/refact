#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
install_sh="$script_dir/../install.sh"

if [ ! -f "$install_sh" ]; then
    printf 'error: install.sh not found at %s\n' "$install_sh" >&2
    exit 1
fi

begin_marker='# >>> Refact installer >>>'
end_marker='# <<< Refact installer <<<'
export_body='export PATH="$HOME/.refact/bin:$PATH"'
fish_body='fish_add_path "$HOME/.refact/bin"'

failures=0

host_os=$(uname -s 2>/dev/null || printf unknown)
if [ "$host_os" = "Darwin" ]; then
    bash_profile_name=".bash_profile"
    zsh_profile_name=".zprofile"
else
    bash_profile_name=".bashrc"
    zsh_profile_name=".zshrc"
fi

fail_test() {
    printf 'FAIL: %s\n' "$1" >&2
    failures=$((failures + 1))
}

pass_test() {
    printf 'ok: %s\n' "$1"
}

make_sandbox() {
    sandbox=$(mktemp -d 2>/dev/null || mktemp -d -t refact-install-test)
    fake_home="$sandbox/home"
    mkdir -p "$fake_home"
    fake_binary="$sandbox/refact"
    printf '#!/bin/sh\nexit 0\n' > "$fake_binary"
    chmod 755 "$fake_binary"
}

run_install() {
    run_home=$1
    run_shell=$2
    shift 2
    run_install_with_path "$run_home" "$run_shell" "$PATH" "$@"
}

run_install_with_path() {
    run_home=$1
    run_shell=$2
    run_path=$3
    shift 3
    env -i \
        HOME="$run_home" \
        SHELL="$run_shell" \
        PATH="$run_path" \
        bash "$install_sh" --binary "$fake_binary" "$@" >/dev/null 2>&1
}

link_target() {
    readlink "$1" 2>/dev/null || printf ''
}

count_lines() {
    if [ ! -f "$2" ]; then
        printf '0'
        return 0
    fi
    awk -v expected="$1" '{ sub(/\r$/, "") } $0 == expected { count++ } END { print count + 0 }' "$2"
}

assert_installed() {
    label=$1
    home=$2
    if [ ! -x "$home/.refact/bin/refact" ]; then
        fail_test "$label: binary not installed at ~/.refact/bin/refact"
        return 1
    fi
    return 0
}

assert_single_block() {
    label=$1
    file=$2
    body=$3
    begins=$(count_lines "$begin_marker" "$file")
    ends=$(count_lines "$end_marker" "$file")
    bodies=$(count_lines "$body" "$file")
    if [ "$begins" != "1" ] || [ "$ends" != "1" ] || [ "$bodies" != "1" ]; then
        fail_test "$label: expected exactly one managed block (begins=$begins ends=$ends bodies=$bodies) in $file"
        return 1
    fi
    return 0
}

# Test 1: initial add for bash selects the OS-appropriate profile
make_sandbox
run_install "$fake_home" "/bin/bash"
profile="$fake_home/$bash_profile_name"
if assert_installed "bash-add" "$fake_home" && [ -f "$profile" ] && assert_single_block "bash-add" "$profile" "$export_body"; then
    pass_test "bash initial add writes managed block to $bash_profile_name"
else
    fail_test "bash initial add"
fi

# Test 2: second-run idempotence keeps a single block
run_install "$fake_home" "/bin/bash"
if assert_single_block "bash-idempotent" "$profile" "$export_body"; then
    pass_test "bash rerun is idempotent"
else
    fail_test "bash rerun idempotence"
fi
rm -rf "$sandbox"

# Test 3: stale block body is replaced
make_sandbox
profile="$fake_home/$bash_profile_name"
{
    printf 'keep-before\n'
    printf '%s\n' "$begin_marker"
    printf 'export PATH="$HOME/.stale/bin:$PATH"\n'
    printf '%s\n' "$end_marker"
    printf 'keep-after\n'
} > "$profile"
run_install "$fake_home" "/bin/bash"
if assert_single_block "stale-update" "$profile" "$export_body" \
    && ! grep -Fq '.stale/bin' "$profile" \
    && grep -Fxq 'keep-before' "$profile" \
    && grep -Fxq 'keep-after' "$profile"; then
    pass_test "stale managed block body is replaced in place"
else
    fail_test "stale block update"
fi
rm -rf "$sandbox"

# Test 4: unbalanced (single marker) leaves file untouched
make_sandbox
profile="$fake_home/$bash_profile_name"
{
    printf 'keep-before\n'
    printf '%s\n' "$begin_marker"
    printf 'some orphan line\n'
} > "$profile"
before=$(cat "$profile")
run_install "$fake_home" "/bin/bash"
after=$(cat "$profile")
if [ "$before" = "$after" ]; then
    pass_test "unbalanced single marker leaves file untouched"
else
    fail_test "unbalanced marker should leave file untouched"
fi
rm -rf "$sandbox"

# Test 5: --no-modify-path does not create/modify profile
make_sandbox
profile="$fake_home/$bash_profile_name"
run_install "$fake_home" "/bin/bash" --no-modify-path
if assert_installed "no-modify-path" "$fake_home" && [ ! -f "$profile" ]; then
    pass_test "--no-modify-path installs binary without touching profile"
else
    fail_test "--no-modify-path should not modify profile"
fi
rm -rf "$sandbox"

# Test 6: zsh profile selection uses the OS-appropriate profile
make_sandbox
run_install "$fake_home" "/bin/zsh"
profile="$fake_home/$zsh_profile_name"
if [ -f "$profile" ] && assert_single_block "zsh" "$profile" "$export_body"; then
    pass_test "zsh selects $zsh_profile_name"
else
    fail_test "zsh profile selection"
fi
rm -rf "$sandbox"

# Test 7: fish syntax and config location
make_sandbox
run_install "$fake_home" "/usr/bin/fish"
profile="$fake_home/.config/fish/config.fish"
if [ -f "$profile" ] && assert_single_block "fish" "$profile" "$fish_body"; then
    pass_test "fish selects config.fish with fish_add_path body"
else
    fail_test "fish profile selection/syntax"
fi
rm -rf "$sandbox"

# Test 8: CRLF managed blocks remain idempotent and are normalized on update
make_sandbox
profile="$fake_home/$bash_profile_name"
printf '%s\r\n%s\r\n%s\r\n' "$begin_marker" "$export_body" "$end_marker" > "$profile"
run_install "$fake_home" "/bin/bash"
if assert_single_block "crlf-idempotent" "$profile" "$export_body"; then
    pass_test "CRLF managed block remains idempotent"
else
    fail_test "CRLF managed block idempotence"
fi
rm -rf "$sandbox"

# Test 9: a directory already on PATH receives a symlink to the installed binary
make_sandbox
mkdir -p "$fake_home/.local/bin"
run_install_with_path "$fake_home" "/bin/bash" "$fake_home/.local/bin:$PATH"
managed_link="$fake_home/.local/bin/refact"
if assert_installed "link into on-PATH dir" "$fake_home" &&
    [ -L "$managed_link" ] &&
    [ "$(link_target "$managed_link")" = "$fake_home/.refact/bin/refact" ]; then
    pass_test "on-PATH dir receives a symlink to the installed binary"
else
    fail_test "on-PATH dir symlink"
fi

# Test 10: relinking is idempotent
run_install_with_path "$fake_home" "/bin/bash" "$fake_home/.local/bin:$PATH"
if [ -L "$managed_link" ] && [ "$(link_target "$managed_link")" = "$fake_home/.refact/bin/refact" ]; then
    pass_test "symlink rerun is idempotent"
else
    fail_test "symlink rerun idempotence"
fi
rm -rf "$sandbox"

# Test 11: a pre-existing real file is never clobbered
make_sandbox
mkdir -p "$fake_home/.local/bin"
foreign_bin="$fake_home/.local/bin/refact"
printf '#!/bin/sh\necho foreign\n' > "$foreign_bin"
chmod 755 "$foreign_bin"
run_install_with_path "$fake_home" "/bin/bash" "$fake_home/.local/bin:$PATH"
if assert_installed "foreign binary conflict" "$fake_home" &&
    [ ! -L "$foreign_bin" ] &&
    grep -q "echo foreign" "$foreign_bin"; then
    pass_test "pre-existing real file is left untouched"
else
    fail_test "pre-existing real file must not be replaced"
fi
rm -rf "$sandbox"

# Test 12: a symlink escaping the install dir via .. is not treated as ours
make_sandbox
mkdir -p "$fake_home/.local/bin" "$fake_home/.refact/bin"
traversal_link="$fake_home/.local/bin/refact"
traversal_target="$fake_home/.refact/bin/../../evil"
ln -s "$traversal_target" "$traversal_link"
run_install_with_path "$fake_home" "/bin/bash" "$fake_home/.local/bin:$PATH"
if [ -L "$traversal_link" ] && [ "$(link_target "$traversal_link")" = "$traversal_target" ]; then
    pass_test "symlink escaping the install dir is treated as a conflict"
else
    fail_test "traversal symlink must not be claimed as installer-owned"
fi
rm -rf "$sandbox"

# Test 13: a directory that exists but is not on PATH is never linked into
make_sandbox
mkdir -p "$fake_home/.local/bin"
run_install_with_path "$fake_home" "/bin/bash" "/usr/bin:/bin"
if assert_installed "off-PATH dir" "$fake_home" && [ ! -e "$fake_home/.local/bin/refact" ]; then
    pass_test "directory not on PATH receives no symlink"
else
    fail_test "off-PATH directory must not be linked into"
fi
rm -rf "$sandbox"

# Test 14: --no-modify-path suppresses linking as well as profile edits
make_sandbox
mkdir -p "$fake_home/.local/bin"
run_install_with_path "$fake_home" "/bin/bash" "$fake_home/.local/bin:$PATH" --no-modify-path
if assert_installed "no-modify-path link suppression" "$fake_home" &&
    [ ! -e "$fake_home/.local/bin/refact" ] &&
    [ ! -f "$fake_home/$bash_profile_name" ]; then
    pass_test "--no-modify-path creates no symlink"
else
    fail_test "--no-modify-path must not create a symlink"
fi
rm -rf "$sandbox"

if [ "$failures" -ne 0 ]; then
    printf '\n%d test(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nAll install.sh path registration tests passed\n'
