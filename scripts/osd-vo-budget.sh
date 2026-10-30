#!/usr/bin/env bash
# CPU, frame rate and GPU budget per vo popup look (RD-02).
#
#   scripts/osd-vo-budget.sh [BINARY]   (default target/measure, the optimised
#   build: cargo build --profile measure --features osd-vo --bin voxtype-osd-vo)
#
# For each look in OsdTheme::VO_LOOKS (read from src/osd/config.rs), with its
# default motion and the default [osd] config (an empty config file, so the
# user's own config is not read), it measures two states:
#
#   recording  `--live-moment dictation`, scene time 1.5 to 6.5 s: words
#              arriving, one revision and a 1 s pause.
#   idle-open  `--live-moment long-silence`, scene time 1.8 to 5.9 s: the
#              popup open and recording room tone, no new words.
#              The popup paces itself to 15 frames a second on room tone
#              (RD-11, RD-14) from 0.6 s after the last word, about 1.8 s here.
#
# Each live run plays the moment in the real popup surface inside its own
# nested `kwin_wayland --virtual` (never the real display). CPU is the
# process's utime + stime from /proc/<pid>/stat over the window, in percent of
# one core. Frames per second count the `vo: frame` lines (VO_DIAG_FRAMES=1)
# whose scene time falls in the window. GPU is the worst surface's median, p95
# and p99 per frame from `--offline --gpu-times --moment <moment>` at --fps 10 (no
# display, needs a GPU with timestamp queries; "-" when it has none).
#
# The gate is the median GPU time per frame. One run's p95 jumps between 0.3
# and 4.2 ms on the same binary (RD-13): at --fps 10 the GPU drops to its
# lowest clock between frames (an RTX 3060 Ti idles at 225 of 2115 MHz), so
# p95 mostly measures the clock, while the median holds still (Showcase:
# median 0.41 to 0.49 ms, p95 0.59 to 2.51 ms over 3 runs of one binary).
# Other work on the GPU only ever adds time, so the GPU part runs
# VOXTYPE_BUDGET_GPU_RUNS times (default 3) and each column is the LOWEST of
# the runs. p95 and p99 are reported and not gated. Use an optimised binary
# (cargo build --profile measure) so debug-build cost is not in the numbers.
#
# Each state runs VOXTYPE_BUDGET_RUNS times (default 3) and keeps the run
# with the median CPU: the first start of a look builds its GPU pipelines and
# can cost three times as much, and one quiet run alone would hide noise the
# other way.
#
# Writes $VOXTYPE_BUDGET_DIR/looks.tsv (default /var/tmp/claude/vo-budget),
# with logs in its looks/ folder:
#   look  state  cpu_pct  frames_per_s  gpu_ms_p95  gpu_ms_p99  gpu_ms_median
# and exits 1 naming each look and state over a limit. Limits (env vars):
#
#   VOXTYPE_BUDGET_IDLE_CPU_PCT  default 2    idle-open CPU, % of one core
#   VOXTYPE_BUDGET_SHOWCASE_IDLE_CPU_PCT  default 3   the same for Showcase (see below)
#   VOXTYPE_BUDGET_IDLE_FPS      default 30   idle-open frames per second
#   VOXTYPE_BUDGET_REC_CPU_PCT   default 10   recording CPU, % of one core
#   VOXTYPE_BUDGET_GPU_MS        default 1    GPU median per frame (lowest of 3 runs), both states
#   VOXTYPE_BUDGET_GPU_P99_MS    default 0    GPU p99 limit; 0 reports it without gating
#
# Other settings: LOOKS="swiss type" measures only those looks.
# VOXTYPE_BUDGET_CONFIG=FILE runs with that config file instead of an empty
# one, for example one with only `[osd] width_px = 600` for the wide popup.
# The policy behind the limits is rule 8 of docs/MOTION-PLAN.md in the vo
# framework repo (voxtype-osd-gpu).
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-$root/target/measure/voxtype-osd-vo}
out=${VOXTYPE_BUDGET_DIR:-/var/tmp/claude/vo-budget}
# Its own folder: the framework's budget.sh uses the same parent.
work=$out/looks
runs=${VOXTYPE_BUDGET_RUNS:-3}
gpu_runs=${VOXTYPE_BUDGET_GPU_RUNS:-3}
idle_cpu=${VOXTYPE_BUDGET_IDLE_CPU_PCT:-2}
idle_fps=${VOXTYPE_BUDGET_IDLE_FPS:-30}
# Showcase animates at idle (particle trail, breathing dots and shapes), and its idle cost
# is accepted at 3 % (giyy, 2026-10-30): the looks stay alive, no frame-rate cut at idle.
# Measured at 48cb3e4 (vo bc9e935), 7 interleaved runs: idle-open median 2.44 %. With every
# offscreen layer pass and its bloom skipped (the most a layer cache could save) the median
# is 2.20 %, so no layer cache brings it under 2 %. The rest is per-frame scene sampling,
# view, encoding and submit.
showcase_idle_cpu=${VOXTYPE_BUDGET_SHOWCASE_IDLE_CPU_PCT:-3}
rec_cpu=${VOXTYPE_BUDGET_REC_CPU_PCT:-10}
gpu_ms=${VOXTYPE_BUDGET_GPU_MS:-1}
gpu_p99=${VOXTYPE_BUDGET_GPU_P99_MS:-0}

