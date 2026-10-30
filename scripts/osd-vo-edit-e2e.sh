#!/usr/bin/env bash
# End-to-end check of keyboard editing (plan M7 and M8) in a private nested
# KWin, with the real daemon, the real popup and real speech.
#
#   scripts/osd-vo-edit-e2e.sh VOXTYPE FOCUS_PROBE FAKE_KEYS
#
# VOXTYPE is a voxtype binary with the parakeet and osd-vo features; the
# popup launcher (voxtype-osd) and the popup (voxtype-osd-vo) must sit next
# to it, where the daemon looks for them first.
# FOCUS_PROBE and FAKE_KEYS are the vo framework's examples (cargo build -p
# vo-host-wayland --example focus_probe --example fake_keys).
#
# Environment:
#   VOXTYPE_E2E_LIBRISPEECH  LibriSpeech folder with test-clean (default
#                            /var/tmp/claude/asr/librispeech/LibriSpeech)
#   VOXTYPE_E2E_MODELS       voxtype models folder, read only (default
#                            ~/.local/share/voxtype/models); needs
#                            parakeet-unified-en-0.6b-int8 and
#                            parakeet-tdt-0.6b-v3-int8
#   ORT_DYLIB_PATH           libonnxruntime.so, for a load-dynamic build
#   VOXTYPE_E2E_DIR          work folder (default /var/tmp/claude/osd-vo-edit-e2e)
#
# Everything runs inside `kwin_wayland --virtual` on its own socket, never the
# real display, with its own D-Bus session, its own XDG runtime, config, data
# and state folders, and no sound server. The microphone is an ALSA file
# device that plays two LibriSpeech utterances in real time. Hotkeys come in
# as `voxtype record start|edit|cancel` (the evdev listener cannot see keys
# that are not from a kernel device, and the test makes no uinput device); the
# keys in the edit are typed into the popup with fake_keys. A plain window
# (focus_probe target) stands in for the app the text goes to.
#
# Script: dictate, tap, Up twice, type, Esc (resume), dictate more, tap,
# Enter (finish). Then a second dictation: dictate, tap, type, cancel. A
# third: dictate, tap, Enter at once, before the text settles. A fourth:
# dictate, tap, close the window, Enter.
# Checks, one PASS or FAIL line each:
#   - the pasted text (the clipboard at the paste) equals what the popup
#     showed (its edit buffer when Enter was pressed)
#   - the paste and the Enter go to the confirmed window, and the Enter comes
#     after the paste
#   - a cancel pastes nothing and sends no Enter
#   - with the window closed, the text is only copied: no keys reach the
#     other window and the hook sends no Enter
#   - focus comes back to the window within 500 ms of Esc, Enter and cancel
#   - an Enter before the text settles still delivers once, and the popup
#     does not take focus again
#   - the popup keeps its size from the tap to the hand-back
#   - no key tool that could reach the real desktop ran, and the real
#     desktop's dotool pipe and clipboard are the same after the run
#
# dotool, ydotool, eitype and xdotool are stubs that fail; wl-copy and
# wl-paste work only on the nested socket; notify-send only logs. The run
# stops before it starts anything when one of them is not the stub.
#
# Exits 0 when every check passes.
set -uo pipefail

voxtype=${1:?usage: osd-vo-edit-e2e.sh VOXTYPE FOCUS_PROBE FAKE_KEYS}
probe=${2:?focus_probe binary}
keys=${3:?fake_keys binary}
libri=${VOXTYPE_E2E_LIBRISPEECH:-/var/tmp/claude/asr/librispeech/LibriSpeech}
models=${VOXTYPE_E2E_MODELS:-$HOME/.local/share/voxtype/models}
work=${VOXTYPE_E2E_DIR:-/var/tmp/claude/osd-vo-edit-e2e}
bin=$(dirname "$voxtype")
for b in "$voxtype" "$bin/voxtype-osd" "$bin/voxtype-osd-vo" "$probe" "$keys"; do
    [ -x "$b" ] || { echo "osd-vo-edit-e2e: $b is not an executable" >&2; exit 2; }
done
for c in kwin_wayland kdotool wl-copy wl-paste ffmpeg python3; do
    command -v "$c" >/dev/null || { echo "osd-vo-edit-e2e: $c is not installed" >&2; exit 2; }
