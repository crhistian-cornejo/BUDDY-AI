#!/usr/bin/env python3
"""Buddy's base character: a chibi in a mint mochi hood with a sprout, 48x48.

Writes core/characters/buddy-base.json (the source the core loads). With --preview DIR it also writes a PNG sheet
and one animated GIF per state. The JSON is what ships; this script is how it is drawn: every state is a list of
poses (eyes, arms, legs, mouth, bob, a symbol), so new states are added in STATES below. Needs Pillow.
"""
import json
import sys
from dataclasses import dataclass, replace
from pathlib import Path

N = 48
OY = 3  # the whole figure sits this many rows down, leaving room for the sprout
PAL = {
    ".": None,
    "o": "#1E4D3C",  # outline
    "d": "#3E9474",  # deep shade
    "B": "#5FBF98",  # shade
    "b": "#86DDB8",  # mint
    "l": "#B9F0D6",  # light
    "s": "#FCE6CC",  # skin
    "S": "#EDC7A3",  # skin shade
    "k": "#1A1A22",  # eyes
    "w": "#FFFFFF",  # glint
    "c": "#FF9AAE",  # blush
    "m": "#B5735A",  # mouth
    "g": "#8ED66B",  # leaf
    "G": "#4E9E3C",  # leaf shade, stem
    "L": "#C8F29E",  # leaf light
    "y": "#FFD45C",  # symbols: ? and star
    "r": "#FF5D6C",  # symbols: !
    # The laptop (a prop, never recoloured by an agent's look): silver lid, its sheen and hinge shade, and the
    # screen's cool light on Buddy's face. No logo on the lid.
    "X": "#C3C9D2",  # silver
    "x": "#E6EAF0",  # silver sheen
    "h": "#8D95A1",  # silver shade, hinge
    "e": "#D2E8FF",  # screen glow on the skin
}


@dataclass(frozen=True)
class Pose:
    eyes: str = "open"  # open closed left right up down happy wide x half half-left half-right
    mouth: str = "smile"  # smile o flat yawn
    head: int = 0  # head bob (rows down)
    lift: int = 0  # whole figure up (rows), for jumps
    shake: int = 0  # whole figure sideways (columns)
    left_arm: str = "down"  # down up
    right_arm: str = "down"  # down out up
    hands: int = 0  # typing: 1 lifts the left hand, 2 the right
    left_leg: int = 0  # rows lifted
    right_leg: int = 0
    left_step: int = 0  # columns the foot moves (walking stride)
    right_step: int = 0
    dangle: bool = False  # legs hang (being dragged)
    sit: int = 0  # 0 standing, 1 crouching (sitting down / getting up), 2 sitting with the feet forward
    glasses: int = 0  # 1 round glasses over the eyes
    laptop: int = 0  # in front, seen from behind (sitting only): 1 closed, 2 lid half up, 3 open
    glow: int = 0  # the open screen lights the face: 1 soft, 2 brighter (flicker, thinking pulse)
    chin: bool = False  # at the laptop: the right hand rests on the chin (thinking)
    symbol: str = ""  # dots1 dots2 dots3 question question2 exclaim star


def rr(x, y, x0, y0, x1, y1, r):
    """Inside a rounded rect (inclusive pixel bounds)."""
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r + r * 0.6


def shape(g, mask, fill, outline="o"):
    for y in range(N):
        for x in range(N):
            if not mask(x, y):
                continue
            edge = any(not mask(x + dx, y + dy) for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))
            g[y][x] = outline if edge else fill(x, y)


def noise(x, y):
    return ((x * 73856093) ^ (y * 19349663)) % 11


def put(g, x, y, ch):
    if 0 <= x < N and 0 <= y < N:
        g[y][x] = ch


def outline_points(g, pts):
    for x, y in list(pts):
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            nx, ny = x + dx, y + dy
            if 0 <= nx < N and 0 <= ny < N and (nx, ny) not in pts and g[ny][nx] == ".":
                g[ny][nx] = "o"


