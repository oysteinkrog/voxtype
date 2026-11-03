#!/usr/bin/env bash
# Screen reader access to the popup on a private accessibility bus (plan F8).
#
#   scripts/osd-vo-a11y.sh POPUP FOCUS_PROBE
#
# POPUP is a voxtype-osd-vo binary built with the osd-vo feature (which turns
# on vo's a11y feature). FOCUS_PROBE is the vo framework's example
# (cargo build -p vo-host-wayland --example focus_probe).
#
# Inside a nested `kwin_wayland --virtual` on its own socket, never the real
# display, under its own `dbus-run-session`, so the session bus and the
# accessibility bus are private to the test. at-spi-bus-launcher runs on it
# with screen reader support switched on, as when Orca runs, and a small
# AT-SPI client (Atspi from GObject introspection) reads the tree. It checks:
#
#   off      with screen reader support off, the popup exposes nothing
#   tree     with it on, a window "Voxtype dictation" with the shown text
#   edit     while editing, a focused text field whose text is the buffer
#            and whose caret is at the buffer's caret
#   action   the screen reader moves the caret (AT-SPI SetCaretOffset) and
#            sets a selection (SetSelection), and the popup's buffer follows:
#            the field reads back the new caret and selection
#
# Writes logs under $VOXTYPE_A11Y_DIR (default /var/tmp/claude/osd-vo-a11y)
# and exits 0 when every check passes.
set -uo pipefail

popup=${1:?usage: osd-vo-a11y.sh POPUP FOCUS_PROBE}
probe=${2:?focus_probe binary}
work=${VOXTYPE_A11Y_DIR:-/var/tmp/claude/osd-vo-a11y}
launcher=/usr/lib/at-spi-bus-launcher
for b in "$popup" "$probe" "$launcher" /usr/lib/at-spi2-registryd; do
    [ -x "$b" ] || { echo "osd-vo-a11y: $b is not an executable" >&2; exit 2; }
done
command -v kwin_wayland >/dev/null || { echo "osd-vo-a11y: kwin_wayland is not installed" >&2; exit 2; }
python3 -c "import gi; gi.require_version('Atspi', '2.0')" 2>/dev/null \
    || { echo "osd-vo-a11y: python3 gi with Atspi 2.0 is not installed" >&2; exit 2; }

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

cat > "$work/read_tree.py" <<'EOF'
# Prints the popup's AT-SPI tree as JSON, or {} when it is not there.
import json, sys
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

def walk(node, depth=0):
    role = node.get_role_name()
    out = {"role": role, "name": node.get_name() or ""}
    states = node.get_state_set()
    out["focused"] = states.contains(Atspi.StateType.FOCUSED)
    if node.get_text_iface() is not None and role in ("text", "entry", "paragraph"):
        out["text"] = Atspi.Text.get_text(node, 0, -1)
        out["caret"] = Atspi.Text.get_caret_offset(node)
        if Atspi.Text.get_n_selections(node) > 0:
            r = Atspi.Text.get_selection(node, 0)
            out["selection"] = [r.start_offset, r.end_offset]
    if depth < 3:
        out["children"] = [walk(node.get_child_at_index(i), depth + 1) for i in range(node.get_child_count())]
    return out

desktop = Atspi.get_desktop(0)
for i in range(desktop.get_child_count()):
    app = desktop.get_child_at_index(i)
    for j in range(app.get_child_count()):
        win = app.get_child_at_index(j)
        if win.get_name() == "Voxtype dictation":
            print(json.dumps(walk(win)))
            sys.exit(0)
print("{}")
EOF

cat > "$work/act.py" <<'EOF'
# Acts on the popup's text field as a screen reader would: caret N, or select A B.
import sys
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

def field(node, depth=0):
    if node.get_role_name() in ("text", "entry", "paragraph") and node.get_text_iface() is not None:
        return node
    if depth < 4:
        for i in range(node.get_child_count()):
            hit = field(node.get_child_at_index(i), depth + 1)
            if hit:
                return hit
    return None

desktop = Atspi.get_desktop(0)
for i in range(desktop.get_child_count()):
    app = desktop.get_child_at_index(i)
    for j in range(app.get_child_count()):
        win = app.get_child_at_index(j)
        if win.get_name() != "Voxtype dictation":
            continue
        f = field(win)
        if f is None:
            print("no field")
            sys.exit(1)
        op = sys.argv[1]
        if op == "caret":
            ok = Atspi.Text.set_caret_offset(f, int(sys.argv[2]))
        else:
            a, b = int(sys.argv[2]), int(sys.argv[3])
            if Atspi.Text.get_n_selections(f) > 0:
                ok = Atspi.Text.set_selection(f, 0, a, b)
            else:
                ok = Atspi.Text.add_selection(f, a, b)
        print("ok" if ok else "refused")
        sys.exit(0)
print("no window")
sys.exit(1)
EOF

cat > "$work/inner.py" <<'EOF'
import json, os, subprocess, sys, time

work, popup, probe, launcher = sys.argv[1:5]
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
    write("preview", text)

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

