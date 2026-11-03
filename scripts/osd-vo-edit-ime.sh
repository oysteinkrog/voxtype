#!/usr/bin/env bash
# Keyboard edit with an input method in a private nested KWin (plan F7).
#
#   scripts/osd-vo-edit-ime.sh POPUP FOCUS_PROBE FAKE_IME
#
# POPUP is a voxtype-osd-vo binary (cargo build --features osd-vo --bin
# voxtype-osd-vo). FOCUS_PROBE and FAKE_IME are the vo framework's examples
# (cargo build -p vo-host-wayland --example focus_probe --example fake_ime).
#
# Inside a nested `kwin_wayland --virtual` on its own socket, never the real
# display, with a private runtime folder for the voxtype files. KWin starts
# FAKE_IME as its input method; each time a text input is enabled it shows
# the preedits " pl", " plu" and " plum", 150 ms apart, then commits " plum".
# A plain window stands in for the dictation target, and a small fake
# daemon writes the state file and preview.json. It checks:
#
#   on        the input method is activated when editing takes focus, and
#             is told the text around the caret
#   preedit   while it composes, the buffer never holds the preedit
#   commit    the commit goes in at the caret, and the input method is
#             told the new text
#   focus     no detach is sent, and the target window stays unfocused
#
# Writes logs under $VOXTYPE_EDIT_DIR (default
# /var/tmp/claude/osd-vo-edit-ime) and exits 0 when every check passes.
set -uo pipefail

popup=${1:?usage: osd-vo-edit-ime.sh POPUP FOCUS_PROBE FAKE_IME}
probe=${2:?focus_probe binary}
ime=${3:?fake_ime binary}
work=${VOXTYPE_EDIT_DIR:-/var/tmp/claude/osd-vo-edit-ime}
for b in "$popup" "$probe" "$ime"; do
    [ -x "$b" ] || { echo "osd-vo-edit-ime: $b is not an executable" >&2; exit 2; }
done
command -v kwin_wayland >/dev/null || { echo "osd-vo-edit-ime: kwin_wayland is not installed" >&2; exit 2; }