def draw(p: Pose):
    g = [["."] * N for _ in range(N)]
    bd = (0, 2, 3)[p.sit]  # sitting lowers everything above the feet
    dy = p.head + bd

    # legs (lifted ones are shorter; dangling ones hang lower and apart); sitting draws the feet after the body
    for i, lx in enumerate((18, 26) if p.sit != 2 else ()):
        lift = p.left_leg if i == 0 else p.right_leg
        lx += p.left_step if i == 0 else p.right_step
        if p.dangle:
            lx += -1 if i == 0 else 1
        y1 = 43 - lift + (1 if p.dangle else 0)
        if p.sit == 1:  # crouching: short bent legs, feet a little apart
            lx += -1 if i == 0 else 1
            shape(g, lambda x, y, lx=lx: rr(x, y, lx, 39, lx + 4, 43, 2),
                  lambda x, y, lx=lx: "d" if y >= 42 else ("B" if x == lx + 3 else "b"))
            continue
        if lift:  # a lifted foot rises whole, so it still shows under the body
            shape(g, lambda x, y, lx=lx, y1=y1: rr(x, y, lx, 38 - lift, lx + 4, y1, 2),
                  lambda x, y, lx=lx, y1=y1: "d" if y >= y1 - 2 else ("B" if x == lx + 3 else "b"))
            continue
        shape(g, lambda x, y, lx=lx, y1=y1: rr(x, y, lx, 38, lx + 4, y1, 2),
              lambda x, y, lx=lx, y1=y1: "d" if y >= y1 - 2 else ("B" if x == lx + 3 else "b"))

    # arms resting at the sides (mint sleeve, cream hand); typing lifts a hand
    def side_arm(ax, lifted):
        y0, y1 = 31 + bd - lifted, 37 + bd - lifted
        shape(g, lambda x, y: rr(x, y, ax, y0, ax + 4, y1, 2),
              lambda x, y: ("S" if y >= y1 - 1 else "s") if y >= y1 - 2
              else ("B" if (x == ax + 3 if ax > 20 else x == ax + 1) else "b"))

    if p.left_arm == "down" and not p.laptop:
        side_arm(12, 1 if p.hands == 1 else 0)
    if p.right_arm == "down" and not p.laptop:
        side_arm(32, 1 if p.hands == 2 else 0)

    # body (mint onesie) with a cream belly
    low = 41 + bd - (1 if p.sit == 2 else 0)
    shape(g, lambda x, y: rr(x, y, 15, 29 + bd, 32, low, 4),
          lambda x, y: "d" if y >= low - 1 else ("B" if x >= 30 or y >= 38 + bd else ("l" if x <= 16 else "b")))
    for y in range(33 + bd, 39 + bd):
        for x in range(19, 29):
            if rr(x, y, 19, 33 + bd, 28, 38 + bd, 3):
                g[y][x] = "S" if (y >= 37 + bd or x >= 27) else "s"
    if p.sit == 2:
        seated_feet(g, p.left_leg, p.right_leg)

    # hood (head), over the body; `head` bobs it down
    def hood(x, y):
        t = (x - 9) / 29 * 0.45 + (y - 8 - dy) / 23 * 0.9
        n = noise(x, y)
        tone = "l" if t < 0.28 else ("b" if t < 0.82 else ("B" if t < 1.12 else "d"))
        if n == 0 and tone == "b":
            tone = "l"
        if n == 1 and tone in "bl":
            tone = "B"
        return tone

    shape(g, lambda x, y: rr(x, y, 9, 8 + dy, 38, 31 + dy, 7), hood)
    for x, y in ((13, 12 + dy), (14, 12 + dy), (12, 13 + dy), (12, 14 + dy)):
        g[y][x] = "w"

    # face window: recessed rim, skin inside
    F = (15, 15 + dy, 32, 28 + dy, 4)
    for y in range(N):
        for x in range(N):
            if rr(x, y, *F):
                if not rr(x - 1, y, *F) or not rr(x, y - 1, *F):
                    g[y][x] = "d"
                elif not rr(x + 1, y, *F) or not rr(x, y + 1, *F):
                    g[y][x] = "B"
                else:
                    g[y][x] = "S" if (x >= F[2] - 2 or y >= F[3] - 1) else "s"

    if p.glasses:
        draw_glasses(g, 19 + dy)
    draw_eyes(g, p.eyes, 19 + dy)
    ey = 19 + dy
    for bx in (17, 18, 29, 30):
        g[ey + 5][bx] = "c"
    for bx in (17, 30):
        g[ey + 4][bx] = "c"
    if p.mouth == "smile":
        g[ey + 5][23] = g[ey + 5][24] = "m"
    elif p.mouth == "o":
        for x, y in ((23, ey + 5), (24, ey + 5), (23, ey + 6), (24, ey + 6)):
            g[y][x] = "m"
    elif p.mouth == "yawn":  # a small yawn: 4 wide, 3 open rows, dark inside
        for x in (23, 24):
            g[ey + 5][x] = g[ey + 8][x] = "m"
        for y in (ey + 6, ey + 7):
            g[y][22] = g[y][25] = "m"
            g[y][23] = g[y][24] = "k"
    else:  # flat
        g[ey + 5][22] = g[ey + 5][23] = g[ey + 5][24] = g[ey + 5][25] = "m"

    if p.laptop:
        laptop_front(g, p, F)

    # raised arms go outside the hood
    if p.left_arm == "up":
        raised_arm(g, 4, 20 + dy, mirror=True)
    if p.right_arm == "up":
        raised_arm(g, 39, 20 + dy)
    elif p.right_arm == "out":
        shape(g, lambda x, y: rr(x, y, 37, 26 + dy, 43, 31 + dy, 2),
              lambda x, y: "s" if x >= 41 else ("B" if y >= 30 + dy else "b"))

    draw_sprout(g, 8 + dy)

    # whole-figure moves, then the symbol (in final coordinates, top right)
    lift = OY - p.lift
    g = [["."] * N for _ in range(lift)] + g[: N - lift] if lift >= 0 else g[-lift:] + [["."] * N for _ in range(-lift)]
    if p.shake:
        g = [(row[-p.shake:] + row[:-p.shake]) if p.shake > 0 else (row[-p.shake:] + row[:-p.shake]) for row in g]
    draw_symbol(g, p.symbol)
    return ["".join(r) for r in g]


