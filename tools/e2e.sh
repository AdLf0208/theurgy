#!/usr/bin/env bash
# In-game check for Theurgy slice 1: the calcination oven turns one item into
# its alchemical salt while the pyromantic brazier below it burns fuel.
#
# Every mod command is typed with `ctl chat "/cmd"` because `ctl command` runs
# the console path and bypasses the native `perform_command` hook.
#
# The End, in the void at x=200 (away from the central island, its dragon egg
# and the ambient item entities of a generated level). Every pair stands on the
# stone floor: a brazier at y=101 and the oven above it at y=102.
#
# Default run:
#  A  the two blocks, the five salts and their block/enchant-free ids register;
#     an empty oven reads progress 0/100, lit=false, empty slots;
#  B  COLD: one blackstone in an unheated oven never progresses: after 240 ticks
#     the input is still there, the output is empty and the oven is dark;
#  C  HEAT + CRAFT: one coal lights the brazier, the oven follows within its
#     20-tick heat sample, and the blackstone becomes one strata salt after
#     100 heated ticks (the jar's `time: 100`), consumed 1 for 1;
#  D  TAKE: an empty hand takes the output first (and the input second);
#  E  RATIO: one packed ice is nine strata salts, not one — the jar's own count;
#  E2 TAG: one diamond is two mineral salts through the port's own #c:gems tag,
#     so the engine's merged item tags answer the jar's tag ingredients;
#  F  COUNT: two plant salts make one creature salt, so five in the input leave
#     three (the recipe's `count: 2`);
#  G  FUEL: one stick heats for 100 ticks and then the brazier and the oven go
#     dark again.
#
# EMBEDDED=1: the game is launched with **only the native bundle** (no
# `--dev-mod`) and the content comes from the bundle's embedded copy
# (`mixins/theurgy/data/…`, the AE2 pattern). CONTROL=1 (default with EMBEDDED)
# first launches the same binary with `.horizon-cache` unusable, where the ids
# must be unknown.
#
# SELFTEST=1 flips E's nine to eight (it must FAIL), proving the scenario can
# fail.
set -Eeuo pipefail
export LC_ALL=C
[ -z "${TH_DEBUG:-}" ] || set -x

EMBEDDED=${EMBEDDED:-0}
CONTROL=${CONTROL:-$EMBEDDED}

CLONE=${CLONE:-$(cd "$(dirname "$0")/../../.." && pwd)}
PORT=$CLONE/ports/theurgy
PIN_BIN=$CLONE/ports/_shared/bin/horizon-1d63e0a5
CTL=${CTL:-$PIN_BIN/horizon-ctl}
GAME=${GAME:-$PORT/target/horizon-cargo/fast/horizon-client}
RES=${RES:-${HORIZON_RES:-${HORIZON_RES:-$HOME/.local/horizon-client/26.3}}}
SOCK=${SOCK:-/tmp/opencode/theurgy-e2e.sock}
SELFTEST=${SELFTEST:-0}
KEEP=${KEEP:-0}
WORLD=TheurgyE2E
CONTROL_WORLD=TheurgyControl
# The arena: the End's void, far from the central island.
X0=200
SX=$((X0 + 2))
OVEN=theurgy:calcination_oven
BRAZIER=theurgy:pyromantic_brazier
STRATA=theurgy:alchemical_salt_strata
CREATURE=theurgy:alchemical_salt_creature
PLANT=theurgy:alchemical_salt_plant
MINERAL=theurgy:alchemical_salt_mineral

