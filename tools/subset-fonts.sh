#!/usr/bin/env bash
#
# subset-fonts.sh — build the six embedded "Longrun" font binaries.
#
# Produces subsetted, renamed TTFs for embedding via include_bytes!:
#
#   LongrunDisplay-Medium.ttf   family "Longrun Display"        (from Fraunces)
#   LongrunSans-Regular.ttf     family "Longrun Sans"           (from Inter)
#   LongrunSans-Medium.ttf      family "Longrun Sans Medium"    (from Inter)
#   LongrunSans-SemiBold.ttf    family "Longrun Sans SemiBold"  (from Inter)
#   LongrunMono-Regular.ttf     family "Longrun Mono"           (from IBM Plex Mono)
#   LongrunMono-Medium.ttf      family "Longrun Mono Medium"    (from IBM Plex Mono)
#
# WHY THE RENAME
#   Subsetting creates OFL 1.1 "Modified Versions". IBM Plex declares the
#   Reserved Font Name "Plex" and Fraunces declares the RFN "Fraunces", so the
#   shipped modified fonts must not use those names (Inter declares no RFN, but
#   is renamed too for consistency). All naming name-table entries (nameIDs
#   1, 3, 4, 6, 16, 17 where present) are rewritten to the Longrun names; the
#   original copyright notice (nameID 0) and license text/URL (nameIDs 13, 14)
#   are retained, as the OFL requires.
#
# ORIGINAL FONTS (exact names and versions)
#   Inter Regular / Inter Medium / Inter SemiBold
#     Version 4.001;git-9221beed3 (release v4.1)
#     https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip
#     (static TTFs from extras/ttf/ inside the zip)
#   Fraunces (variable: opsz 9-144, wght 100-900, SOFT 0-100, WONK 0-1)
#     Version 1.000;[0bf87f6ff] (release 1.000)
#     https://github.com/undercasetype/Fraunces/releases/download/1.000/UnderCaseType_Fraunces_1.000.zip
#     No static "72pt Medium" exists upstream (statics jump Regular -> SemiBold),
#     so the variable font is instanced at wght=560 opsz=72 SOFT=0 WONK=1 with
#     fontTools varLib.instancer. SOFT=0/WONK=1 matches the axis values used by
#     upstream's own named instances and shipped statics.
#   IBM Plex Mono Regular / IBM Plex Mono Medium
#     Version 2.3
#     https://github.com/google/fonts/tree/main/ofl/ibmplexmono (static TTFs;
#     pinned to commit 0b58fb370093f9a9f4ff785d94405710b79de67c)
#
# SLASHED ZERO (mono)
#   IBM Plex Mono's DEFAULT zero (U+0030 -> glyph "zero") is already the dotted
#   zero: 3 contours (outer ring, inner counter, centre dot). The OpenType
#   "zero" feature substitutes it for "zero.alt01" (slashed) and "zero.alt02"
#   is an O-shaped plain zero — i.e. the distinguishable zero needs no feature.
#   pyftfeatfreeze is therefore NOT needed and is not run; egui does no
#   OpenType shaping, and the default cmap already yields the dotted zero.
#   The validation step asserts the mono zero keeps its 3 contours.
#
# USAGE
#   tools/subset-fonts.sh [ORIGINALS_DIR] [OUTPUT_DIR]
#     ORIGINALS_DIR  where original (unmodified) downloads live; anything
#                    missing is fetched into it. Default: <repo>/target/fonts-orig
#                    (gitignored — originals must never be committed).
#     OUTPUT_DIR     where the six TTFs + license files are written.
#                    Default: <repo>/crates/craps-app/assets/fonts
#
# Deterministic and re-runnable: timestamps are not recalculated, inputs are
# pinned to exact release URLs, and outputs are pure functions of the inputs.
#
# Requires: bash, curl, unzip, python3 with fontTools >= 4.x
#   (python3 -m pip install --user fonttools)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ORIG_DIR="${1:-$REPO_ROOT/target/fonts-orig}"
OUT_DIR="${2:-$REPO_ROOT/crates/craps-app/assets/fonts}"

MAX_TOTAL_KB=900