def seated_feet(g, left_lift=0, right_lift=0):
    """Sitting, feet forward and splayed: two soft mint feet beside the belly, cream soles toward the viewer."""
    for fx, lift in ((11, left_lift), (30, right_lift)):
        y0, y1 = 39 - lift, 44 - lift
        shape(g, lambda x, y, fx=fx, y0=y0, y1=y1: rr(x, y, fx, y0, fx + 6, y1, 2),
              lambda x, y, fx=fx, y1=y1: "B" if (y == y1 - 1 or x == fx + 5) else "b")
        for y in (y0 + 2, y0 + 3):  # sole pad
            for x in range(fx + 2, fx + 5):
                g[y][x] = "S" if y == y0 + 3 else "s"


def draw_glasses(g, ey):
    """Round dark frames around both eyes, a bridge between them and a glint on each lens."""
    for ex in (19, 27):
        for x in range(ex - 1, ex + 3):
            g[ey - 1][x] = g[ey + 4][x] = "k"
        for y in range(ey, ey + 4):
            g[y][ex - 2] = g[y][ex + 3] = "k"
        g[ey][ex - 1] = "w"
    g[ey + 1][23] = g[ey + 1][24] = "k"
    g[ey + 1][16] = g[ey + 1][31] = "k"


LID_TOP = {1: 40, 2: 37, 3: 32}  # the lid's top row (before the figure's offset) for each laptop stage
LID_X = (15, 32)  # 18 × 11 when open: a laptop's proportions