command -v jq >/dev/null || { echo "missing jq (every assertion parses a ctl reply)" >&2; exit 1; }
[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME" >&2; exit 1; }
[ -d "$RES/assets" ] || { echo "missing prepared resources: $RES" >&2; exit 1; }

SCRATCH=$(mktemp -d /tmp/opencode/theurgy-home.XXXXXX)
export HORIZON_HOME=$SCRATCH/game
export HORIZON_LAUNCHER_HOME=$SCRATCH/launcher
export XDG_DATA_HOME=$SCRATCH/data
export XDG_CONFIG_HOME=$SCRATCH/config
export HORIZON_NO_AUDIO=1
mkdir -p "$(dirname "$SOCK")" "$XDG_CONFIG_HOME"
rm -f "$SOCK"

ctl() { "$CTL" --socket "$SOCK" "$@"; }
say() { ctl chat "/$1" >/dev/null; }
fail() { echo "FAIL: $*" >&2; ctl logs --lines 60 >&2 || true; exit 1; }
on_error() { echo "FAIL: line $1: '$2' exited $?" >&2; ctl logs --lines 40 >&2 || true; }
trap 'on_error "$LINENO" "$BASH_COMMAND"' ERR

cleanup() {
    local status=$?
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    if [ "$KEEP" = 1 ]; then
        echo "kept scratch: $SCRATCH" >&2
    else
        rm -rf "$SCRATCH" 2>/dev/null || true
    fi
    # Last line of every run: `check-e2e-logs.py` reads it as the verdict.
    echo "EXIT:$status"
}
trap cleanup EXIT

launch() { # launch [WORLD]
    local world=${1:-$WORLD}
    rm -f "$SOCK"
    cd "$SCRATCH"
    local args=(--game "$GAME" --world "$world" --seed 20261105 --creative --headless
                --wait --resources "$RES")
    if [ "$EMBEDDED" = 1 ]; then
        echo "  launch: no --dev-mod; the bundle carries its own content" >&2
    else
        args+=(--dev-mod "$PORT/content")
    fi
    "$CTL" --socket "$SOCK" launch "${args[@]}" >/dev/null
    ctl wait-ready 120
    ctl wait 40
}

block_name() { ctl block "$1" "$2" "$3" | jq -r '.name // .block // empty'; }

# The End's void at X0: a stone floor and clear air above it, so a pair can be
# placed and nothing ambient interferes. The player is teleported first, so the
# chunks the fills touch are loaded.
arena() {
    ctl command "execute in minecraft:the_end run tp @s $X0.5 104 0.5" >/dev/null
    local loaded=false
    for _ in $(seq 1 120); do
        ctl wait 5 >/dev/null
        if [ "$(ctl block "$X0" 100 0 | jq -r '.loaded // false')" = true ]; then
            loaded=true
            break
        fi
    done
    [ "$loaded" = true ] || fail "the End arena chunk never loaded"
    ctl command "execute in minecraft:the_end run fill $((X0 - 2)) 100 -14 $((X0 + 4)) 100 14 minecraft:stone" >/dev/null
    ctl wait 6 >/dev/null
    ctl command "execute in minecraft:the_end run fill $((X0 - 2)) 101 -14 $((X0 + 4)) 108 14 minecraft:air" >/dev/null
    ctl wait 6 >/dev/null
    ctl command "execute in minecraft:the_end run kill @e[type=!player]" >/dev/null
    ctl command "execute in minecraft:the_end run tp @s $X0.5 101 0.5" >/dev/null
    ctl wait 20
    [ "$(block_name "$X0" 100 0)" = "minecraft:stone" ] || fail "the arena floor is not stone"
    [ "$(block_name "$X0" 101 0)" = "minecraft:air" ] || fail "the arena air above the floor is not air"
}

# `/theurgy info X Y Z` into OVEN_*/BRAZIER_* shell fields.
info() { # info X Y Z
    say "theurgy info $1 $2 $3"
    ctl wait 12 >/dev/null
    INFO_LINE=$(ctl messages 12 | jq -r '.chat[]' | grep -m1 -E 'Calcination Oven at|Pyromantic Brazier at' || true)
    [ -n "$INFO_LINE" ] || fail "no /theurgy info reply for $1 $2 $3"
    eval "$(python3 "$PORT/tools/parse_info.py" <<<"$INFO_LINE")"
    [ "${ok:-0}" = 1 ] || fail "the info line did not parse: $INFO_LINE"
    echo "  $INFO_LINE" >&2
}

# Put ITEM x COUNT in the first hotbar slot and select it.
hold() { # hold ITEM COUNT
    ctl command "item replace entity @s hotbar.0 with $1 $2" >/dev/null
    ctl wait 4 >/dev/null
    ctl select 1 >/dev/null
    ctl wait 2 >/dev/null
}

empty_hand() { ctl command "clear @s" >/dev/null; ctl wait 3 >/dev/null; }

# Place BLOCK at X Y Z from the held item: stand two blocks west, aim at the
# block under the target (or at the target's own support) and place.
place_block() { # place_block X Y Z BLOCK SUPPORT_Y
    local x=$1 y=$2 z=$3 block=$4 support=$5
    ctl command "execute in minecraft:the_end run tp @s $((x - 2)).5 $y $z.5" >/dev/null
    ctl wait 6 >/dev/null
    hold "$block" 1
    local aim reply name
    aim=$(ctl look-at-block "$x" "$support" "$z" up 2>&1) \
        || fail "cannot aim at $x,$support,$z: $aim"
    reply=$(ctl place "$x" "$y" "$z" --item "$block" 2>&1) \
        || fail "ctl place at $x,$y,$z failed: $(tr -d '\n' <<<"$reply" | cut -c1-300)"
    ctl wait 6 >/dev/null
    name=$(block_name "$x" "$y" "$z")
    [ "$name" = "$block" ] || fail "$block did not place at $x,$y,$z (got '$name')"
}

# A brazier at X 101 Z and its oven above at X 102 Z. Returns once both read back.
place_pair() { # place_pair X Z
    place_block "$1" 101 "$2" "$BRAZIER" 100
    place_block "$1" 102 "$2" "$OVEN" 101
}

# Put one fuel item into a brazier's slot through the container path — the
# hopper the tooltip documents. A click would work too, but `ctl interact` can
# deliver a second use in the same tick once the first consumed the held stack,
# and that second click is an empty-hand take (upstream's own rule).
fuel() { # fuel X Y Z ITEM
    ctl container "$1" "$2" "$3" "$4" 1 >/dev/null
    ctl wait 6 >/dev/null
}

# Right-click a block with what the player holds. The player stands on the
# arena floor two blocks west and lets `ctl` pick the visible face: the oven at
# y=102 is clicked on its side, the brazier at y=101 on its top.
click() { # click X Y Z
    ctl command "execute in minecraft:the_end run tp @s $(($1 - 2)).5 101 $3.5" >/dev/null
    ctl wait 4 >/dev/null
    ctl look-at-block "$1" "$2" "$3" >/dev/null
    ctl interact "$1" "$2" "$3" >/dev/null
    ctl wait 4 >/dev/null
}

# The control for EMBEDDED=1: the same binary (every hook is in it) with the
# embedded content made unavailable — `.horizon-cache` is a plain file, so the
# helper can neither compare nor write its content file and falls back to
# `Continue`. Both ids must then be unknown; if they resolve, the run proves
# nothing about the embedded file.
control_leg() {
    echo "== control: the same game with no embedded content file =="
    rm -rf "$SCRATCH/.horizon-cache"
    : > "$SCRATCH/.horizon-cache"
    launch "$CONTROL_WORLD"
    arena
    ctl command "item replace entity @s hotbar.0 with $OVEN 1" >/dev/null || true
    ctl wait 4 >/dev/null
    ctl inventory | jq -e --arg id "$OVEN" '[.hotbar[], .main[] | select(.item == $id)] | length == 0' >/dev/null \
        || fail "the control game has the oven item, so the content did not come from the embedded file"
    ctl command "execute in minecraft:the_end run setblock $SX 101 0 $BRAZIER" >/dev/null || true
    ctl wait 4 >/dev/null
    local name
    name=$(block_name "$SX" 101 0)
    [ "$name" = "minecraft:air" ] || fail "the control game holds $name, so the block id was known"
    [ -f "$SCRATCH/.horizon-cache" ] || fail "the control's .horizon-cache was replaced: the helper wrote its content"
    echo "  without the embedded file: no oven item, the block stayed $name"
    ctl quit >/dev/null 2>&1 || true
    rm -f "$SCRATCH/.horizon-cache"
}

# EMBEDDED=1: the bundle must have written its content out at startup, byte for
# byte the delivered content mod.
embedded_content_check() {
    local cache=$SCRATCH/.horizon-cache/theurgy-embedded-content.hmod
    [ -f "$cache" ] || fail "the bundle did not write its embedded content to $cache"
    cmp -s "$cache" "$PORT/content/theurgy.hmod" \
        || fail "the embedded content differs from content/theurgy.hmod"
    echo "  embedded content: $(wc -c < "$cache" | tr -d ' ') B at $cache, identical to content/theurgy.hmod"
}

echo "== launch (The End arena, content + native bundle) =="
if [ "$CONTROL" = 1 ]; then control_leg; fi
launch
if [ "$EMBEDDED" = 1 ]; then embedded_content_check; fi
arena

echo "== A: the blocks and the salts register, and an empty oven is idle =="
ctl give "$OVEN" 1 >/dev/null
ctl give "$BRAZIER" 1 >/dev/null
ctl give "$STRATA" 1 >/dev/null
ctl wait 6 >/dev/null
ctl inventory | jq -e --arg a "$OVEN" --arg b "$BRAZIER" --arg c "$STRATA" \
    '[.hotbar[], .main[] | .item] as $ids
     | ($ids | index($a)) != null and ($ids | index($b)) != null and ($ids | index($c)) != null' >/dev/null \
    || fail "the theurgy ids did not register: $(ctl inventory | jq -c '[.hotbar[], .main[] | .item | select(. != null)]')"
place_pair "$SX" 0
info "$SX" 102 0
[ "$station" = oven ] || fail "the oven's info did not answer for the oven"
[ "$lit" = false ] || fail "the unlit oven reads lit=$lit"
[ "$progress" = 0 ] && [ "$total" = 100 ] || fail "the fresh oven reads progress $progress/$total, expected 0/100"
[ -z "${input:-}" ] && [ -z "${output:-}" ] || fail "the fresh oven has items: input ${input:-} output ${output:-}"
echo "  placed $BRAZIER at y=101 and $OVEN at y=102, empty and dark"

echo "== B: COLD — one blackstone in an unheated oven never progresses =="
hold minecraft:blackstone 1
click "$SX" 102 0
ctl wait 240 >/dev/null
info "$SX" 102 0
[ "${input:-}" = "minecraft:blackstone" ] && [ "$input_count" = 1 ] \
    || fail "the cold oven lost its input: ${input:-none} x${input_count:-0}"
[ -z "${output:-}" ] || fail "the cold oven made ${output:-}"
[ "$progress" = 0 ] || fail "the cold oven progressed to $progress"
[ "$lit" = false ] || fail "the unheated oven reads lit=$lit"
echo "  240 ticks cold: input $input x$input_count, no output, progress 0, lit=$lit"

echo "== C: HEAT — one coal lights the brazier, the oven follows, 1 blackstone → 1 strata =="
fuel "$SX" 101 0 minecraft:coal
ctl wait 40 >/dev/null
info "$SX" 101 0
[ "$station" = brazier ] || fail "the brazier's info did not answer for the brazier"
[ "$lit" = true ] || fail "the fuelled brazier is not lit"
REMAINING_SEEN=$remaining
info "$SX" 102 0
[ "$lit" = true ] || fail "the oven did not take the heat from the lit brazier"
ctl wait 140 >/dev/null
info "$SX" 102 0
[ "${output:-}" = "$STRATA" ] && [ "$output_count" = 1 ] \
    || fail "the oven's output is ${output:-none} x${output_count:-0}, expected one strata salt"
[ -z "${input:-}" ] || fail "the blackstone was not consumed: input ${input:-} x${input_count:-0}"
[ "$progress" = 0 ] || fail "progress did not reset after the craft: $progress"
echo "  coal lit the brazier (remaining $REMAINING_SEEN), the oven followed, 1 blackstone became 1 strata salt"

echo "== D: TAKE — an empty hand takes the output first =="
empty_hand
click "$SX" 102 0
ctl inventory | jq -e --arg id "$STRATA" '[.main[], .hotbar[] | select(.item == $id) | .count] | add == 1' >/dev/null \
    || fail "the strata salt did not reach the inventory: $(ctl inventory | jq -c '[.main[], .hotbar[] | select(.item != null) | "\(.item) x\(.count)"]')"
info "$SX" 102 0
[ -z "${output:-}" ] || fail "the output slot still holds ${output:-}"
echo "  one strata salt taken into the inventory, the output slot is empty"

echo "== E: RATIO — one packed ice is nine strata salts =="
place_pair "$SX" 4
fuel "$SX" 101 4 minecraft:coal
hold minecraft:packed_ice 1
click "$SX" 102 4
ctl wait 150 >/dev/null
info "$SX" 102 4
EXPECTED=9
if [ "$SELFTEST" = 1 ]; then
    echo "(selftest: asserting 8)"
    EXPECTED=8
fi
[ "${output:-}" = "$STRATA" ] && [ "$output_count" = "$EXPECTED" ] \
    || fail "one packed ice gave ${output:-none} x${output_count:-0}, expected $EXPECTED strata salts"
[ -z "${input:-}" ] || fail "the packed ice was not consumed: ${input:-} x${input_count:-0}"
echo "  one packed ice → $output_count strata salts (the jar's count)"

echo "== E2: TAG — a gem matches the declared #c:gems tag: one diamond is two mineral salts =="
say "theurgy recipe minecraft:diamond"
ctl wait 12 >/dev/null
RECIPE_LINE=$(ctl messages 12 | jq -r '.chat[]' | grep -m1 'calcines to' || true)
[ -n "$RECIPE_LINE" ] || fail "no /theurgy recipe reply for a diamond"
case "$RECIPE_LINE" in
    *"alchemical_salt_mineral x2"*) echo "  $RECIPE_LINE" >&2 ;;
    *) fail "the #c:gems tag did not answer: $RECIPE_LINE" ;;
