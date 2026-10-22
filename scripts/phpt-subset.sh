#!/usr/bin/env bash
# Extract the --FILE-- section of php-src .phpt tests into plain .php scripts
# that scripts/diff-test.sh can run.
#
# Only self-contained, runnable tests are kept: tests that declare --SKIPIF--,
# --EXTENSIONS--, --INI-- or --POST--/--GET--/--EXPECTHEADERS-- sections are
# skipped because they depend on extensions, server SAPIs or configuration.
#
# Usage:
#   scripts/phpt-subset.sh [php-src-dir] [output-dir] [--limit N]
set -uo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source_dir="$root/vendor/php-src"
out_dir="$root/tests/phpt"
limit=0

positional=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --limit)
            limit=${2:-0}
            shift 2
            ;;
        *)
            positional+=("$1")
            shift
            ;;
    esac
done
[[ ${#positional[@]} -ge 1 ]] && source_dir=${positional[0]}
[[ ${#positional[@]} -ge 2 ]] && out_dir=${positional[1]}

if [[ ! -d "$source_dir" ]]; then
    echo "phpt-subset: $source_dir not found (clone php-src into vendor/php-src)" >&2
    exit 1
fi

mkdir -p "$out_dir"
count=0

while IFS= read -r file; do
    if [[ $limit -gt 0 && $count -ge $limit ]]; then
        break
    fi

    # Tests gated on extensions, configuration or a server SAPI cannot run
    # against the standalone interpreter.
    if grep -qE '^--(SKIPIF|EXTENSIONS|INI|POST|POST_RAW|GET|COOKIE|EXPECTHEADERS|ENV|STDIN)--$' "$file"; then
        continue
    fi

    file_section=$(awk '
        /^--FILE--$/ { inside = 1; next }
        inside && /^--[A-Z_]+--$/ { exit }
        inside { print }
    ' "$file")

    if [[ -z "$file_section" ]]; then
        continue
    fi

    relative=${file#"$source_dir"/}
    target="$out_dir/${relative%.phpt}.php"
    mkdir -p "$(dirname "$target")"
    printf '%s\n' "$file_section" >"$target"
    count=$((count + 1))
done < <(find "$source_dir" -name '*.phpt' -type f | sort)

echo "phpt-subset: extracted $count scripts into $out_dir"