done
utt=$libri/test-clean/1089/134686
[ -f "$utt/1089-134686-0000.flac" ] || { echo "osd-vo-edit-e2e: no LibriSpeech test-clean at $libri" >&2; exit 2; }
for m in parakeet-unified-en-0.6b-int8 parakeet-tdt-0.6b-v3-int8; do
    [ -d "$models/$m" ] || { echo "osd-vo-edit-e2e: no $m in $models" >&2; exit 2; }
done

mkdir -p "$work"
rm -rf "$work/rt" "$work/home" "$work/path" "$work"/*.log "$work"/*.raw
mkdir -p -m 0700 "$work/rt"
mkdir -p "$work/home/config/voxtype" "$work/home/data/voxtype" "$work/home/state" "$work/home/cache" \
    "$work/home/kwin-config"
ln -s "$models" "$work/home/data/voxtype/models"

# The microphone: utterance A, silence, utterance B, silence, as raw S16_LE
# 16 kHz mono. The ALSA device reads it from a FIFO that inner.py fills in
# real time (a plain file would be read as fast as the null device runs).
# Each open of the device starts the speech from the top.
ffmpeg -v error -y -i "$utt/1089-134686-0000.flac" -i "$utt/1089-134686-0002.flac" \
    -filter_complex "[0:a]aresample=16000,apad=whole_dur=21[a];[1:a]aresample=16000,apad=whole_dur=60[b];[a][b]concat=n=2:v=0:a=1" \
    -ac 1 -ar 16000 -f s16le "$work/mic.raw" || { echo "osd-vo-edit-e2e: ffmpeg failed" >&2; exit 2; }
rm -f "$work/mic.fifo"
mkfifo "$work/mic.fifo"
cat > "$work/asound.conf" <<EOF
# Only this device exists: no hardware and no sound server.
pcm.e2efile {
    type file
    slave.pcm {
        type null
    }
    file "/dev/null"
    infile "$work/mic.fifo"
    format "raw"
}
pcm.!default {
    type plug
    slave {
        pcm "e2efile"
        format S16_LE
        rate 16000
        channels 1
    }
    hint.description "voxtype e2e speech file"
}
EOF

# Key tools. Nothing in this test may type on the real desktop: dotool and
# ydotool write to kernel uinput devices, which every session sees, so they
# are stubs that only fail and log. KWin has no virtual keyboard protocol
# for wtype, so wtype is a stand-in that sends the same keys through KWin's
# fake input (fake_keys) to the nested session's focused window.
mkdir -p "$work/path"
for t in dotool dotoolc ydotool eitype xdotool; do
    cat > "$work/path/$t" <<'EOF'
#!/bin/sh
echo "$(date +%s%3N) blocked $(basename "$0") $*" >> "$VOXTYPE_E2E_KEYS_LOG"
exit 1
EOF
    chmod +x "$work/path/$t"
done
# The clipboard tools work only on the nested session's socket.
for t in wl-copy wl-paste; do
    real=$(command -v "$t")
    {
        echo '#!/bin/sh'
        echo 'case "$WAYLAND_DISPLAY" in'
        echo "    */voxtype-e2e-*|voxtype-e2e-*) exec '$real' \"\$@\" ;;"
        echo 'esac'
        echo 'echo "$(date +%s%3N) blocked $(basename "$0") on $WAYLAND_DISPLAY" >> "$VOXTYPE_E2E_KEYS_LOG"'
        echo 'exit 1'
    } > "$work/path/$t"
    chmod +x "$work/path/$t"
done
# No notification reaches the real desktop; the stub logs it.
cat > "$work/path/notify-send" <<'EOF'
#!/bin/sh
echo "$(date +%s%3N) notify-send $*" >> "$VOXTYPE_E2E_NOTIFY_LOG"
echo 1
EOF
chmod +x "$work/path/notify-send"
cat > "$work/path/wtype" <<'EOF'
#!/usr/bin/env bash
echo "$(date +%s%3N) wtype $*" >> "$VOXTYPE_E2E_KEYS_LOG"
name() {
    case "${1,,}" in
        shift) echo leftshift ;; ctrl|control) echo leftctrl ;; alt) echo leftalt ;;
        insert) echo 110 ;; return) echo enter ;; *) echo "${1,,}" ;;
    esac
}
steps=()
while [ $# -gt 0 ]; do
    case "$1" in
        -M) steps+=("+$(name "$2")"); shift 2 ;;
        -m) steps+=("-$(name "$2")"); shift 2 ;;
        -k) steps+=("$(name "$2")"); shift 2 ;;
        *) echo "wtype stand-in: no support for $1" >&2; exit 1 ;;
    esac