# ---------------------------------------------------------------------------
# Glyph coverage.
#
# FULL_UNICODES: ASCII, Latin-1 supplement (incl. x00D7 multiply / x00F7
# divide), dotless i, OE ligature, Delta, sigma, general punctuation
# (thin space, en/em dash, curly quotes, bullet, ellipsis, prime, ...),
# arrows, true minus, approx, comparison operators, mac key symbols
# (cmd/opt/shift/esc/return), geometric shapes (play/disclosure triangles,
# squares, circles, diamond), check and ballot marks.
#
# Codepoints a source font does not contain are silently skipped by
# pyftsubset; per-family coverage is asserted in the validation step.
FULL_UNICODES="0020-007E,00A0-00FF,0131,0152-0153,0394,03C3,2000-206F,2190-2199,21E7,2212,2248,2260-2265,2318,2325,238B,23CE,25A0-25CF,2713,2717"

# REDUCED_UNICODES: fallback if the total exceeds MAX_TOTAL_KB — drops arrows
# and the non-essential half of general punctuation (keeps thin space, en/em
# dash, curly quotes, bullet, ellipsis, and U+2028-206F incl. primes).
REDUCED_UNICODES="0020-007E,00A0-00FF,0131,0152-0153,0394,03C3,2009,2013,2014,2018-201D,2022,2026,2028-206F,21E7,2212,2248,2260-2265,2318,2325,238B,23CE,25A0-25CF,2713,2717"

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------
if ! python3 -c "import fontTools" 2>/dev/null; then
    echo "error: python3 cannot import fontTools." >&2
    echo "       run: python3 -m pip install --user fonttools" >&2
    exit 1
fi

mkdir -p "$ORIG_DIR" "$OUT_DIR"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

FRAUNCES_VF="$ORIG_DIR/Fraunces[SOFT,WONK,opsz,wght].ttf"

# ---------------------------------------------------------------------------
# Fetch originals (only whatever is missing; ORIG_DIR acts as a cache)
# ---------------------------------------------------------------------------
INTER_ZIP_URL="https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip"
FRAUNCES_ZIP_URL="https://github.com/undercasetype/Fraunces/releases/download/1.000/UnderCaseType_Fraunces_1.000.zip"
FRAUNCES_OFL_URL="https://raw.githubusercontent.com/undercasetype/Fraunces/1.000/OFL.txt"
PLEX_PIN="0b58fb370093f9a9f4ff785d94405710b79de67c"
PLEX_BASE="https://raw.githubusercontent.com/google/fonts/$PLEX_PIN/ofl/ibmplexmono"

fetch_inter() {
    echo "-- fetching Inter 4.1 (static TTFs)"
    curl -sSfL -o "$WORK/Inter.zip" "$INTER_ZIP_URL"
    unzip -o -j -q "$WORK/Inter.zip" \
        "extras/ttf/Inter-Regular.ttf" \
        "extras/ttf/Inter-Medium.ttf" \
        "extras/ttf/Inter-SemiBold.ttf" \
        "LICENSE.txt" -d "$ORIG_DIR"
    mv "$ORIG_DIR/LICENSE.txt" "$ORIG_DIR/Inter-LICENSE.txt"
}

fetch_fraunces() {
    echo "-- fetching Fraunces 1.000 (variable TTF)"
    curl -sSfL -o "$WORK/Fraunces.zip" "$FRAUNCES_ZIP_URL"
    # NB: unzip treats [] as glob metacharacters — escape them in the member path.
    unzip -o -j -q "$WORK/Fraunces.zip" \
        "UnderCaseType_Fraunces_1.000/Fonts - Desktop/Fraunces\[SOFT,WONK,opsz,wght\].ttf" \
        -d "$ORIG_DIR"
    curl -sSfL -o "$ORIG_DIR/Fraunces-OFL.txt" "$FRAUNCES_OFL_URL"
}

fetch_plex() {
    echo "-- fetching IBM Plex Mono 2.3 (static TTFs, google/fonts @ ${PLEX_PIN:0:10})"
    curl -sSfL -o "$ORIG_DIR/IBMPlexMono-Regular.ttf" "$PLEX_BASE/IBMPlexMono-Regular.ttf"
    curl -sSfL -o "$ORIG_DIR/IBMPlexMono-Medium.ttf"  "$PLEX_BASE/IBMPlexMono-Medium.ttf"
    curl -sSfL -o "$ORIG_DIR/IBMPlexMono-OFL.txt"     "$PLEX_BASE/OFL.txt"
}

