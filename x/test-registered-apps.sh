#!/bin/bash
# Test every install method for every app in config/registered-apps.json.
#
# Nothing is installed for real: every method runs against a throwaway sandbox
# directory that is deleted afterwards, so ~/.cargo/bin and downloads/ are left
# exactly as they were.
#
# For each app the script tries, in order:
#   global   cargo install --root <sandbox> <crate_name>
#   git      cargo install --root <sandbox> --git <repository>
#   local    git clone <repository> into <sandbox> + cargo build --release
#
# Methods that succeed are written back to "install_methods" in
# config/registered-apps.json, methods that fail to "failed_install_methods".
# An app where no method works is flagged in logs/test-registered-apps.log.
#
# Usage: x/test-registered-apps.sh [--app <key>] [--keep]

set -u

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REGISTERED_JSON="$PROJECT_ROOT/config/registered-apps.json"
LOG_DIR="$PROJECT_ROOT/logs"
LOG_FILE="$LOG_DIR/test-registered-apps.log"

ONLY_APP=""
KEEP_SANDBOX=0

usage() {
    echo "Usage: $(basename "$0") [--app <key>] [--keep]"
    echo "  --app <key>  Only test the given registered-apps.json key"
    echo "  --keep       Keep the sandbox directory instead of deleting it"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --app)
            ONLY_APP="${2:-}"
            if [ -z "$ONLY_APP" ]; then echo "--app needs an app key"; exit 1; fi
            shift 2
            ;;
        --keep) KEEP_SANDBOX=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1"; usage; exit 1 ;;
    esac
done

PY=""
for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1; then PY="$candidate"; break; fi
done
if [ -z "$PY" ]; then echo "Python 3 is required."; exit 1; fi
if ! command -v cargo >/dev/null 2>&1; then echo "cargo is required."; exit 1; fi
if ! command -v git >/dev/null 2>&1; then echo "git is required."; exit 1; fi
if [ ! -f "$REGISTERED_JSON" ]; then echo "Not found: $REGISTERED_JSON"; exit 1; fi

mkdir -p "$LOG_DIR"
RESULTS_FILE="$(mktemp "${TMPDIR:-/tmp}/tuios-install-test.XXXXXX")"
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/tuios-install-sandbox.XXXXXX")"

cleanup() {
    rm -f "$RESULTS_FILE"
    if [ "$KEEP_SANDBOX" -eq 1 ]; then
        echo "Sandbox kept at: $SANDBOX"
    else
        rm -rf "$SANDBOX"
    fi
}
trap cleanup EXIT

log() {
    printf '[%s] [%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$1" "$2" >> "$LOG_FILE"
}

run_logged() {
    log "CMD" "$*"
    "$@" >> "$LOG_FILE" 2>&1
}

# --root keeps the built binary inside the sandbox instead of ~/.cargo/bin.
test_global() {
    local key="$1" crate="$2"
    run_logged cargo install --root "$SANDBOX/root-$key-global" "$crate"
}

test_git() {
    local key="$1" repo="$2"
    run_logged cargo install --root "$SANDBOX/root-$key-git" --git "$repo"
}

test_local() {
    local key="$1" repo="$2"
    local target="$SANDBOX/clone-$key"

    if ! run_logged git clone "$repo" "$target"; then
        return 1
    fi

    # The app store strips VCS metadata from downloads/ copies.
    rm -rf "$target/.git" "$target/.github"

    run_logged cargo build --release --manifest-path "$target/Cargo.toml"
}

APP_LIST="$("$PY" - "$REGISTERED_JSON" "$ONLY_APP" <<'PYEOF'
import json, sys

path, only = sys.argv[1], sys.argv[2]
with open(path, encoding='utf-8') as f:
    data = json.load(f)
for key, meta in data.items():
    if only and key != only:
        continue
    print('#'.join([
        key,
        meta.get('crate_name') or key,
        meta.get('repository') or '',
        meta.get('category') or 'Other',
    ]))
PYEOF
)"

if [ -z "$APP_LIST" ]; then
    echo "No apps to test in $REGISTERED_JSON${ONLY_APP:+ matching '$ONLY_APP'}"
    exit 1
fi

echo "=== tuiOS Registered Apps Install Test ==="
echo "Log:     $LOG_FILE"
echo "Sandbox: $SANDBOX (nothing is installed outside it)"
echo ""
log "SCRIPT" "test-registered-apps.sh started (sandbox $SANDBOX)"

total=0
flagged=0

while IFS='#' read -r key crate repo category; do
    [ -n "$key" ] || continue
    total=$((total + 1))
    working=""
    failed=""

    echo "--- $key ($category) ---"
    log "SCRIPT" "Testing $key (crate=$crate, repo=${repo:-none})"

    for method in global git local; do
        if [ "$method" != "global" ] && [ -z "$repo" ]; then
            echo "  - $method: skipped (no repository)"
            log "SCRIPT" "$key: $method skipped (no repository)"
            continue
        fi

        echo "  - $method: running ..."
        start=$SECONDS
        case "$method" in
            global) test_global "$key" "$crate" ;;
            git)    test_git "$key" "$repo" ;;
            local)  test_local "$key" "$repo" ;;
        esac
        rc=$?
        elapsed=$((SECONDS - start))

        if [ $rc -eq 0 ]; then
            working="${working:+$working,}$method"
            echo "    OK (${elapsed}s)"
            log "RESULT" "$key: $method OK (${elapsed}s)"
        else
            failed="${failed:+$failed,}$method"
            echo "    FAILED (${elapsed}s)"
            log "RESULT" "$key: $method FAILED (${elapsed}s, exit $rc)"
        fi

        if [ "$KEEP_SANDBOX" -eq 0 ]; then
            rm -rf "$SANDBOX/root-$key-$method" "$SANDBOX/clone-$key"
        fi
    done

    printf '%s#%s#%s\n' "$key" "${working:--}" "${failed:--}" >> "$RESULTS_FILE"

    if [ -z "$working" ]; then
        flagged=$((flagged + 1))
        echo "  FLAGGED: no install method works for $key"
        log "FLAG" "No install method works for '$key' - failed: ${failed:-none}"
    fi
    echo ""
done <<< "$APP_LIST"

"$PY" - "$REGISTERED_JSON" "$RESULTS_FILE" <<'PYEOF'
import json, sys

config_path, results_path = sys.argv[1], sys.argv[2]
with open(config_path, encoding='utf-8') as f:
    data = json.load(f)

def methods(field):
    return [] if field == '-' else [m for m in field.split(',') if m]

with open(results_path, encoding='utf-8') as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        key, working, failed = line.split('#')
        if key in data:
            data[key]['install_methods'] = methods(working)
            data[key]['failed_install_methods'] = methods(failed)

with open(config_path, 'w', encoding='utf-8') as f:
    f.write(json.dumps(data, indent=2, ensure_ascii=False) + '\n')
PYEOF

echo "=== Summary ==="
while IFS='#' read -r key working failed; do
    [ -n "$key" ] || continue
    echo "  $key: works=$working failed=$failed"
done < "$RESULTS_FILE"

echo ""
echo "Apps tested: $total, flagged (no working method): $flagged"
log "SCRIPT" "test-registered-apps.sh completed - $total tested, $flagged flagged"
echo "Updated: $REGISTERED_JSON"

if [ "$flagged" -gt 0 ]; then exit 1; fi
exit 0