done
exec "$VOXTYPE_E2E_FAKE_KEYS" "${steps[@]}"
EOF
chmod +x "$work/path/wtype"

# Fail before anything runs when a tool that could reach the real desktop
# is not the stub.
for t in dotool dotoolc ydotool eitype xdotool wtype wl-copy wl-paste notify-send; do
    found=$(PATH="$work/path:$PATH" command -v "$t")
    [ "$found" = "$work/path/$t" ] || { echo "osd-vo-edit-e2e: $t resolves to '$found', not the stub" >&2; exit 2; }
done

# The real desktop, checked before and after the run: the dotoold pipe's
# time and a hash of the real clipboard. wl-paste reads it through
# ext-data-control and opens no window. A copy made on the real desktop
# during the run also changes the hash.
real_wl=${VOXTYPE_E2E_REAL_WAYLAND:-}
if [ -z "$real_wl" ]; then
    for sock in /run/user/$(id -u)/wayland-[0-9]; do
        [ -S "$sock" ] && { real_wl=$(basename "$sock"); break; }
    done
fi
real_state() {
    stat -c 'dotool-pipe %y' /tmp/dotool-pipe 2>/dev/null || echo "dotool-pipe none"
    if [ -n "$real_wl" ]; then
        XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=$real_wl timeout 3 \
            sh -c 'wl-paste --list-types; wl-paste --no-newline' 2>/dev/null | sha256sum | sed 's/^/clipboard /'
    else
        echo "clipboard unknown"
    fi
}
real_before=$(real_state)

# As the user's auto-Enter hook: no Enter after a paste-only finish or a
# dictation that did not deliver text.
cat > "$work/hook.sh" <<'EOF'
#!/usr/bin/env bash
rt="$XDG_RUNTIME_DIR/voxtype"
outcome=${VOXTYPE_OUTCOME:-$(cat "$rt/outcome" 2>/dev/null)}
echo "$(date +%s%3N) hook last=$(cat "$rt/hotkey-last" 2>/dev/null) outcome=$outcome" >> "$VOXTYPE_E2E_HOOK_LOG"
[ "$(cat "$rt/hotkey-last" 2>/dev/null)" = alt ] && exit 0
case "$outcome" in
    cancelled|empty|failed|not-pasted) exit 0 ;;
esac
echo "$(date +%s%3N) hook enter" >> "$VOXTYPE_E2E_HOOK_LOG"
wtype -k Return
EOF
chmod +x "$work/hook.sh"

cat > "$work/home/config/voxtype/config.toml" <<EOF
engine = "parakeet"
engine_isolation = true
live_typing = false
live_preview_model = "parakeet-unified-en-0.6b-int8"
live_preview_final = "preview"

[hotkey]
enabled = false

[audio]
device = "default"
sample_rate = 16000
max_duration_secs = 120

[audio.feedback]
enabled = false

[parakeet]
model = "parakeet-tdt-0.6b-v3-int8"

[output]
mode = "paste"
driver_order = ["wtype"]
paste_keys = "shift+insert"
restore_clipboard = false
fallback_to_clipboard = true
post_output_command = "$work/hook.sh"

[output.notification]
on_recording_start = false
on_recording_stop = false
on_transcription = false

[osd]
frontend = "vo"
theme = "swiss"
keyboard_edit = true
realtime = false
EOF

cat > "$work/inner.py" <<'EOF'
import json, os, re, shutil, subprocess, sys, threading, time

work, voxtype, probe, keys = sys.argv[1:5]
home = os.path.join(work, "home")
rt = os.path.join(work, "rt")
vox = os.path.join(rt, "voxtype")
wl = os.path.join(os.environ["XDG_RUNTIME_DIR"], os.environ["WAYLAND_DISPLAY"])
results = []

def ms():
    return time.clock_gettime(time.CLOCK_MONOTONIC) * 1e3

def check(name, ok, detail=""):
    results.append(bool(ok))
    print(f"{'PASS' if ok else 'FAIL'} {name} {detail}".rstrip(), flush=True)

