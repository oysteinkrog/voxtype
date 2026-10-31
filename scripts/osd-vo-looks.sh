#!/usr/bin/env bash
# Checks every vo popup look with its default motion (RD-05).
#
#   scripts/osd-vo-looks.sh [--motion NAMES] [BINARY]
#
# For each look in OsdTheme::VO_LOOKS (read from src/osd/config.rs) it runs
# `--envelope` and `--exit-check`, each plain, with `--cancel` and with
# VO_REDUCED_MOTION=1, plus `--exit-check --moment look-switch`. Prints one
# line per run and a summary, and exits 1 when any run fails, naming the look
# and the run. The envelope runs need a GPU; the exit checks run on the CPU.
# No display and no daemon.
#
# With --motion NAMES every run gets `--motion NAMES` (see `--help` of the
# popup: a feature name replaces the look's own set, `look,NAME` adds to
# it). The lattice mode, `--motion kinetic-lattice` (LT-11), checks the five
# looks that can run the lattice unless LOOKS says otherwise.
#
# Environment: ENVELOPE_FPS and EXIT_FPS (both default 30) set the frame
# rates, LOOKS="mecha swiss" checks only those looks.
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
motion=()
if [[ ${1:-} == --motion ]]; then
    if [[ -z ${2:-} ]]; then
        echo "osd-vo-looks: --motion needs a list of names" >&2
        exit 2
    fi
    motion=(--motion "$2")
    shift 2
fi
bin=${1:-$root/target/debug/voxtype-osd-vo}
# The looks that can run the kinetic lattice (MO-85, LT-07).
lattice_looks="scope river mission tron swiss"
# The envelope tracker follows a word from frame to frame, so at a low rate a
# word that moves far in one step reads as a breach (karaoke's centred block at
# 10 fps). 30 fps is the rate every look is checked at (RD-09).
envelope_fps=${ENVELOPE_FPS:-30}
exit_fps=${EXIT_FPS:-30}

if [[ ! -x $bin ]]; then
    echo "osd-vo-looks: no binary at $bin; build it with" >&2
    echo "  heavy-build cargo build --features osd-vo --bin voxtype-osd-vo" >&2
    exit 2
fi

if [[ -n ${LOOKS:-} ]]; then
    read -ra looks <<<"$LOOKS"
elif [[ ${motion[1]:-} == *kinetic-lattice* ]]; then
    read -ra looks <<<"$lattice_looks"
else
    # The names between `VO_LOOKS` and the closing `];`, lower case.
    mapfile -t looks < <(sed -n '/pub const VO_LOOKS/,/\];/s/.*OsdTheme::\([A-Za-z]*\),.*/\1/p' \
        "$root/src/osd/config.rs" | tr '[:upper:]' '[:lower:]')
fi
if ((${#looks[@]} == 0)); then
    echo "osd-vo-looks: found no looks in $root/src/osd/config.rs" >&2
    exit 2
fi

total=0
failed=()

# check LOOK RUN ENV... -- ARGS...: one run, one line.
check() {
    local look=$1 name=$2
    shift 2
    local envs=()
    while [[ $1 != -- ]]; do
        envs+=("$1")
        shift
    done
    shift
    local out status detail
    # A scope inherits this environment; the builds slice and nice keep the
    # checks off the desktop's CPU.
    out=$(env RUST_LOG=warn "${envs[@]}" systemd-run --user --slice=builds.slice --scope -q \
        -p CPUQuota=400% nice -n 10 "$bin" --theme "$look" "${motion[@]}" "$@" 2>&1)
    status=$?
    total=$((total + 1))
    case $name in
    exit-check*)
        detail=$(grep -m1 '^exit-check:' <<<"$out" | sed -E 's/^exit-check: (ok|FAIL) [a-z]+ //')
        ;;
    *)
        # The envelope's breach lines, or its frame count when it passed.
        detail=$(awk '/^envelope:/ { on = 1; next } /^$/ { on = 0 } on' <<<"$out" |
            awk 'NR == 1 && /frames/ { head = $0; next } { printf "%s%s", sep, $1 " x" $2; sep = ", " }
                 END { if (sep == "") printf "%s", head }')
        grep -q '^budget: over' <<<"$out" && detail+="; $(grep -m1 '^budget: over' <<<"$out")"
        ;;
    esac
    [[ -z $detail ]] && detail=$(tail -n1 <<<"$out")
    if ((status == 0)); then
        printf 'ok    %-10s %-24s %s\n' "$look" "$name" "$detail"
    else
        printf 'FAIL  %-10s %-24s %s\n' "$look" "$name" "$detail"
        failed+=("$look $name")
    fi
}

for look in "${looks[@]}"; do
    check "$look" envelope -- --envelope --fps "$envelope_fps"
    check "$look" "envelope --cancel" -- --envelope --cancel --fps "$envelope_fps"
    check "$look" "envelope reduced" VO_REDUCED_MOTION=1 -- --envelope --fps "$envelope_fps"
    check "$look" exit-check -- --exit-check --fps "$exit_fps"
    check "$look" "exit-check --cancel" -- --exit-check --cancel --fps "$exit_fps"
    check "$look" "exit-check reduced" VO_REDUCED_MOTION=1 -- --exit-check --fps "$exit_fps"
    check "$look" "exit-check look-switch" -- --exit-check --moment look-switch --fps "$exit_fps"
done

echo
echo "osd-vo-looks: ${#looks[@]} looks, $total runs, ${#failed[@]} failed${motion[1]:+ (--motion ${motion[1]})}"
for f in "${failed[@]}"; do
    echo "  FAIL $f"
done
((${#failed[@]} == 0))
