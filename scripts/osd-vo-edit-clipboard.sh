#!/usr/bin/env bash
# Keyboard edit clipboard in a private nested KWin (plan F5, R18).
#
#   scripts/osd-vo-edit-clipboard.sh POPUP FOCUS_PROBE FAKE_KEYS
#
# POPUP is a voxtype-osd-vo binary (cargo build --features osd-vo --bin
# voxtype-osd-vo). FOCUS_PROBE and FAKE_KEYS are the vo framework's examples
# (cargo build -p vo-host-wayland --example focus_probe --example fake_keys).
#
# Inside a nested `kwin_wayland --virtual` on its own socket, never the real
# display, with a private runtime folder for the voxtype files. A plain
# window stands in for the dictation target, and a small fake daemon writes
# the state file and preview.json. With the popup editing, it checks:
#
#   paste   wl-copy on the nested socket, then Ctrl+V: the buffer has it
#   copy    Ctrl+A, Ctrl+C: wl-paste on the nested socket prints the buffer
#   cut     Shift+Home, Ctrl+X: the text goes to the clipboard and out
#   R18     selecting with Shift+arrows leaves the clipboard alone, and the
#           primary selection is never touched (wl-paste --primary)
#
# Writes logs under $VOXTYPE_EDIT_DIR (default
# /var/tmp/claude/osd-vo-edit-clipboard) and exits 0 when every check passes.
set -uo pipefail

popup=${1:?usage: osd-vo-edit-clipboard.sh POPUP FOCUS_PROBE FAKE_KEYS}
probe=${2:?focus_probe binary}
keys=${3:?fake_keys binary}
work=${VOXTYPE_EDIT_DIR:-/var/tmp/claude/osd-vo-edit-clipboard}
for b in "$popup" "$probe" "$keys"; do
    [ -x "$b" ] || { echo "osd-vo-edit-clipboard: $b is not an executable" >&2; exit 2; }
done
for c in kwin_wayland wl-copy wl-paste; do
    command -v "$c" >/dev/null || { echo "osd-vo-edit-clipboard: $c is not installed" >&2; exit 2; }
done

mkdir -p "$work"
rm -rf "$work/rt" "$work/xdg-config" "$work"/*.log
mkdir -p -m 0700 "$work/rt/voxtype"
mkdir -p "$work/xdg-config"
cat > "$work/config.toml" <<'EOF'
[osd]
frontend = "vo"
keyboard_edit = true
realtime = false
EOF

cat > "$work/inner.py" <<'EOF'
import json, os, subprocess, sys, time

work, popup, probe, keys = sys.argv[1:5]
rt = os.path.join(work, "rt")
vox = os.path.join(rt, "voxtype")
wl = os.path.join(os.environ["XDG_RUNTIME_DIR"], os.environ["WAYLAND_DISPLAY"])
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

def press(*steps):
    subprocess.run([keys, *steps], check=False)
    time.sleep(0.3)

def clip(primary=False):
    args = ["wl-paste", "--no-newline"] + (["--primary"] if primary else [])
    r = subprocess.run(args, capture_output=True, text=True, timeout=3)
    return r.stdout if r.returncode == 0 else None

def copy(value, primary=False):
    # wl-copy forks a server that holds the selection until another client takes it.
    subprocess.run(["wl-copy"] + (["--primary"] if primary else []) + [value], check=False)

tlog = os.path.join(work, "target.log")
plog = os.path.join(work, "popup.log")
target = subprocess.Popen([probe, "target", "--title", "dictation-target"],
                          stdout=open(tlog, "w"), stderr=subprocess.STDOUT)
if not wait_for(tlog, lambda l: " target enter " in l, 5):
    check("target window gets focus", False)
    sys.exit(1)

copy("plum ")
copy("kiwi", primary=True)

env = dict(os.environ, XDG_RUNTIME_DIR=rt, WAYLAND_DISPLAY=wl,
           VOXTYPE_CONFIG=os.path.join(work, "config.toml"), RUST_LOG="voxtype_osd_vo=info,vo=info")
write("state", "recording\n")
preview("helo world")
pop = subprocess.Popen([popup, "--no-prewarm"], env=env, stdout=open(plog, "w"), stderr=subprocess.STDOUT)
check("popup shows while recording", bool(wait_for(plog, lambda l: "showing" in l, 10)))
time.sleep(0.5)
preview("helo world", block("editing", "helo world"))
check("editing takes focus", bool(wait_for(tlog, lambda l: " target leave " in l, 3)))
time.sleep(0.3)

# Paste at the start: Ctrl+Home, Ctrl+V.
press("+leftctrl", "home", "v", "-leftctrl")
check("paste: Ctrl+V inserts what wl-copy set", text() == "plum helo world", f"buffer={text()!r}")

# R18: a selection made with Shift+arrows sets nothing.
press("+leftshift", "left", "left", "left", "-leftshift")
check("R18 selecting leaves the clipboard alone", clip() == "plum ", f"clipboard={clip()!r}")

# Copy all: Ctrl+A, Ctrl+C.
press("+leftctrl", "a", "c", "-leftctrl")
check("copy: Ctrl+C reaches wl-paste", clip() == "plum helo world", f"clipboard={clip()!r}")

# Cut the start: Ctrl+Home, Shift+Right x5, Ctrl+X.
press("+leftctrl", "home", "-leftctrl", "+leftshift", "right", "right", "right", "right", "right", "-leftshift")
press("+leftctrl", "x", "-leftctrl")
check("cut: the text goes to the clipboard", clip() == "plum ", f"clipboard={clip()!r}")
check("cut: and out of the buffer", text() == "helo world", f"buffer={text()!r}")

# Shift+Insert pastes it back at the caret.
press("+leftshift", "insert", "-leftshift")
check("paste: Shift+Insert inserts too", text() == "plum helo world", f"buffer={text()!r}")

check("R18 the primary selection is untouched", clip(primary=True) == "kiwi", f"primary={clip(primary=True)!r}")

preview("plum helo world", block("listening", "plum helo world"))
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
exec python3 -I "$work/inner.py" "$work" "$popup" "$probe" "$keys" > "$work/inner.log" 2>&1
EOF
chmod +x "$work/inner.sh"

XDG_CONFIG_HOME=$work/xdg-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 WAYLAND_DISPLAY=no-such-wl DISPLAY= \
    timeout 60 dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --socket "voxtype-clip-$$" --width 1280 --height 800 \
    --exit-with-session "$work/inner.sh" > "$work/kwin.log" 2>&1
cat "$work/inner.log"
grep -q '^RESULT PASS' "$work/inner.log"