def laptop_front(g, p, F):
    """A MacBook-style laptop in front of seated Buddy, seen from behind: a thin silver lid with rounded top
    corners, a sheen and a darker hinge, on a thin base. Blank lid: no logo, no mark. The paws reach the keys from
    both sides of the lid (a typing paw rises a row); the open screen lights the bottom of the face."""
    top = LID_TOP[p.laptop]
    x0, x1 = LID_X
    if p.laptop == 3 and p.glow:
        # The screen's cool light on the chin (two rows when it flickers brighter), and on the glasses' glints.
        for y in range(F[3] - p.glow, F[3]):  # the rows above the face window's rim
            for x in range(F[0], F[2] + 1):
                if g[y][x] in "sS":
                    g[y][x] = "e"
        if p.glasses and p.glow == 2:
            for y in range(F[1], F[3]):
                for x in range(F[0], F[2] + 1):
                    if g[y][x] == "w":
                        g[y][x] = "e"
    # arms: mint sleeves down the sides, cream paws at the lid's edges (the lid hides the inner half)
    for i, ax in enumerate((11, 32)):
        if i == 1 and p.chin:
            continue
        up = 1 if (p.hands == 1 and i == 0) or (p.hands == 2 and i == 1) else 0
        y0, y1 = 35 - up, 41 - up
        shape(g, lambda x, y, ax=ax, y0=y0, y1=y1: rr(x, y, ax, y0, ax + 5, y1, 2),
              lambda x, y, ax=ax, y1=y1: ("S" if y >= y1 - 1 else "s") if y >= y1 - 3
              else ("B" if x == (ax + 1 if i == 0 else ax + 4) else "b"))
    if p.chin:  # the right paw props the chin; its sleeve goes down behind the lid
        shape(g, lambda x, y: rr(x, y, 28, F[3] - 1, 33, top + 2, 2),
              lambda x, y: "B" if x >= 32 else "b")
        shape(g, lambda x, y: rr(x, y, 25, F[3] - 4, 30, F[3], 2),
              lambda x, y: "S" if (x == 29 or y == F[3] - 1) else "s")
    # the lid
    for y in range(top, 43):
        for x in range(x0, x1 + 1):
            if y == top and x in (x0, x1):
                continue  # rounded top corners
            if y in (top, 42) or x in (x0, x1):
                g[y][x] = "k"
            elif y == 41 and top < 40:
                g[y][x] = "h"  # hinge
            elif y == top + 1 or x == x0 + 1 or (p.laptop == 3 and (x - x0) + (y - top) in (5, 6) and y <= top + 5):
                g[y][x] = "x"  # sheen along the top and left, and a diagonal glint
            elif x == x1 - 1:
                g[y][x] = "h"
            else:
                g[y][x] = "X"
    # the base: a thin slab, a pixel wider than the lid on each side
    for x in range(x0 - 1, x1 + 2):
        g[43][x] = "k" if x in (x0 - 1, x1 + 1) else "h"
        g[44][x] = "k"
    g[44][x0 - 1] = g[44][x1 + 1] = "."


def raised_arm(g, ax, ay, mirror=False):
    """An arm up beside the head: sleeve below, cream hand on top."""
    shape(g, lambda x, y: rr(x, y, ax, ay, ax + 4, ay + 8, 2),
          lambda x, y: ("s" if y <= ay + 2 else ("B" if (x == ax + 1 if mirror else x == ax + 3) else "b")))


def draw_eyes(g, eyes, ey):
    for ex in (19, 27):
        if eyes in ("open", "left", "right", "up", "down", "wide"):
            ox = {"left": -1, "right": 1}.get(eyes, 0)
            oy = {"up": -1, "down": 1}.get(eyes, 0)
            height = 5 if eyes == "wide" else 4
            for y in range(ey + oy - (1 if eyes == "wide" else 0), ey + oy + height - (1 if eyes == "wide" else 0)):
                g[y][ex + ox] = g[y][ex + ox + 1] = "k"
            g[ey + oy - (1 if eyes == "wide" else 0)][ex + ox + (1 if eyes == "right" else 0)] = "w"
        elif eyes.startswith("half"):  # heavy lids (bored): a flat lid reaching outwards over the lower half of the eye
            ox = {"half-left": -1, "half-right": 1}.get(eyes, 0)
            outer = ex - 1 if ex < 24 else ex + 2
            for x in (outer, ex, ex + 1):
                g[ey + 1][x] = "k"
            for y in (ey + 2, ey + 3):
                g[y][ex + ox] = g[y][ex + ox + 1] = "k"
        elif eyes == "closed":
            for x in range(ex - 1, ex + 3):
                g[ey + 3][x] = "k"
        elif eyes == "happy":  # ^ ^
            for x, y in ((ex - 1, ey + 3), (ex, ey + 2), (ex + 1, ey + 2), (ex + 2, ey + 3)):
                g[y][x] = "k"
        elif eyes == "x":
            for d in range(4):
                g[ey + d][ex - 1 + d] = "k"
                g[ey + d][ex + 2 - d] = "k"