if [[ ! -x $bin ]]; then
    echo "osd-vo-budget: no binary at $bin; build it with" >&2
    echo "  heavy-build cargo build --features osd-vo --bin voxtype-osd-vo" >&2
    exit 2
fi
command -v kwin_wayland >/dev/null || { echo "osd-vo-budget: kwin_wayland is not installed" >&2; exit 2; }

if [[ -n ${LOOKS:-} ]]; then
    read -ra looks <<<"$LOOKS"
else
    mapfile -t looks < <(sed -n '/pub const VO_LOOKS/,/\];/s/.*OsdTheme::\([A-Za-z]*\),.*/\1/p' \
        "$root/src/osd/config.rs" | tr '[:upper:]' '[:lower:]')
fi
((${#looks[@]} > 0)) || { echo "osd-vo-budget: found no looks in $root/src/osd/config.rs" >&2; exit 2; }

mkdir -p "$work/xdg-config"
: >"$work/empty.toml"
config=${VOXTYPE_BUDGET_CONFIG:-$work/empty.toml}
tsv=$out/looks.tsv
printf 'look\tstate\tcpu_pct\tframes_per_s\tgpu_ms_p95\tgpu_ms_p99\tgpu_ms_median\n' >"$tsv"
failed=()
over() { awk -v a="$1" -v b="$2" 'BEGIN { exit !(a != "-" && a > b) }'; }

# Runs inside the nested KWin: plays the moment, reads the popup's CPU ticks
# at the window's start and end, and waits for the popup to exit.
cat >"$work/inner.sh" <<'EOF'
#!/usr/bin/env bash
set -u
log=$1 ticks=$2 from=$3 to=$4
shift 4
VO_DIAG_FRAMES=1 RUST_LOG=warn "$@" >"$log" 2>&1 &
pid=$!
start=
for _ in $(seq 400); do
    start=$(grep -oP 'live-moment: \S+ starts at uptime \K[0-9.]+' "$log" 2>/dev/null) && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.02
done
cpu() { awk '{ sub(/.*\) /, ""); print $12 + $13 }' "/proc/$pid/stat" 2>/dev/null; }
until_uptime() {
    local now
    now=$(cut -d' ' -f1 /proc/uptime)
    sleep "$(awk -v a="$1" -v n="$now" 'BEGIN { d = a - n; print (d > 0 ? d : 0) }')"
}
if [[ -n $start ]]; then
    until_uptime "$(awk -v s="$start" -v f="$from" 'BEGIN { print s + f }')"
    a=$(cpu) ua=$(cut -d' ' -f1 /proc/uptime)
    until_uptime "$(awk -v s="$start" -v t="$to" 'BEGIN { print s + t }')"
    b=$(cpu) ub=$(cut -d' ' -f1 /proc/uptime)
    echo "$a $ua $b $ub" >"$ticks"
fi
wait "$pid"
echo "popup exit $?" >>"$log"
EOF
chmod +x "$work/inner.sh"

hz=$(getconf CLK_TCK)

# live LOOK STATE MOMENT FROM TO: one nested KWin run. Prints "cpu_pct fps".
live() {
    local look=$1 state=$2 moment=$3 from=$4 to=$5
    local log=$work/$look-$state.log ticks=$work/$look-$state.ticks unit=vo-look-budget-$look-$state-$$
    rm -f "$log" "$ticks"
    # Its own scope in builds.slice: stopping it ends KWin and every helper
    # its D-Bus session started.
    XDG_CONFIG_HOME=$work/xdg-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 WAYLAND_DISPLAY=no-such-wl DISPLAY= \
        timeout 60 systemd-run --user --scope --quiet --collect --slice=builds.slice --unit="$unit" \
        -p CPUQuota=400% -- dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
        --socket "vo-look-budget-$$" --width 1920 --height 1080 \
        --exit-with-session "$work/inner.sh $log $ticks $from $to $bin --config $config --theme $look --live-moment $moment" \
        >"$work/kwin-$look-$state.log" 2>&1
    systemctl --user stop "$unit.scope" 2>/dev/null
    if [[ ! -s $ticks ]] || ! grep -q 'popup exit 0' "$log" 2>/dev/null; then
        echo "- -"
        return
    fi
    read -r a ua b ub <"$ticks"
    local cpu fps
    cpu=$(awk -v a="$a" -v b="$b" -v ua="$ua" -v ub="$ub" -v hz="$hz" \
        'BEGIN { printf "%.2f", 100 * (b - a) / hz / (ub - ua) }')
    fps=$(grep -oP '^vo: frame [0-9]+ t=\K[0-9.]+' "$log" |
        awk -v f="$from" -v t="$to" '$1 >= f && $1 < t { n++ } END { printf "%.1f", n / (t - f) }')
    echo "$cpu $fps"
}

# gpu LOOK MOMENT: prints "p95 p99 median": the lowest over $gpu_runs runs of
# the worst surface's p95, p99 and median, or "- - -".
gpu() {
    local i
    for i in $(seq "$gpu_runs"); do gpu_once "$1" "$2"; done |
        awk '$1 != "-" { if (!n || $1 < a) a = $1; if (!n || $2 < b) b = $2; if (!n || $3 < c) c = $3; n++ }
            END { if (n) printf "%.3f %.3f %.3f\n", a, b, c; else print "- - -" }'
}

gpu_once() {
    local look=$1 moment=$2 dir=$work/offline-$1-$2
    rm -rf "$dir"
    local out
    out=$(RUST_LOG=warn systemd-run --user --slice=builds.slice --scope -q -p CPUQuota=400% nice -n 10 \
        "$bin" --config "$config" --theme "$look" --offline "$dir" --fps 10 --gpu-times \
        --moment "$moment" 2>&1)
    rm -rf "$dir"
    awk '/GPU .* frames,/ {
            for (i = 1; i <= NF; i++) {
                if ($i == "p95") p95 = ($(i + 1) > p95 ? $(i + 1) : p95)
                if ($i == "p99") p99 = ($(i + 1) > p99 ? $(i + 1) : p99)
                if ($i == "median") med = ($(i + 1) > med ? $(i + 1) : med)
            }
            seen = 1
        }
        END { if (seen) printf "%.3f %.3f %.3f\n", p95, p99, med; else print "- - -" }' <<<"$out"
}

for look in "${looks[@]}"; do
    for state in recording idle-open; do
        if [[ $state == recording ]]; then
            moment=dictation from=1.5 to=6.5 cpu_max=$rec_cpu fps_max=
        else
            moment=long-silence from=1.8 to=5.9 cpu_max=$idle_cpu fps_max=$idle_fps
            [[ ${look,,} == showcase ]] && cpu_max=$showcase_idle_cpu
        fi
        # The run with the median CPU of those that finished (the lower middle one
        # of an even count), with its frame rate.
        read -r cpu fps < <(for _ in $(seq "$runs"); do live "$look" "$state" "$moment" "$from" "$to"; done |
            awk '$1 != "-"' | sort -n -k1,1 | awk '{ r[NR] = $0 } END { print (NR ? r[int((NR + 1) / 2)] : "- -") }')
        read -r p95 p99 med < <(gpu "$look" "$moment")
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$look" "$state" "$cpu" "$fps" "$p95" "$p99" "$med" >>"$tsv"
        why=()
        [[ $cpu == - ]] && why+=("live run failed (see $work/$look-$state.log)")
        over "$cpu" "$cpu_max" && why+=("cpu ${cpu}% > ${cpu_max}%")
        [[ -n $fps_max ]] && over "$fps" "$fps_max" && why+=("${fps} frames/s > $fps_max")
        over "$med" "$gpu_ms" && why+=("gpu median ${med} ms > $gpu_ms ms")
        [[ $gpu_p99 != 0 ]] && over "$p99" "$gpu_p99" && why+=("gpu p99 ${p99} ms > $gpu_p99 ms")
        if ((${#why[@]} == 0)); then
            printf 'ok    %-10s %-10s cpu %6s%%  %5s frames/s  gpu median %s ms (p95 %s, p99 %s)\n' "$look" "$state" "$cpu" "$fps" "$med" "$p95" "$p99"
        else
            printf 'OVER  %-10s %-10s %s\n' "$look" "$state" "$(IFS=';'; echo "${why[*]}")"
            failed+=("$look $state")
        fi
    done
done

echo
echo "osd-vo-budget: ${#looks[@]} looks, ${#failed[@]} rows over a limit; table $tsv"
for f in "${failed[@]}"; do
    echo "  OVER $f"
done
((${#failed[@]} == 0))
