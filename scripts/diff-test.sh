#!/usr/bin/env bash
# Differential test: run every test script with the reference PHP binary and
# with phpcraft, then compare stdout and exit status.
#
# Usage:
#   scripts/diff-test.sh [directory] [--verbose]
#
# Environment:
#   PHP       reference interpreter (default: php)
#   PHPCRAFT  interpreter under test (default: target/debug/phpcraft)
#
# The corpus must stay inside the supported subset. Unsupported constructs are
# reported as failures with the parser/compiler diagnostic, which is the point:
# the failure tells us which feature to implement next.
set -uo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
php=${PHP:-php}
phpcraft=${PHPCRAFT:-$root/target/debug/phpcraft}
verbose=0
dir="$root/tests/php"

for arg in "$@"; do
    case "$arg" in
        --verbose|-v) verbose=1 ;;
        *) dir=$arg ;;
    esac
done

if [[ ! -x "$phpcraft" ]]; then
    echo "diff-test: $phpcraft not found, building..." >&2
    cargo build --quiet --manifest-path "$root/Cargo.toml" || exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

pass=0
fail=0
skip=0

while IFS= read -r file; do
    [[ -e "$file" ]] || continue
    name=${file#"$root"/}

    "$php" "$file" >"$tmp/expected.out" 2>"$tmp/expected.err"
    expected_status=$?
    "$phpcraft" "$file" >"$tmp/actual.out" 2>"$tmp/actual.err"
    actual_status=$?

    # PHP exits 255 for uncaught errors, phpcraft does the same. Treat any
    # non-zero reference status as a skipped case: the corpus is about valid
    # programs, and matching fatal diagnostics is tracked separately.
    if [[ $expected_status -ne 0 ]]; then
        skip=$((skip + 1))
        if [[ $verbose -eq 1 ]]; then
            echo "SKIP $name (reference exit $expected_status)"
        fi
        continue
    fi

    if [[ $actual_status -eq $expected_status ]] && cmp -s "$tmp/expected.out" "$tmp/actual.out"; then
        pass=$((pass + 1))
        if [[ $verbose -eq 1 ]]; then
            echo "PASS $name"
        fi
    else
        fail=$((fail + 1))
        echo "FAIL $name (exit expected=$expected_status actual=$actual_status)"
        diff -u --label "$name (php)" --label "$name (phpcraft)" \
            "$tmp/expected.out" "$tmp/actual.out" | sed 's/^/    /'
        if [[ -s "$tmp/actual.err" ]]; then
            sed 's/^/    phpcraft stderr: /' "$tmp/actual.err"
        fi
    fi
done < <(find "$dir" -type f -name '*.php' | sort)

echo
echo "diff-test: $pass passed, $fail failed, $skip skipped ($dir)"
[[ $fail -eq 0 ]]