mkdir -p "$work"
rm -rf "$work/rt" "$work/xdg-config" "$work"/*.log
mkdir -p -m 0700 "$work/rt/voxtype"
mkdir -p "$work/xdg-config"
cat > "$work/xdg-config/kwinrc" <<EOF
[Wayland]
InputMethod=$ime
VirtualKeyboardEnabled=true
EOF
cat > "$work/config.toml" <<'EOF'
[osd]
frontend = "vo"
keyboard_edit = true
realtime = false
EOF

cat > "$work/inner.py" <<'EOF'
import json, os, subprocess, sys, time

work, popup, probe = sys.argv[1:4]
rt = os.path.join(work, "rt")
vox = os.path.join(rt, "voxtype")
wl = os.path.join(os.environ["XDG_RUNTIME_DIR"], os.environ["WAYLAND_DISPLAY"])
imelog = os.path.join(work, "ime.log")
results = []

def check(name, ok, detail=""):
    results.append(ok)
    print(f"{'PASS' if ok else 'FAIL'} {name} {detail}", flush=True)

def write(name, data):
    path = os.path.join(vox, name)
    with open(path + ".tmp", "w") as f:
        f.write(data)
    os.rename(path + ".tmp", path)

def preview(text, edit=None):
    doc = {"version": 1, "rev": 1, "text": text, "words": [], "status": None, "target": None}
    if edit:
        doc["edit"] = edit
    write("preview.json", json.dumps(doc))

def block(phase, frozen, pause=1):
    return {"session": 1, "phase": phase, "frozen": frozen, "pause": pause}

def lines(path):
    try:
        with open(path) as f:
            return f.read().splitlines()
    except FileNotFoundError:
        return []

def wait_for(path, pred, timeout=3.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        hit = [l for l in lines(path) if pred(l)]
        if hit:
            return hit
        time.sleep(0.005)
    return []

def text():
    try:
        with open(os.path.join(vox, "edit-buffer.json")) as f:
            return json.load(f)["text"]
    except (FileNotFoundError, ValueError, KeyError):
        return None

tlog = os.path.join(work, "target.log")
plog = os.path.join(work, "popup.log")
target = subprocess.Popen([probe, "target", "--title", "dictation-target"],
                          stdout=open(tlog, "w"), stderr=subprocess.STDOUT)
if not wait_for(tlog, lambda l: " target enter " in l, 5):
    check("target window gets focus", False)
    sys.exit(1)
check("KWin started the input method", bool(wait_for(imelog, lambda l: l == "ready", 5)))

env = dict(os.environ, XDG_RUNTIME_DIR=rt, WAYLAND_DISPLAY=wl,
           VOXTYPE_CONFIG=os.path.join(work, "config.toml"), RUST_LOG="voxtype_osd_vo=info,vo=info")
write("state", "recording\n")
preview("helo world")
pop = subprocess.Popen([popup, "--no-prewarm"], env=env, stdout=open(plog, "w"), stderr=subprocess.STDOUT)
check("popup shows while recording", bool(wait_for(plog, lambda l: "showing" in l, 10)))
time.sleep(0.5)
preview("helo world", block("editing", "helo world"))
check("editing takes focus", bool(wait_for(tlog, lambda l: " target leave " in l, 3)))
check("on: the input method is activated", bool(wait_for(imelog, lambda l: l == "activate", 3)))
check("on: it is told the text and caret",
      bool(wait_for(imelog, lambda l: l == 'surrounding 10 10 "helo world"', 3)),
      f"log={lines(imelog)}")

# While it composes, sample the buffer: the preedit must never be in it.
seen = set()
end = time.monotonic() + 3
while time.monotonic() < end and not any(l.startswith("commit") for l in lines(imelog)):
    seen.add(text())
    time.sleep(0.005)
check("preedit: shown three times", sum(l.startswith("preedit") for l in lines(imelog)) == 3, f"log={lines(imelog)}")
check("preedit: never in the buffer", seen <= {"helo world"}, f"seen={seen}")
time.sleep(0.3)
check("commit: goes in at the caret", text() == "helo world plum", f"buffer={text()!r}")
check("commit: the input method is told the new text",
      bool(wait_for(imelog, lambda l: l == 'surrounding 15 15 "helo world plum"', 2)), f"log={lines(imelog)}")
cmds = lines(os.path.join(vox, "edit-cmd"))
check("focus: no detach was sent", not any("detach" in c.lower() for c in cmds), f"edit-cmd={cmds}")
check("focus: the target window did not get focus back",
      sum(" target enter " in l for l in lines(tlog)) == 1)

preview("helo world plum", block("listening", "helo world plum"))
check("off: the input method is deactivated when editing ends",
      bool(wait_for(imelog, lambda l: l == "deactivate", 3)))
write("state", "idle\n")
preview("")
time.sleep(0.5)
pop.terminate()
target.terminate()
for p in (pop, target):
    try:
        p.wait(5)
    except subprocess.TimeoutExpired:
        p.kill()
print(f"RESULT {'PASS' if all(results) else 'FAIL'} {sum(results)}/{len(results)}", flush=True)
EOF

cat > "$work/inner.sh" <<EOF
#!/usr/bin/env bash
exec python3 -I "$work/inner.py" "$work" "$popup" "$probe" > "$work/inner.log" 2>&1
EOF
chmod +x "$work/inner.sh"

FAKE_IME_LOG=$work/ime.log FAKE_IME_PREEDITS=" pl, plu, plum" FAKE_IME_COMMIT=" plum" \
XDG_CONFIG_HOME=$work/xdg-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 WAYLAND_DISPLAY=no-such-wl DISPLAY= \
    timeout 60 dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --inputmethod "$ime" --socket "voxtype-ime-$$" --width 1280 --height 800 \
    --exit-with-session "$work/inner.sh" > "$work/kwin.log" 2>&1
cat "$work/inner.log"
grep -q '^RESULT PASS' "$work/inner.log"
