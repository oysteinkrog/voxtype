#!/usr/bin/env bash
# Keyboard edit focus in a private nested KWin (plan M7.2: R3, R5, R6, R40).
#
#   scripts/osd-vo-edit-focus.sh POPUP FOCUS_PROBE FAKE_KEYS
#
# POPUP is a voxtype-osd-vo binary (cargo build --features osd-vo --bin
# voxtype-osd-vo). FOCUS_PROBE and FAKE_KEYS are the vo framework's examples
# (cargo build -p vo-host-wayland --example focus_probe --example fake_keys).
#
# Inside a nested `kwin_wayland --virtual` on its own socket, never the real
# display, with its own D-Bus session and a private runtime folder for the
# voxtype files: a plain window (focus_probe target) stands in for the app
# that dictated text goes to. A small fake daemon writes the state file and
# preview.json, as the daemon does, and reads the popup's edit-cmd. It checks:
#
#   R3   the popup shows while recording and takes no focus; a pause takes it
#        through the focus proxy, and resuming gives it back to the window
#   R40  focus reaches the proxy within 100 ms of the edit block
#   keys typed while editing go to the edit buffer, not to the window
#   R5   focus taken by another window (kdotool windowactivate, as Alt+Tab)
#        detaches: the popup says so and does not take focus back; a tap
#        (the daemon's "editing") takes it again with the same text and caret
#   R6   a key held when focus goes stops repeating
#   Enter (on release) finishes with paste and Enter
#
# Writes logs under $VOXTYPE_EDIT_DIR (default /var/tmp/claude/osd-vo-edit)
# and exits 0 when every check passes.
set -uo pipefail

popup=${1:?usage: osd-vo-edit-focus.sh POPUP FOCUS_PROBE FAKE_KEYS}
probe=${2:?focus_probe binary}
keys=${3:?fake_keys binary}
work=${VOXTYPE_EDIT_DIR:-/var/tmp/claude/osd-vo-edit}
for b in "$popup" "$probe" "$keys"; do
    [ -x "$b" ] || { echo "osd-vo-edit-focus: $b is not an executable" >&2; exit 2; }
done
command -v kwin_wayland >/dev/null || { echo "osd-vo-edit-focus: kwin_wayland is not installed" >&2; exit 2; }
command -v kdotool >/dev/null || { echo "osd-vo-edit-focus: kdotool is not installed" >&2; exit 2; }

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

def ms():
    return time.clock_gettime(time.CLOCK_MONOTONIC) * 1e3

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

def target_events(kind, after):
    out = []
    for l in lines(tlog):
        parts = l.split()
        if len(parts) >= 3 and parts[1] == "target" and parts[2] == kind and float(parts[0]) >= after:
            out.append(float(parts[0]))
    return out

def buffer():
    try:
        with open(os.path.join(vox, "edit-buffer.json")) as f:
            return json.load(f)
    except (FileNotFoundError, ValueError):
        return None

def text():
    b = buffer()
    return b["text"] if b else ""

def cmds():
    return [json.loads(l)["cmd"] for l in lines(os.path.join(vox, "edit-cmd"))]

def press(*steps):
    subprocess.run([keys, *steps], env=env_keys, check=False)

# fake_keys releases what it holds when it exits, so a held key needs one
# run that waits while the test goes on.
def hold(*steps):
    return subprocess.Popen([keys, *steps], env=env_keys)

env_keys = dict(os.environ)
tlog = os.path.join(work, "target.log")
plog = os.path.join(work, "popup.log")
target = subprocess.Popen([probe, "target", "--title", "dictation-target"],
                          stdout=open(tlog, "w"), stderr=subprocess.STDOUT)
if not wait_for(tlog, lambda l: " target enter " in l, 5):
    check("target window gets focus", False)
    sys.exit(1)

env = dict(os.environ, XDG_RUNTIME_DIR=rt, WAYLAND_DISPLAY=wl,
           VOXTYPE_CONFIG=os.path.join(work, "config.toml"), RUST_LOG="voxtype_osd_vo=info,vo=info")