def note(msg):
    print(f"     {msg}", flush=True)

def lines(path):
    try:
        with open(path) as f:
            return f.read().splitlines()
    except FileNotFoundError:
        return []

def read(name):
    try:
        with open(os.path.join(vox, name)) as f:
            return f.read()
    except FileNotFoundError:
        return None

def doc():
    try:
        return json.loads(read("preview.json") or "null")
    except ValueError:
        return None

def phase():
    d = doc()
    e = d and d.get("edit")
    return e and e.get("phase")

def wait(pred, timeout):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        v = pred()
        if v:
            return v
        time.sleep(0.01)
    return None

def target(kind, after, log=None):
    out = []
    for l in lines(log or tlog):
        p = l.split()
        if len(p) >= 3 and p[1] == "target" and p[2] == kind and float(p[0]) >= after:
            out.append((float(p[0]), p[3:]))
    return out

def keys_after(after, log=None):
    return [(t, int(a[0])) for t, a in target("key", after, log) if a[1] == "down"]

def hook_enters(hook_lines):
    return [l for l in hook_lines if l.split()[2:3] == ["enter"]]

env = dict(os.environ, XDG_RUNTIME_DIR=rt, WAYLAND_DISPLAY=wl,
           XDG_CONFIG_HOME=os.path.join(home, "config"), XDG_DATA_HOME=os.path.join(home, "data"),
           XDG_STATE_HOME=os.path.join(home, "state"), XDG_CACHE_HOME=os.path.join(home, "cache"),
           ALSA_CONFIG_PATH=os.path.join(work, "asound.conf"),
           VOXTYPE_E2E_HOOK_LOG=os.path.join(work, "hook.log"),
           VOXTYPE_E2E_KEYS_LOG=os.path.join(work, "keys.log"), VOXTYPE_E2E_FAKE_KEYS=keys,
           VOXTYPE_E2E_NOTIFY_LOG=os.path.join(work, "notify.log"),
           PATH=os.path.join(work, "path") + os.pathsep + os.environ["PATH"],
           DOTOOL_PIPE=os.path.join(work, "no-dotool-pipe"),
           YDOTOOL_SOCKET=os.path.join(work, "no-ydotool.sock"),
           RUST_LOG="voxtype=info,voxtype_osd_vo=info")
# A Plasma session as kdotool and the daemon's window checks expect it.
env.update(KDE_SESSION_VERSION="6", KDE_FULL_SESSION="true", XDG_CURRENT_DESKTOP="KDE")
for k in ("PULSE_SERVER", "PIPEWIRE_REMOTE", "PIPEWIRE_RUNTIME_DIR", "DISPLAY"):
    env.pop(k, None)

stubs = os.path.join(work, "path")
for t in ("dotool", "dotoolc", "ydotool", "eitype", "xdotool", "wtype", "wl-copy", "wl-paste", "notify-send"):
    if shutil.which(t, path=env["PATH"]) != os.path.join(stubs, t):
        check(f"{t} is the stub", False, f"found {shutil.which(t, path=env['PATH'])}")
        sys.exit(1)

def vt(*args):
    return subprocess.run([voxtype, *args], env=env, capture_output=True, text=True, timeout=20)

def press(*steps):
    subprocess.run([keys, *steps], env=env, check=False)

def clipboard():
    r = subprocess.run(["wl-paste", "--no-newline"], env=env, capture_output=True, text=True, timeout=5)
    return r.stdout if r.returncode == 0 else None

def buffer():
    try:
        return json.loads(read("edit-buffer.json") or "null")
    except ValueError:
        return None

def shown():
    # The text the popup shows: after a resume, the edited text and then the
    # preview's words past the freeze.
    d = doc() or {}
    e = d.get("edit")
    words = (d.get("text") or "").split()
    if not e:
        return " ".join(words)
    return " ".join(e.get("frozen", "").split() + words[e.get("raw_words", 0):])

def popup_size():
    # The popup's layer surfaces as KWin has them, leaving out the 1x1 focus
    # proxy that holds keyboard focus during an edit.
    r = subprocess.run(["kdotool", "search", "--class", "voxtype-osd-vo"], env=env,
                       capture_output=True, text=True, timeout=5)
    sizes = []
    for wid in r.stdout.split():
        g = subprocess.run(["kdotool", "getwindowgeometry", wid], env=env,
                           capture_output=True, text=True, timeout=5).stdout
        for l in g.splitlines():
            if "Geometry:" in l:
                size = l.split("Geometry:")[1].strip()
                if size != "1x1":
                    sizes.append(size)
    return sizes