[ -f "$ORIG_DIR/Inter-Regular.ttf" ] && [ -f "$ORIG_DIR/Inter-Medium.ttf" ] && \
    [ -f "$ORIG_DIR/Inter-SemiBold.ttf" ] && [ -f "$ORIG_DIR/Inter-LICENSE.txt" ] || fetch_inter
[ -f "$FRAUNCES_VF" ] && [ -f "$ORIG_DIR/Fraunces-OFL.txt" ] || fetch_fraunces
[ -f "$ORIG_DIR/IBMPlexMono-Regular.ttf" ] && [ -f "$ORIG_DIR/IBMPlexMono-Medium.ttf" ] && \
    [ -f "$ORIG_DIR/IBMPlexMono-OFL.txt" ] || fetch_plex

# ---------------------------------------------------------------------------
# Python helpers (written to the work dir; heredocs are quoted => verbatim)
# ---------------------------------------------------------------------------

# rename.py IN OUT FAMILY FULLNAME PSNAME TYPO_FAMILY TYPO_SUBFAMILY [WEIGHT_CLASS]
#   Rewrites nameIDs 1/3/4/6 always, 16/17 only where the source had them
#   (pass "-" to skip), drops nameIDs 18/20/21/22/25 (legacy, WWS, and
#   variations aliases that may embed the old name), keeps 0/2/5/7-14
#   untouched, and optionally forces OS/2.usWeightClass. Never recalculates
#   the head timestamp.
cat > "$WORK/rename.py" <<'PY'
import sys
from fontTools.ttLib import TTFont

src, dst, family, fullname, psname, typo_family, typo_sub = sys.argv[1:8]
weight_class = int(sys.argv[8]) if len(sys.argv) > 8 else None

font = TTFont(src, recalcTimestamp=False)
name = font["name"]

version = f"{font['head'].fontRevision:.3f}"
unique_id = f"{version};Longrun;{psname}"

new_values = {1: family, 3: unique_id, 4: fullname, 6: psname}
if typo_family != "-":
    new_values[16] = typo_family
if typo_sub != "-":
    new_values[17] = typo_sub

DROP = {18, 20, 21, 22, 25}
kept = []
for rec in name.names:
    if rec.nameID in DROP:
        continue
    if rec.nameID in new_values:
        rec.string = new_values[rec.nameID].encode(
            "utf_16_be" if rec.getEncoding().startswith("utf_16") else "latin1")
    kept.append(rec)
name.names = kept

if weight_class is not None:
    font["OS/2"].usWeightClass = weight_class

font.save(dst)
PY

# validate.py OUT_DIR — asserts naming, coverage, mono zero, total size.
cat > "$WORK/validate.py" <<'PY'
import sys, os
from fontTools.ttLib import TTFont

out_dir = sys.argv[1]
max_total_kb = int(sys.argv[2])

FILES = [
    "LongrunDisplay-Medium.ttf",
    "LongrunSans-Regular.ttf",
    "LongrunSans-Medium.ttf",
    "LongrunSans-SemiBold.ttf",
    "LongrunMono-Regular.ttf",
    "LongrunMono-Medium.ttf",
]

# Codepoints every one of the six outputs must map. (U+2009 thin space and
# U+2318 cmd exist ONLY in Inter upstream; U+25B8/U+25BE exist in NONE of the
# three source families — those are therefore asserted per-family below, not
# here. Rust code should use U+25B6/U+25BC for disclosure triangles.)
CORE = [0x0030, 0x0041, 0x00D7, 0x2013, 0x2014, 0x2018, 0x2019, 0x201C,
        0x201D, 0x2022, 0x2026, 0x2212, 0x2260, 0x2264, 0x2265]

