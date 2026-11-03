#!/usr/bin/env bash
# Keyboard edit with the pointer in a private nested KWin (plan F6).
#
#   scripts/osd-vo-edit-pointer.sh POPUP FOCUS_PROBE FAKE_KEYS FAKE_CLICK
#
# POPUP is a voxtype-osd-vo binary (cargo build --features osd-vo --bin
# voxtype-osd-vo). FOCUS_PROBE, FAKE_KEYS and FAKE_CLICK are the vo
# framework's examples (cargo build -p vo-host-wayland --example focus_probe
# --example fake_keys --example fake_click).
#
# Inside a nested `kwin_wayland --virtual` on its own socket, never the real
# display, with a private runtime folder for the voxtype files. A plain
# window stands in for the dictation target, and a small fake daemon writes
# the state file and preview.json. The popup sits top left, so its surface
# starts at margin_px less the effect margin it grows by (VO_SURFACE_LOG
# logs that margin), and VO_DIAG_INPUT makes it log the surface-local rects
# that take the pointer. It checks:
#
#   region   no input region before editing (clicks go through), one after
#   click    a click at the start of the text puts the caret there
#   drag     a drag across the text selects it
#   double   two clicks select the word, three the line
#   focus    after the clicks, keys still reach the buffer, and the popup
#            never reported a detach
#
# Writes logs under $VOXTYPE_EDIT_DIR (default
# /var/tmp/claude/osd-vo-edit-pointer) and exits 0 when every check passes.
set -uo pipefail

popup=${1:?usage: osd-vo-edit-pointer.sh POPUP FOCUS_PROBE FAKE_KEYS FAKE_CLICK}
probe=${2:?focus_probe binary}
keys=${3:?fake_keys binary}
click=${4:?fake_click binary}
work=${VOXTYPE_EDIT_DIR:-/var/tmp/claude/osd-vo-edit-pointer}
for b in "$popup" "$probe" "$keys" "$click"; do
    [ -x "$b" ] || { echo "osd-vo-edit-pointer: $b is not an executable" >&2; exit 2; }
done
command -v kwin_wayland >/dev/null || { echo "osd-vo-edit-pointer: kwin_wayland is not installed" >&2; exit 2; }

mkdir -p "$work"
rm -rf "$work/rt" "$work/xdg-config" "$work"/*.log
mkdir -p -m 0700 "$work/rt/voxtype"
mkdir -p "$work/xdg-config"
cat > "$work/config.toml" <<'EOF'
[osd]
frontend = "vo"
keyboard_edit = true
realtime = false
position = "top-left"
margin_px = 20
EOF

cat > "$work/inner.py" <<'EOF'
import json, os, re, subprocess, sys, time

work, popup, probe, keys, click = sys.argv[1:6]
margin = 20
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

def buffer():
    try:
        with open(os.path.join(vox, "edit-buffer.json")) as f:
            b = json.load(f)
            return b["text"], tuple(b["sel"])
    except (FileNotFoundError, ValueError, KeyError):
        return None, None

def rects(line):
    return [tuple(int(v) for v in m) for m in
            re.findall(r"x: (-?\d+), y: (-?\d+), w: (\d+), h: (\d+)", line)]

def act(tool, *args):
    subprocess.run([tool, *map(str, args)], check=False)
    time.sleep(0.3)

tlog = os.path.join(work, "target.log")
plog = os.path.join(work, "popup.log")
target = subprocess.Popen([probe, "target", "--title", "dictation-target"],
                          stdout=open(tlog, "w"), stderr=subprocess.STDOUT)
if not wait_for(tlog, lambda l: " target enter " in l, 5):
    check("target window gets focus", False)
    sys.exit(1)

env = dict(os.environ, XDG_RUNTIME_DIR=rt, WAYLAND_DISPLAY=wl, VO_DIAG_INPUT="1", VO_SURFACE_LOG="1",
           VOXTYPE_CONFIG=os.path.join(work, "config.toml"), RUST_LOG="voxtype_osd_vo=info,vo=info")
write("state", "recording\n")
preview("helo world")
pop = subprocess.Popen([popup, "--no-prewarm"], env=env, stdout=open(plog, "w"), stderr=subprocess.STDOUT)
check("popup shows while recording", bool(wait_for(plog, lambda l: "showing" in l, 10)))
time.sleep(0.5)
regions = [rects(l) for l in lines(plog) if "vo: input region" in l]
check("region: none before editing", bool(regions) and all(r == [] for r in regions), f"logged={regions}")

preview("helo world", block("editing", "helo world"))
check("editing takes focus", bool(wait_for(tlog, lambda l: " target leave " in l, 3)))
hit = wait_for(plog, lambda l: "vo: input region" in l and rects(l), 3)
check("region: the edit text takes the pointer", bool(hit), hit[-1] if hit else "")
if not hit:
    sys.exit(1)
# The text box grows in as editing starts; click once it is still.
time.sleep(1.5)
hit = [l for l in lines(plog) if "vo: input region" in l and rects(l)]
x, y, w, h = rects(hit[-1])[0]
# "vo: surface WxH at t T, margin LEFT TOP RIGHT BOTTOM": the effect margin.
grow = [l.split("margin ")[1].split() for l in lines(plog) if l.startswith("vo: surface ")]
ox, oy = (margin - round(float(grow[-1][0])), margin - round(float(grow[-1][1]))) if grow else (margin, margin)
gy = oy + y + h // 2
left = ox + x + 1
right = ox + x + w - 2
print(f"surface at {ox},{oy}, text rect {x},{y} {w}x{h}: clicks at x {left}..{right}, y {gy}", flush=True)
check("buffer starts with the caret at the end", buffer() == ("helo world", (10, 10)), f"buffer={buffer()}")

act(click, "click", left, gy)
check("click: the caret goes to the start", buffer() == ("helo world", (0, 0)), f"buffer={buffer()}")

time.sleep(0.5)
act(click, "drag", left, gy, right, gy)
check("drag: selects from the press to the release", buffer() == ("helo world", (0, 10)), f"buffer={buffer()}")

time.sleep(0.5)
act(click, "click", left, gy, 2)
check("double click: selects the word", buffer() == ("helo world", (0, 4)), f"buffer={buffer()}")

time.sleep(0.5)
act(click, "click", left, gy, 3)
check("triple click: selects the line", buffer() == ("helo world", (0, 10)), f"buffer={buffer()}")

act(click, "click", right, gy)
act(keys, "x")
check("focus: keys still reach the buffer after clicks", buffer()[0] == "helo worldx", f"buffer={buffer()}")
cmds = lines(os.path.join(vox, "edit-cmd"))
check("focus: no detach was sent", not any("detach" in c.lower() for c in cmds), f"edit-cmd={cmds}")
check("focus: the target window did not get focus back",
      sum(" target enter " in l for l in lines(tlog)) == 1)

preview("helo worldx", block("listening", "helo worldx"))
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
exec python3 -I "$work/inner.py" "$work" "$popup" "$probe" "$keys" "$click" > "$work/inner.log" 2>&1
EOF
chmod +x "$work/inner.sh"

XDG_CONFIG_HOME=$work/xdg-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 WAYLAND_DISPLAY=no-such-wl DISPLAY= \
    timeout 60 dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --socket "voxtype-pointer-$$" --width 1280 --height 800 \
    --exit-with-session "$work/inner.sh" > "$work/kwin.log" 2>&1
cat "$work/inner.log"
grep -q '^RESULT PASS' "$work/inner.log"
