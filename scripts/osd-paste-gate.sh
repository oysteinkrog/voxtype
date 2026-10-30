#!/usr/bin/env bash
# Paste gate after a keyboard edit, against a real KWin and kdotool
# (plan M8.1: R26 to R29).
#
#   scripts/osd-paste-gate.sh LIB_TESTS FOCUS_PROBE
#
# LIB_TESTS is voxtype's lib test binary (cargo test --lib --no-run prints
# its path). FOCUS_PROBE is the vo framework's example (cargo build -p
# vo-host-wayland --example focus_probe).
#
# Runs the ignored test daemon::tests::nested_kwin_same_class_window_gets_no_paste_and_no_enter
# inside a nested `kwin_wayland --virtual` on its own socket, never the real
# display, with its own D-Bus session. Two windows of one app stand in for
# two terminals: the test checks that the second is not a match for the
# first, that the hand-back activates the first, and that with the first
# closed an edited dictation goes to the clipboard with no paste and no
# output hook (so no Enter).
#
# Writes logs under $VOXTYPE_GATE_DIR (default /var/tmp/claude/osd-paste-gate)
# and exits 0 when the test passes.
set -uo pipefail

tests=${1:?usage: osd-paste-gate.sh LIB_TESTS FOCUS_PROBE}
probe=${2:?focus_probe binary}
work=${VOXTYPE_GATE_DIR:-/var/tmp/claude/osd-paste-gate}
for b in "$tests" "$probe"; do
    [ -x "$b" ] || { echo "osd-paste-gate: $b is not an executable" >&2; exit 2; }
done
command -v kwin_wayland >/dev/null || { echo "osd-paste-gate: kwin_wayland is not installed" >&2; exit 2; }
command -v kdotool >/dev/null || { echo "osd-paste-gate: kdotool is not installed" >&2; exit 2; }

mkdir -p "$work"
rm -rf "$work/xdg-config" "$work"/*.log
mkdir -p "$work/xdg-config"
socket="voxtype-paste-gate-$$"

cat > "$work/inner.sh" <<EOF
#!/usr/bin/env bash
export KDE_SESSION_VERSION=6 KDE_FULL_SESSION=true VOXTYPE_TEST_FOCUS_PROBE="$probe"
exec "$tests" --ignored --exact daemon::tests::nested_kwin_same_class_window_gets_no_paste_and_no_enter \
    > "$work/inner.log" 2>&1
EOF
chmod +x "$work/inner.sh"

XDG_CONFIG_HOME=$work/xdg-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 WAYLAND_DISPLAY=no-such-wl DISPLAY= \
    timeout 90 dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --socket "$socket" --width 1280 --height 800 \
    --exit-with-session "$work/inner.sh" > "$work/kwin.log" 2>&1
cat "$work/inner.log"
grep -q '^test result: ok. 1 passed' "$work/inner.log"