write("state", "recording\n")
preview("helo world")
pop = subprocess.Popen([popup, "--no-prewarm"], env=env, stdout=open(plog, "w"), stderr=subprocess.STDOUT)
shown = wait_for(plog, lambda l: "showing" in l, 10)
check("popup shows while recording", bool(shown))
t_show = ms()
time.sleep(1.0)
check("R3 the popup itself takes no focus", not target_events("leave", 0), f"leaves={target_events('leave', 0)}")

# Pause: settling, then editing.
t0 = ms()
preview("helo world", block("settling", "helo world"))
leave = wait_for(tlog, lambda l: " target leave " in l and float(l.split()[0]) >= t0, 2)
lat = float(leave[0].split()[0]) - t0 if leave else None
check("R3 a pause takes focus through the proxy", bool(leave))
check("R40 focus within 100 ms of the edit block", lat is not None and lat <= 100, f"latency_ms={lat}")
preview("helo world", block("editing", "helo world"))
time.sleep(0.2)

t1 = ms()
press(*(["left"] * 7), "l")
time.sleep(0.3)
b = buffer()
check("keys go to the edit buffer", b is not None and b["text"] == "hello world", f"buffer={b}")
check("keys do not reach the window", not [l for l in lines(tlog) if " key " in l and float(l.split()[0]) >= t1])

# Another window takes focus (as Alt+Tab): detached.
t2 = ms()
held = hold("+x", "wait:1800", "-x")
time.sleep(0.6)
subprocess.run(["kdotool", "search", "--name", "dictation-target", "windowactivate"], check=False,
               env=dict(os.environ, KDE_SESSION_VERSION="6", KDE_FULL_SESSION="true"))
back = wait_for(tlog, lambda l: " target enter " in l and float(l.split()[0]) >= t2, 2)
check("R5 the other window gets focus", bool(back))
det = wait_for(os.path.join(vox, "edit-cmd"), lambda l: '"detached"' in l, 2)
check("R5 the popup reports detached", bool(det), f"cmds={cmds()}")
xs = text().count("x")
held.wait()
check("R6 a held key stops repeating when focus goes", text().count("x") == xs, f"x={xs}->{text().count('x')}")
preview("helo world", block("detached", "helo world"))
t3 = ms()
time.sleep(1.0)
check("R5 no focus taken back by itself", not target_events("leave", t3))

# A tap: editing again, the same text and caret.
before = buffer()
t4 = ms()
preview("helo world", block("editing", "helo world"))
again = wait_for(tlog, lambda l: " target leave " in l and float(l.split()[0]) >= t4, 2)
check("R5 a tap takes focus again", bool(again))
time.sleep(0.2)
press("1")
time.sleep(0.3)
after = buffer()
want = before and before["text"][:before["sel"][1]] + "1" + before["text"][before["sel"][1]:]
check("R5 same text and caret after re-entry", bool(after) and after["text"] == want, f"before={before} after={after}")

# Resume gives focus back.
t5 = ms()
preview("hello world", block("listening", "hello world"))
ret = wait_for(tlog, lambda l: " target enter " in l and float(l.split()[0]) >= t5, 2)
check("R3 resuming gives focus back", bool(ret))
time.sleep(0.2)
check("the popup says the proxy closed", "proxy_closed" in cmds(), f"cmds={cmds()}")

# Second pause, Enter finishes on release.
preview("hello world more", block("editing", "hello world more", pause=2))
wait_for(tlog, lambda l: " target leave " in l and float(l.split()[0]) >= ms() - 50, 2)
time.sleep(0.3)
n = len(cmds())
held = hold("+enter", "wait:500", "-enter")
time.sleep(0.3)
check("Enter does nothing on press", len(cmds()) == n, f"cmds={cmds()[n:]}")
held.wait()
fin = wait_for(os.path.join(vox, "edit-cmd"), lambda l: '"finish"' in l and '"paste_enter"' in l, 2)
check("Enter finishes with paste and Enter on release", bool(fin), f"cmds={cmds()}")

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
    timeout 90 dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --socket "voxtype-edit-$$" --width 1280 --height 800 \
    --exit-with-session "$work/inner.sh" > "$work/kwin.log" 2>&1
cat "$work/inner.log"
grep -q '^RESULT PASS' "$work/inner.log"