# Full symbol set: only the Sans (Inter) faces carry these upstream; at
# runtime egui falls back to the Sans family for glyphs the others lack.
SANS_EXTRA = [0x2009, 0x0394, 0x03C3, 0x2190, 0x2192, 0x21E7, 0x2248,
              0x2318, 0x2325, 0x238B, 0x23CE, 0x25B6, 0x25BC, 0x2713, 0x2717]
MONO_EXTRA = [0x2248, 0x2713]  # Plex Mono also lacks 2009/2318/etc upstream

FORBIDDEN = ("fraunces", "inter", "plex")
NAMING_IDS = (1, 3, 4, 6, 16, 17)

failures = []
total = 0
print(f"{'file':<28} {'bytes':>8}  family / postscript")
for fn in FILES:
    path = os.path.join(out_dir, fn)
    if not os.path.exists(path):
        failures.append(f"{fn}: missing")
        continue
    size = os.path.getsize(path)
    total += size
    font = TTFont(path, lazy=True)
    name = font["name"]

    # 1. No reserved/original names in naming records (0/13/14 may keep them).
    for rec in name.names:
        if rec.nameID in NAMING_IDS:
            s = rec.toUnicode().lower()
            for bad in FORBIDDEN:
                if bad in s:
                    failures.append(f"{fn}: nameID {rec.nameID} contains '{bad}': {rec.toUnicode()}")

    # 2. Coverage.
    cmap = font.getBestCmap()
    for cp in CORE:
        if cp not in cmap:
            failures.append(f"{fn}: cmap missing required U+{cp:04X}")
    if fn.startswith("LongrunSans"):
        for cp in SANS_EXTRA:
            if cp not in cmap:
                failures.append(f"{fn}: cmap missing sans symbol U+{cp:04X}")
    if fn.startswith("LongrunMono"):
        for cp in MONO_EXTRA:
            if cp not in cmap:
                failures.append(f"{fn}: cmap missing mono symbol U+{cp:04X}")

    # 3. Mono zero must still be the dotted zero (3 contours: ring, counter,
    #    dot). A plain zero would have 2.
    if fn.startswith("LongrunMono"):
        zg = cmap[0x0030]
        contours = font["glyf"][zg].numberOfContours
        if contours < 3:
            failures.append(f"{fn}: U+0030 glyph '{zg}' has {contours} contours; dotted zero lost")
        else:
            print(f"  [{fn}] U+0030 -> '{zg}', {contours} contours (dotted zero intact)")

    fam = name.getDebugName(1)
    ps = name.getDebugName(6)
    print(f"{fn:<28} {size:>8}  {fam} / {ps}")
    font.close()

print(f"{'TOTAL':<28} {total:>8}  ({total/1024:.1f} KB, limit {max_total_kb} KB)")
if total > max_total_kb * 1024:
    failures.append(f"total {total/1024:.1f} KB exceeds {max_total_kb} KB")

if failures:
    print("\nVALIDATION FAILED:")
    for f_ in failures:
        print("  -", f_)
    sys.exit(1)
print("\nvalidation OK")
PY

# ---------------------------------------------------------------------------
# Step 1: instance Fraunces (variable -> static) at wght=560 opsz=72
# ---------------------------------------------------------------------------
FRAUNCES_STATIC="$WORK/Fraunces-opsz72-wght560.ttf"
echo "-- instancing Fraunces at wght=560 opsz=72 SOFT=0 WONK=1"
# Python API rather than the varLib.instancer CLI so the head.modified
# timestamp is preserved (recalcTimestamp=False) and the output is
# byte-for-byte reproducible run to run.
python3 - "$FRAUNCES_VF" "$FRAUNCES_STATIC" <<'PY'
import sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

src, dst = sys.argv[1:3]
font = TTFont(src, recalcTimestamp=False)
instancer.instantiateVariableFont(
    font, {"wght": 560, "opsz": 72, "SOFT": 0, "WONK": 1}, inplace=True)
font.save(dst)
PY

# ---------------------------------------------------------------------------
# Step 2+3: subset then rename each font
# ---------------------------------------------------------------------------
# subset_one IN OUT UNICODES
subset_one() {
    # Hinting instructions are KEPT (fontTools keeps them unless --no-hinting
    # is passed). --name-IDs='*' keeps every name record; the rename step
    # rewrites the naming ones afterwards. --glyph-names keeps post-table
    # glyph names; --notdef-outline keeps the .notdef box.
    python3 -m fontTools.subset "$1" \
        --output-file="$2" \
        --unicodes="$3" \
        --layout-features="kern,liga" \
        --glyph-names \
        --name-IDs='*' \
        --notdef-outline
}