def feed_mic():
    # Speech at 16 kHz mono S16_LE in real time, then silence, for as long as
    # the reader keeps the device open.
    with open(os.path.join(work, "mic.raw"), "rb") as f:
        speech = f.read()
    chunk = 640  # 20 ms
    while True:
        try:
            with open(os.path.join(work, "mic.fifo"), "wb", buffering=0) as w:
                start = time.monotonic()
                n = 0
                while True:
                    data = speech[n:n + chunk] or bytes(chunk)
                    w.write(data.ljust(chunk, b"\0"))
                    n += chunk
                    lag = start + n / 32000 - time.monotonic()
                    if lag > 0:
                        time.sleep(lag)
        except (BrokenPipeError, OSError):
            continue

threading.Thread(target=feed_mic, daemon=True).start()

# A second window, under the target, for the window-closed case.
olog = os.path.join(work, "other.log")
other = subprocess.Popen([probe, "target", "--title", "other-window"], env=env,
                         stdout=open(olog, "w"), stderr=subprocess.STDOUT)
wait(lambda: target("enter", 0, olog), 5)
tlog = os.path.join(work, "target.log")
tgt = subprocess.Popen([probe, "target", "--title", "dictation-target"], env=env,
                       stdout=open(tlog, "w"), stderr=subprocess.STDOUT)
if not wait(lambda: target("enter", 0), 5):
    check("the target window gets focus", False)
    sys.exit(1)

dlog = os.path.join(work, "daemon.log")
daemon = subprocess.Popen([voxtype, "-v", "daemon"], env=env, stdout=open(dlog, "w"), stderr=subprocess.STDOUT)
if not wait(lambda: (read("state") or "").strip() == "idle", 60):
    check("the daemon starts", False, "no idle state in 60 s; see daemon.log")
    daemon.terminate()
    sys.exit(1)

# --- First dictation: edit, resume, dictate more, edit, Enter. ---
t0 = ms()
vt("record", "start")
said = wait(lambda: len(((doc() or {}).get("text") or "").split()) >= 20, 25)
note(f"preview: {(doc() or {}).get('text')!r}")
check("live text shows the first utterance", said)
time.sleep(max(0.0, 12.0 - (ms() - t0) / 1e3))

size0 = popup_size()
vt("record", "edit")
editing = wait(lambda: phase() == "editing", 5)
check("a tap pauses for editing", editing, f"phase={phase()}")
left = wait(lambda: target("leave", t0), 2)
check("the popup takes focus", left)
time.sleep(0.4)
before = (buffer() or {}).get("text", "")
press("up", "up", "space", "q", "u", "i", "c", "k", "l", "y")
time.sleep(0.4)
after = (buffer() or {}).get("text", "")
note(f"edited: {after!r}")
check("Up twice and typing edit the text", "quickly" in after and after != before
      and not after.rstrip().endswith("quickly"), f"before={before!r}")
check("keys typed in the popup do not reach the window", not keys_after(t0))
size1 = popup_size()

t_esc = ms()
press("esc")
back = wait(lambda: target("enter", t_esc), 2)
lat = back[0][0] - t_esc if back else None
check("Esc gives focus back within 500 ms", lat is not None and lat <= 500, f"latency_ms={lat}")
listening = wait(lambda: phase() in ("listening", None) and "quickly" in shown(), 3)
check("after Esc the popup shows the edited text and listens", listening,
      f"phase={phase()} shown={shown()!r}")

# Utterance B starts 21 s into the recording.
more = wait(lambda: len(shown().split()) >= len(after.split()) + 8, 20)
note(f"shown: {shown()!r}")
check("the second utterance follows the edited text", more and shown().startswith(" ".join(after.split())))
time.sleep(max(0.0, 31.0 - (ms() - t0) / 1e3))

