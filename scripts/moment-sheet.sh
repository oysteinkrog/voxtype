#!/usr/bin/env bash
# Contact sheet of one moment scenario (MO-07) in one look.
#
#   scripts/moment-sheet.sh THEME MOMENT OUT.png [BINARY] [FEATURES]
#
# Renders the moment offline at 10 frames a second, then lays every third
# frame out in a grid, each labelled with its time. MOMENT is a name from
# `voxtype-osd-vo --moment list`, or `all` for one sheet per moment next to
# OUT.png. FEATURES is a comma separated `--motion` list. Needs a GPU and
# ImageMagick, no display and no daemon.
set -euo pipefail

theme=${1:?theme}
moment=${2:?moment}
out=${3:?output png}
bin=${4:-target/debug/voxtype-osd-vo}
features=${5:-}
fps=10
step=3

if [[ $moment == all ]]; then
    for m in $("$bin" --moment list); do
        "$0" "$theme" "$m" "${out%.png}-$m.png" "$bin" "$features"
    done
    exit 0
fi

frames=$(mktemp -d "${TMPDIR:-/var/tmp}/moment-sheet.XXXXXX")
trap 'rm -rf "$frames"' EXIT
args=(--theme "$theme" --offline "$frames" --fps "$fps" --moment "$moment")
[[ -n $features ]] && args+=(--motion "$features")
"$bin" "${args[@]}" >/dev/null

labelled=()
i=0
for f in "$frames"/*.png; do
    if ((i % step == 0)); then
        t=$(awk -v i="$i" -v f="$fps" 'BEGIN { printf "%.1f s", i / f }')
        convert "$f" -background '#202228' -fill '#c8ccd4' -gravity south \
            -splice 0x22 -pointsize 14 -annotate +0+3 "$t" "$f.l.png"
        labelled+=("$f.l.png")
    fi
    i=$((i + 1))
done

montage "${labelled[@]}" -tile 6x -geometry +6+6 -background '#141519' \
    -title "$theme  $moment${features:+  ($features)}" -fill '#e0e4ea' "$out"
echo "$out"