def tree():
    r = subprocess.run(["python3", "-I", os.path.join(work, "read_tree.py")],
                       capture_output=True, text=True, timeout=10)
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {"error": r.stderr[-300:]}

def wait_tree(pred, timeout=5.0):
    end = time.monotonic() + timeout
    t = {}
    while time.monotonic() < end:
        t = tree()
        if pred(t):
            return t, True
        time.sleep(0.2)
    return t, False

def screen_reader(on):
    v = "true" if on else "false"
    for prop in ("IsEnabled",):
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", prop, "b", v], check=False)

def find(node, role):
    if node.get("role") == role:
        return node
    for c in node.get("children", []):
        hit = find(c, role)
        if hit:
            return hit
    return None

bus = subprocess.Popen([launcher, "--launch-immediately"], stdout=open(os.path.join(work, "atspi.log"), "w"),
                       stderr=subprocess.STDOUT)
time.sleep(1.0)
# The registry is normally started by systemd activation, which a private bus
# does not have: start it here. It finds the accessibility bus by itself.
registry = subprocess.Popen([launcher.replace("at-spi-bus-launcher", "at-spi2-registryd"), "--use-gnome-session"],
                            stdout=open(os.path.join(work, "registry.log"), "w"), stderr=subprocess.STDOUT)
time.sleep(0.5)
screen_reader(False)

tlog = os.path.join(work, "target.log")
plog = os.path.join(work, "popup.log")
target = subprocess.Popen([probe, "target", "--title", "dictation-target"],
                          stdout=open(tlog, "w"), stderr=subprocess.STDOUT)
wait_for(tlog, lambda l: " target enter " in l, 5)

env = dict(os.environ, XDG_RUNTIME_DIR=rt, WAYLAND_DISPLAY=wl,
           VOXTYPE_CONFIG=os.path.join(work, "config.toml"), RUST_LOG="voxtype_osd_vo=info,vo=info")
write("state", "recording\n")
preview("plum pear fig")
pop = subprocess.Popen([popup, "--no-prewarm"], env=env, stdout=open(plog, "w"), stderr=subprocess.STDOUT)
check("popup shows while recording", bool(wait_for(plog, lambda l: "showing" in l, 10)))
time.sleep(1.0)
check("off: nothing exposed with screen reader support off", tree() == {}, f"tree={tree()}")

screen_reader(True)
t, ok = wait_tree(lambda t: t.get("role") == "frame" or t.get("role") == "window")
check("tree: a window named Voxtype dictation", ok, f"tree={t}")
# The label shows the live text at most every 2 s; the first one goes at once.
t, ok = wait_tree(lambda t: any(c.get("name") == "plum pear fig" for c in t.get("children", [])))
check("tree: the shown text", ok, f"tree={t}")

preview("plum pear fig", block("editing", "plum pear fig"))
t, ok = wait_tree(lambda t: (find(t, "text") or find(t, "entry") or {}).get("text") == "plum pear fig")
field = find(t, "text") or find(t, "entry") or {}
check("edit: a text field holds the buffer", ok, f"tree={t}")
check("edit: the caret is at the buffer's caret", field.get("caret") == 13, f"field={field}")
check("edit: the field has focus", bool(field.get("focused")), f"field={field}")

def act(*args):
    r = subprocess.run(["python3", "-I", os.path.join(work, "act.py"), *map(str, args)],
                       capture_output=True, text=True, timeout=10)
    return (r.stdout + r.stderr).strip()

def field_of(t):
    return find(t, "text") or find(t, "entry") or {}

said = act("caret", 5)
t, ok = wait_tree(lambda t: field_of(t).get("caret") == 5)
check("action: the caret moves where the screen reader puts it", ok, f"act={said} field={field_of(t)}")
said = act("select", 5, 9)
t, ok = wait_tree(lambda t: field_of(t).get("selection") == [5, 9])
check("action: a selection the screen reader sets is the buffer's", ok, f"act={said} field={field_of(t)}")
check("action: the text is unchanged", field_of(t).get("text") == "plum pear fig", f"field={field_of(t)}")

preview("plum pear fig", block("listening", "plum pear fig"))
write("state", "idle\n")
preview("")
time.sleep(0.5)
for p in (pop, target, registry, bus):
    p.terminate()
for p in (pop, target, registry, bus):
    try:
        p.wait(5)
    except subprocess.TimeoutExpired:
        p.kill()
print(f"RESULT {'PASS' if all(results) else 'FAIL'} {sum(results)}/{len(results)}", flush=True)
EOF

cat > "$work/inner.sh" <<EOF
#!/usr/bin/env bash
exec python3 -I "$work/inner.py" "$work" "$popup" "$probe" "$launcher" > "$work/inner.log" 2>&1
EOF
chmod +x "$work/inner.sh"

XDG_CONFIG_HOME=$work/xdg-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 WAYLAND_DISPLAY=no-such-wl DISPLAY= \
    timeout 90 dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --socket "voxtype-a11y-$$" --width 1280 --height 800 \
    --exit-with-session "$work/inner.sh" > "$work/kwin.log" 2>&1
cat "$work/inner.log"
grep -q '^RESULT PASS' "$work/inner.log"