vt("record", "edit")
check("a second tap pauses again", wait(lambda: phase() == "editing", 5), f"phase={phase()}")
wait(lambda: target("leave", t_esc + 1), 2)
time.sleep(0.4)
size2 = popup_size()
at_enter = (buffer() or {}).get("text", "")
note(f"shown at Enter: {at_enter!r}")
t_enter = ms()
press("enter")
back = wait(lambda: target("enter", t_enter), 2)
lat = back[0][0] - t_enter if back else None
check("Enter gives focus back within 500 ms", lat is not None and lat <= 500, f"latency_ms={lat}")
done = wait(lambda: (read("outcome") or "").strip() == "done", 15)
check("the dictation ends as done", done, f"outcome={read('outcome')!r}")
receipt = json.loads(read("result.json") or "null") or {}
check("the popup is told what was pasted", receipt.get("text") == at_enter and receipt.get("method")
      and receipt.get("latency_ms") is not None,
      f"result={receipt}")
time.sleep(1.0)
pasted = clipboard()
check("the pasted text equals what the popup showed", pasted is not None and pasted.strip() == at_enter.strip(),
      f"pasted={pasted!r}")
k = keys_after(t_enter)
codes = [c for _, c in k]
note(f"window keys after Enter: {codes}")
paste_at = next((i for i in range(len(codes) - 1) if codes[i] == 42 and codes[i + 1] == 110), None)
check("the paste goes to the window", paste_at is not None)
check("one Enter goes to the window, after the paste",
      codes.count(28) == 1 and paste_at is not None and codes.index(28) > paste_at)
hook = lines(os.path.join(work, "hook.log"))
note(f"hook: {hook}")
check("the popup keeps its size from the tap to the hand-back",
      size0 and size0 == size1 == size2, f"{size0} {size1} {size2}")

wait(lambda: (read("state") or "").strip() == "idle", 10)
time.sleep(1.5)

# --- Second dictation: edit, then cancel (Right Alt double tap). ---
t1 = ms()
hooks_before = len(lines(os.path.join(work, "hook.log")))
clip_before = clipboard()
vt("record", "start")
wait(lambda: len(((doc() or {}).get("text") or "").split()) >= 10, 25)
time.sleep(max(0.0, 12.0 - (ms() - t1) / 1e3))
vt("record", "edit")
wait(lambda: phase() == "editing", 5)
wait(lambda: target("leave", t1), 2)
time.sleep(0.3)
press("space", "x", "y", "z")
time.sleep(0.3)
t_cancel = ms()
vt("record", "cancel")
back = wait(lambda: target("enter", t_cancel), 2)
lat = back[0][0] - t_cancel if back else None
check("a cancel gives focus back within 500 ms", lat is not None and lat <= 500, f"latency_ms={lat}")
wait(lambda: (read("state") or "").strip() == "idle", 10)
time.sleep(2.0)
check("a cancel pastes nothing and sends no Enter", not keys_after(t_cancel), f"keys={keys_after(t_cancel)}")
check("a cancel leaves the clipboard alone", clipboard() == clip_before)
new_hooks = lines(os.path.join(work, "hook.log"))[hooks_before:]
check("a cancel runs no Enter from the hook", not hook_enters(new_hooks), f"hook={new_hooks}")
kept = vt("record", "restore", "--print")
check("the cancelled edit's text can be restored", kept.returncode == 0 and "xyz" in kept.stdout, f"rc={kept.returncode}")

# --- Third dictation: Enter right after the tap, before the text settles. ---
wait(lambda: (read("state") or "").strip() == "idle", 10)
time.sleep(1.5)
t3 = ms()
hooks_before = len(lines(os.path.join(work, "hook.log")))
vt("record", "start")
wait(lambda: len(shown().split()) >= 10, 25)
time.sleep(max(0.0, 12.0 - (ms() - t3) / 1e3))
vt("record", "edit")
wait(lambda: target("leave", t3), 2)
at_press = phase()
t_early = ms()
press("enter")
note(f"phase at Enter: {at_press}")
check("Enter lands before the text has settled", at_press in ("pending", "settling"), f"phase={at_press}")
back = wait(lambda: target("enter", t_early), 2)
lat = back[0][0] - t_early if back else None
check("an early Enter gives focus back within 500 ms", lat is not None and lat <= 500, f"latency_ms={lat}")
done = wait(lambda: (read("outcome") or "").strip() == "done", 15)
check("an early Enter delivers", done, f"outcome={read('outcome')!r}")
receipt = json.loads(read("result.json") or "null") or {}
time.sleep(1.5)
codes = [c for _, c in keys_after(t_early)]
note(f"window keys after the early Enter: {codes}")
check("after an early Enter the paste and one Enter go to the window",
      codes[:2] == [42, 110] and codes.count(28) == 1 and codes.index(28) > 1, f"keys={codes}")