def draw_sprout(g, top):
    leaves = {
        "left": [(x, y) for y in range(top - 8, top - 2) for x in range(15, 24)
                 if abs(x - 19.0) <= 4.6 and abs(y - (top - 5.0)) <= 2.6 * (1 - ((x - 19.0) / 4.6) ** 2) + 0.4],
        "right": [(x, y) for y in range(top - 11, top - 2) for x in range(24, 34)
                  if abs(x - 28.5) <= 5.2 and abs(y - (top - 6.5)) <= 2.9 * (1 - ((x - 28.5) / 5.2) ** 2) + 0.4],
    }
    stem = {(23, y) for y in range(top - 3, top)} | {(24, y) for y in range(top - 3, top)}
    pts = set(p for v in leaves.values() for p in v) | stem
    for x, y in pts:
        if 0 <= y < N:
            g[y][x] = "g"
    for x, y in leaves["left"]:
        if y == top - 5 and 16 <= x <= 22:
            g[y][x] = "G"
        elif y <= top - 7:
            g[y][x] = "L"
    for x, y in leaves["right"]:
        if y == top - 6 and 25 <= x <= 32:
            g[y][x] = "G"
        elif y <= top - 8:
            put(g, x, y, "L")
    for y in range(top - 3, top + 1):
        g[y][23] = g[y][24] = "G"
    outline_points(g, {(x, y) for x, y in pts if 0 <= y < N})


GLYPHS = {
    # 5x7, drawn in the symbol color with an outline
    "question": [".###.", "#...#", "....#", "..##.", "..#..", ".....", "..#.."],
    "exclaim": ["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#.."],
    "star": [".....", "..#..", ".###.", "#####", ".###.", ".#.#.", "....."],
}


def draw_symbol(g, symbol):
    if not symbol:
        return
    if symbol.startswith("dots"):
        for i in range(int(symbol[-1])):
            x = 37 + i * 3
            for dx, dy in ((0, 0), (1, 0), (0, 1), (1, 1)):
                g[11 + dy][x + dx] = "o"
        return
    name = symbol.rstrip("2")
    color = "r" if name == "exclaim" else "y"
    top = 2 if symbol.endswith("2") else 3
    pts = set()
    for r, row in enumerate(GLYPHS[name]):
        for c, ch in enumerate(row):
            if ch == "#":
                pts.add((39 + c, top + r))
                g[top + r][39 + c] = color
    outline_points(g, pts)


