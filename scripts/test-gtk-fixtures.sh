#!/usr/bin/env bash
# Run the #[ignore]d GTK fixture tests matching a filter, without showing windows.
# Each test gets its own process (GTK binds to one thread), a private virtual KWin
# compositor that delivers real frame callbacks, a session bus and XDG directories.
#
#   scripts/test-gtk-fixtures.sh frames_     # the viewport stability tests
set -u
filter="${1:-fixture_}"
cd "$(dirname "$0")/.."
command -v kwin_wayland >/dev/null || { echo "kwin_wayland is required" >&2; exit 2; }
binary=$(cargo test --all-features --bin line-gtk --no-run --message-format=json 2>/dev/null |
    python3 -c 'import json,sys
for line in sys.stdin:
    event = json.loads(line)
    if event.get("executable") and event.get("profile", {}).get("test"):
        print(event["executable"])')
[ -n "$binary" ] || { echo "the test binary did not build" >&2; exit 2; }
mapfile -t tests < <("$binary" --list --ignored 2>/dev/null | sed -n "s/: test$//p" | grep -- "$filter")
scratch=$(mktemp -d)
# The Wayland socket path must fit a unix address, so keep the runtime dir short.
runtime=$(mktemp -d -p "${XDG_RUNTIME_DIR:-/tmp}" lgtk.XXXXXX)
# The session bus can leave the document portal's FUSE mount behind.
trap 'fusermount3 -u "$runtime/doc" 2>/dev/null; rm -rf "$scratch" "$runtime" 2>/dev/null' EXIT
cat >"$scratch/session.sh" <<'SESSION'
#!/usr/bin/env bash
QT_QPA_PLATFORM=offscreen kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --no-kactivities --socket "$LGTK_COMPOSITOR_SOCKET" --width 1000 --height 900 >/dev/null 2>&1 &
compositor=$!
for _ in $(seq 1 100); do [ -S "$XDG_RUNTIME_DIR/$LGTK_COMPOSITOR_SOCKET" ] && break; sleep 0.1; done
sleep 0.5
WAYLAND_DISPLAY=$LGTK_COMPOSITOR_SOCKET "$@"
status=$?
kill "$compositor" 2>/dev/null
wait "$compositor" 2>/dev/null
exit $status
SESSION
chmod +x "$scratch/session.sh"
failed=0
index=0
for test in "${tests[@]}"; do
    index=$((index + 1))
    if LGTK_COMPOSITOR_SOCKET="lgtk$index" env -u WAYLAND_DISPLAY -u DISPLAY -u KDE_FULL_SESSION GDK_BACKEND=wayland GTK_USE_PORTAL=0 \
        GSK_RENDERER=cairo GTK_A11Y=none NO_AT_BRIDGE=1 XDG_RUNTIME_DIR="$runtime" \
        XDG_DATA_HOME="$scratch/data" XDG_CONFIG_HOME="$scratch/config" \
        XDG_CACHE_HOME="$scratch/cache" XDG_STATE_HOME="$scratch/state" \
        timeout -k 3 120 setsid dbus-run-session -- "$scratch/session.sh" \
        "$binary" --ignored --exact --nocapture --test-threads=1 "$test" \
        >"$scratch/out" 2>&1 </dev/null; then
        echo "ok    $test"
    else
        failed=1
        echo "FAIL  $test"
        grep -E -A2 "panicked|Bail out" "$scratch/out" || tail -20 "$scratch/out"
    fi
done
exit $failed