early = clipboard() or ""
note(f"pasted: {early!r}")
check("an early Enter pastes the dictated words", early.startswith("He hoped") and len(early.split()) >= 10)
check("the popup is told what an early Enter pasted", receipt.get("text") == early, f"result={receipt}")
check("the popup does not take focus again after an early Enter",
      not target("leave", t_early), f"leaves={target('leave', t_early)}")
check("the daemon is idle after an early Enter", (read("state") or "").strip() == "idle", f"state={read('state')!r}")

# --- Fourth dictation: the target window closes during the edit. ---
wait(lambda: (read("state") or "").strip() == "idle", 10)
time.sleep(1.5)
t2 = ms()
hooks_before = len(lines(os.path.join(work, "hook.log")))
vt("record", "start")
wait(lambda: len(shown().split()) >= 10, 25)
time.sleep(max(0.0, 12.0 - (ms() - t2) / 1e3))
vt("record", "edit")
wait(lambda: phase() == "editing", 5)
wait(lambda: target("leave", t2), 2)
time.sleep(0.3)
tgt.terminate()
tgt.wait(5)
time.sleep(0.3)
shown3 = (buffer() or {}).get("text", "")
t_gone = ms()
press("enter")
wait(lambda: (read("state") or "").strip() == "idle", 15)
time.sleep(1.5)
check("with the window closed, nothing is typed into another window", not keys_after(t_gone, olog),
      f"keys={keys_after(t_gone, olog)}")
check("with the window closed, the text is on the clipboard", (clipboard() or "").strip() == shown3.strip(),
      f"clipboard={clipboard()!r}")
check("with the window closed, the popup says it was not pasted", (read("not-pasted") or "").strip() != "",
      f"not-pasted={read('not-pasted')!r}")
new_hooks = lines(os.path.join(work, "hook.log"))[hooks_before:]
check("with the window closed, the hook sends no Enter", not hook_enters(new_hooks), f"hook={new_hooks}")
notes = [l for l in lines(os.path.join(work, "notify.log")) if "not pasted" in l.lower()]
check("with the window closed, a notification says so", notes, f"{lines(os.path.join(work, 'notify.log'))}")

# The popup logs each delivery it shows; the warm-up frames at its start
# log none.
plain = [re.sub(r"\x1b\[[0-9;]*m", "", l) for l in lines(dlog)]
shown_receipts = [l for l in plain if "lifecycle: delivered" in l]
vias = [l.split("via=")[1].split()[0] for l in shown_receipts if "via=" in l]
check("the popup shows each receipt as it went", vias == ["Pasted", "Pasted", "Copied"], f"vias={vias}")

blocked = [l for l in lines(os.path.join(work, "keys.log")) if " blocked " in l]
check("no key tool outside the nested session ran", not blocked, f"{blocked}")

daemon.terminate()
try:
    daemon.wait(10)
except subprocess.TimeoutExpired:
    daemon.kill()
other.terminate()
print(f"RESULT {'PASS' if all(results) else 'FAIL'} {sum(results)}/{len(results)}", flush=True)
EOF

cat > "$work/inner.sh" <<EOF
#!/usr/bin/env bash
exec python3 -I "$work/inner.py" "$work" "$voxtype" "$probe" "$keys" > "$work/inner.log" 2>&1
EOF
chmod +x "$work/inner.sh"

XDG_CONFIG_HOME=$work/home/kwin-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 WAYLAND_DISPLAY=no-such-wl DISPLAY= \
    timeout 400 dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --socket "voxtype-e2e-$$" --width 1280 --height 800 \
    --exit-with-session "$work/inner.sh" > "$work/kwin.log" 2>&1
real_after=$(real_state)
cat "$work/inner.log"
if [ "$real_before" = "$real_after" ]; then
    echo "PASS the real desktop's dotool pipe and clipboard are unchanged"
else
    echo "FAIL the real desktop's dotool pipe or clipboard changed during the run"
    printf '     before: %s\n     after:  %s\n' "$(echo $real_before)" "$(echo $real_after)"
    exit 1
fi
grep -q '^RESULT PASS' "$work/inner.log"