I = Pose()
SIT = Pose(sit=2, eyes="half", mouth="flat")
LAP = Pose(sit=2, eyes="down", mouth="flat", glasses=1)  # seated at the laptop
STATES = {
    # name: (fps, poses). Every state but idle is played by the app and then returns to idle.
    "idle": (4, [I, replace(I, hands=1), replace(I, eyes="left", hands=1),
                 replace(I, eyes="closed"), I, replace(I, eyes="right", hands=2),
                 replace(I, hands=2), I]),
    "blink": (10, [replace(I, eyes="closed")]),
    "look": (2, [replace(I, eyes="left"), replace(I, eyes="left"), replace(I, eyes="right"), replace(I, eyes="right")]),
    "wave": (6, [
        replace(I, eyes="happy", right_arm="out"),
        replace(I, eyes="happy", right_arm="up"),
        replace(I, eyes="happy", right_arm="out"),
        replace(I, eyes="happy", right_arm="up"),
        replace(I, eyes="happy", right_arm="out"),
        replace(I, eyes="happy", right_arm="up"),
    ]),
    "walk-right": (8, [
        replace(I, eyes="right", lift=1, left_leg=1, left_step=2, right_step=-1),
        replace(I, eyes="right", head=1),
        replace(I, eyes="right", lift=1, right_leg=1, right_step=2, left_step=-1),
        replace(I, eyes="right", head=1),
    ]),
    "walk-left": (8, [
        replace(I, eyes="left", lift=1, right_leg=1, right_step=-2, left_step=1),
        replace(I, eyes="left", head=1),
        replace(I, eyes="left", lift=1, left_leg=1, left_step=-2, right_step=1),
        replace(I, eyes="left", head=1),
    ]),
    "drag": (4, [
        replace(I, eyes="wide", mouth="o", left_arm="up", right_arm="up", dangle=True),
        replace(I, eyes="wide", mouth="o", left_arm="up", right_arm="up", dangle=True, head=1),
    ]),
    "think": (3, [
        replace(I, eyes="up", mouth="flat", symbol="dots1"),
        replace(I, eyes="up", mouth="flat", symbol="dots2"),
        replace(I, eyes="up", mouth="flat", symbol="dots3"),
        replace(I, eyes="up", mouth="flat"),
    ]),
    "work": (8, [
        replace(I, eyes="down", mouth="flat", hands=1),
        replace(I, eyes="down", mouth="flat"),
        replace(I, eyes="down", mouth="flat", hands=2),
        replace(I, eyes="down", mouth="flat"),
    ]),
    "ask": (4, [
        replace(I, eyes="open", symbol="question"),
        replace(I, eyes="open", symbol="question2"),
        replace(I, eyes="open", head=1, symbol="question2"),
        replace(I, eyes="open", head=1, symbol="question"),
    ]),
    "error": (8, [
        replace(I, eyes="x", mouth="flat", symbol="exclaim", shake=1),
        replace(I, eyes="x", mouth="flat", symbol="exclaim", shake=-1),
        replace(I, eyes="x", mouth="flat", symbol="exclaim"),
    ]),
    "done": (8, [
        replace(I, eyes="happy", mouth="o", head=1),
        replace(I, eyes="happy", mouth="o", lift=2, left_arm="up", right_arm="up", symbol="star"),
        replace(I, eyes="happy", mouth="o", lift=3, left_arm="up", right_arm="up", symbol="star"),
        replace(I, eyes="happy", mouth="o", lift=2, left_arm="up", right_arm="up", symbol="star"),
        replace(I, eyes="happy", symbol="star"),
    ]),
    # Seated and bored (after a while without being used; PetBrain decides). `sit` is the still frame held in
    # between; the others are short, sparse moves that end back on it. Buddy dozes off seated too.
    "sit-down": (8, [replace(I, sit=1, mouth="flat"), replace(SIT, eyes="open", head=1), SIT]),
    "stand-up": (8, [replace(I, sit=1), I]),
    "sit": (1, [SIT]),
    "sit-blink": (4, [replace(SIT, eyes="closed"), replace(SIT, eyes="closed")]),
    "sit-look": (2, [replace(SIT, eyes="half-left"), replace(SIT, eyes="half-left"),
                     replace(SIT, eyes="half-right"), replace(SIT, eyes="half-right")]),
    "sit-yawn": (4, [replace(SIT, mouth="o"), replace(SIT, eyes="closed", mouth="yawn", head=-1),
                     replace(SIT, eyes="closed", mouth="yawn", head=-1), replace(SIT, eyes="closed", mouth="yawn"),
                     replace(SIT, eyes="closed", mouth="o")]),
    "sit-swing": (6, [replace(SIT, right_leg=1), replace(SIT, right_leg=2), replace(SIT, right_leg=1), SIT,
                      replace(SIT, right_leg=1), replace(SIT, right_leg=2), replace(SIT, right_leg=1)]),
    # Working at the laptop: glasses on, sits down behind a silver laptop, opens it and types (6 fps: alternating
    # paws, a small nod, the screen's light flickering on the face); thinking rests a paw on the chin while the
    # glow pulses. Opening and closing take two frames each.
    "laptop-on": (8, [replace(I, glasses=1, sit=1, mouth="flat"),
                      replace(LAP, laptop=1, head=1),
                      replace(LAP, laptop=2),
                      replace(LAP, laptop=3, glow=1, hands=1)]),
    "laptop-off": (8, [replace(LAP, laptop=2, eyes="open"),
                       replace(LAP, laptop=1, eyes="open", head=1),
                       replace(I, glasses=1, sit=1, mouth="flat")]),
    "laptop-type": (6, [replace(LAP, laptop=3, glow=1, hands=1),
                        replace(LAP, laptop=3, glow=1, hands=2),
                        replace(LAP, laptop=3, glow=2, hands=1, head=1),
                        replace(LAP, laptop=3, glow=1, hands=2, head=1),
                        replace(LAP, laptop=3, glow=1, hands=1, eyes="half"),
                        replace(LAP, laptop=3, glow=1)]),
    "laptop-think": (3, [replace(LAP, laptop=3, glow=1, chin=True, eyes="up", symbol="dots1"),
                         replace(LAP, laptop=3, glow=2, chin=True, eyes="up", symbol="dots2"),
                         replace(LAP, laptop=3, glow=2, chin=True, eyes="up", symbol="dots3"),
                         replace(LAP, laptop=3, glow=1, chin=True, eyes="up")]),
    "sleep": (1, [
        replace(SIT, eyes="closed"),
        replace(SIT, eyes="closed", head=1),
    ]),
}