esac
place_pair "$SX" 12
fuel "$SX" 101 12 minecraft:coal
hold minecraft:diamond 1
click "$SX" 102 12

ctl wait 150 >/dev/null
info "$SX" 102 12
[ "${output:-}" = "$MINERAL" ] && [ "$output_count" = 2 ] \
    || fail "one diamond gave ${output:-none} x${output_count:-0}, expected two mineral salts through #c:gems"
[ -z "${input:-}" ] || fail "the diamond was not consumed: ${input:-} x${input_count:-0}"
echo "  one diamond → $output_count mineral salts (the port's own #c:gems tag)"

echo "== F: COUNT — two plant salts make one creature salt, so five leave three =="
place_pair "$SX" -4
fuel "$SX" 101 -4 minecraft:coal
hold "$PLANT" 5
click "$SX" 102 -4
ctl wait 150 >/dev/null
info "$SX" 102 -4
[ "${output:-}" = "$CREATURE" ] && [ "$output_count" = 1 ] \
    || fail "two plant salts gave ${output:-none} x${output_count:-0}, expected one creature salt"
[ "${input:-}" = "$PLANT" ] && [ "$input_count" = 3 ] \
    || fail "the input is ${input:-none} x${input_count:-0}, expected three plant salts left of five"
echo "  five plant salts → 3 left + 1 creature salt (the recipe's count: 2)"

echo "== G: FUEL — one stick heats for 100 ticks, then both blocks go dark =="
place_pair "$SX" 8
fuel "$SX" 101 8 minecraft:stick
ctl wait 10 >/dev/null
info "$SX" 101 8
[ "$lit" = true ] || fail "the stick did not light the brazier"
[ "$remaining" -gt 0 ] && [ "$remaining" -le 100 ] \
    || fail "the stick's heat reads $remaining ticks, expected 1..100"
ctl wait 130 >/dev/null
info "$SX" 101 8
[ "$lit" = false ] || fail "the brazier is still lit after the stick burnt out ($remaining ticks left)"
info "$SX" 102 8
[ "$lit" = false ] || fail "the oven is still lit after the fuel ran out"
echo "  stick: lit at first, dark again after 140 ticks (brazier and oven)"

echo "PASS: an unheated oven never progresses, one coal lights the brazier and the oven follows within its 20-tick heat sample, one blackstone becomes one strata salt in 100 heated ticks, an empty hand takes the output, one packed ice is nine strata salts, two plant salts make one creature salt, and one stick heats for 100 ticks before both blocks go dark"