# build_all UNICODES — produces the six renamed TTFs in OUT_DIR
build_all() {
    local unicodes="$1"

    #          source                              out                     family                 fullname                   psname                 typo16          typo17     weight
    subset_one "$FRAUNCES_STATIC"                  "$WORK/f1.ttf" "$unicodes"
    python3 "$WORK/rename.py" "$WORK/f1.ttf" "$OUT_DIR/LongrunDisplay-Medium.ttf" \
        "Longrun Display" "Longrun Display Medium" "LongrunDisplay-Medium" "Longrun Display" "Medium" 560

    subset_one "$ORIG_DIR/Inter-Regular.ttf"       "$WORK/f2.ttf" "$unicodes"
    python3 "$WORK/rename.py" "$WORK/f2.ttf" "$OUT_DIR/LongrunSans-Regular.ttf" \
        "Longrun Sans" "Longrun Sans Regular" "LongrunSans-Regular" "-" "-"

    subset_one "$ORIG_DIR/Inter-Medium.ttf"        "$WORK/f3.ttf" "$unicodes"
    python3 "$WORK/rename.py" "$WORK/f3.ttf" "$OUT_DIR/LongrunSans-Medium.ttf" \
        "Longrun Sans Medium" "Longrun Sans Medium" "LongrunSans-Medium" "Longrun Sans" "Medium"

    subset_one "$ORIG_DIR/Inter-SemiBold.ttf"      "$WORK/f4.ttf" "$unicodes"
    python3 "$WORK/rename.py" "$WORK/f4.ttf" "$OUT_DIR/LongrunSans-SemiBold.ttf" \
        "Longrun Sans SemiBold" "Longrun Sans SemiBold" "LongrunSans-SemiBold" "Longrun Sans" "SemiBold"

    subset_one "$ORIG_DIR/IBMPlexMono-Regular.ttf" "$WORK/f5.ttf" "$unicodes"
    python3 "$WORK/rename.py" "$WORK/f5.ttf" "$OUT_DIR/LongrunMono-Regular.ttf" \
        "Longrun Mono" "Longrun Mono Regular" "LongrunMono-Regular" "-" "-"

    subset_one "$ORIG_DIR/IBMPlexMono-Medium.ttf"  "$WORK/f6.ttf" "$unicodes"
    python3 "$WORK/rename.py" "$WORK/f6.ttf" "$OUT_DIR/LongrunMono-Medium.ttf" \
        "Longrun Mono Medium" "Longrun Mono Medium" "LongrunMono-Medium" "Longrun Mono" "Medium"
}

echo "-- subsetting + renaming (full glyph set)"
build_all "$FULL_UNICODES"

TOTAL_KB=$(( $(du -ck "$OUT_DIR"/Longrun*.ttf | tail -1 | cut -f1) ))
if [ "$TOTAL_KB" -gt "$MAX_TOTAL_KB" ]; then
    echo "-- total ${TOTAL_KB} KB > ${MAX_TOTAL_KB} KB: rebuilding with reduced glyph set"
    build_all "$REDUCED_UNICODES"
fi

# ---------------------------------------------------------------------------
# Step 4: license files (verbatim OFL 1.1 texts with original copyright lines)
# ---------------------------------------------------------------------------
cp "$ORIG_DIR/Fraunces-OFL.txt"    "$OUT_DIR/LICENSE-Fraunces.txt"
cp "$ORIG_DIR/Inter-LICENSE.txt"   "$OUT_DIR/LICENSE-Inter.txt"
cp "$ORIG_DIR/IBMPlexMono-OFL.txt" "$OUT_DIR/LICENSE-IBMPlexMono.txt"

# ---------------------------------------------------------------------------
# Step 5: size report + validation
# ---------------------------------------------------------------------------
echo
python3 "$WORK/validate.py" "$OUT_DIR" "$MAX_TOTAL_KB"