def main():
    states = {name: {"fps": fps, "frames": [draw(p) for p in poses]} for name, (fps, poses) in STATES.items()}
    # The square the apps crop as Buddy's avatar (hood, face and sprout), in idle frame 0.
    face = {"x": 7, "y": 1, "size": 34}
    data = {"id": "buddy-base", "name": "Buddy", "size": N, "face": face, "palette": PAL, "states": states}
    root = Path(__file__).resolve().parents[2]
    out = root / "core/characters/buddy-base.json"
    out.write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n")
    print(f"→ {out}")
    if "--preview" in sys.argv:
        preview(states, face, Path(sys.argv[sys.argv.index("--preview") + 1]))


def preview(states, face, folder):
    from PIL import Image, ImageDraw

    def render(frame, scale, bg=(250, 250, 250)):
        img = Image.new("RGB", (N * scale, N * scale), bg)
        d = ImageDraw.Draw(img)
        for r, row in enumerate(frame):
            for c, ch in enumerate(row):
                if PAL[ch]:
                    d.rectangle([c * scale, r * scale, c * scale + scale - 1, r * scale + scale - 1], fill=PAL[ch])
        return img

    folder.mkdir(parents=True, exist_ok=True)
    rows = list(states.items())
    cols = max(len(s["frames"]) for _, s in rows)
    scale = 4
    sheet = Image.new("RGB", (cols * N * scale + 140, len(rows) * N * scale), (250, 250, 250))
    d = ImageDraw.Draw(sheet)
    for i, (name, s) in enumerate(rows):
        d.text((6, i * N * scale + N * scale // 2), name, fill=(40, 40, 40))
        for j, f in enumerate(s["frames"]):
            sheet.paste(render(f, scale), (140 + j * N * scale, i * N * scale))
        frames = [render(f, 6) for f in s["frames"]]
        frames[0].save(folder / f"{name}.gif", save_all=True, append_images=frames[1:],
                       duration=int(1000 / s["fps"]), loop=0)
    sheet.save(folder / "sheet.png")
    f = render(states["idle"]["frames"][0], 8)
    f.crop((face["x"] * 8, face["y"] * 8, (face["x"] + face["size"]) * 8, (face["y"] + face["size"]) * 8)).save(folder / "face.png")


if __name__ == "__main__":
    main()
