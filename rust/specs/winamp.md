# Winamp Classic Player — UI Design Specification v1.0

> Pixel-perfect clone reference. All dimensions in CSS pixels at 1:1 scale. Colors are 6-digit hex.

---

## Table of Contents

1. [Overall Dimensions](#1-overall-dimensions)
2. [Outer Shell & Window Chrome](#2-outer-shell--window-chrome)
3. [LCD Display Module](#3-lcd-display-module)
4. [Timer Zone (7-Segment Display)](#4-timer-zone-7-segment-display)
5. [Spectrum Analyzer](#5-spectrum-analyzer)
6. [Metadata Panel](#6-metadata-panel)
7. [Sliders](#7-sliders)
8. [Playback Controls Bar](#8-playback-controls-bar)
9. [States Summary](#9-states-summary)
10. [Color Reference](#10-color-reference)
11. [Interaction Specification](#11-interaction-specification)
12. [Critical Layout Constraint Rules](#12-critical-layout-constraint-rules)

---

## 1. Overall Dimensions

| Property | Value |
|---|---|
| Total width | `540px` (fixed, no horizontal overflow) |
| Title bar height | `22px` |
| Main body padding | `6px 8px 8px` |
| Main body row gap | `6px` |
| Top row height | `120px` |
| Controls bar height | `44px` |
| Bottom strip height | `8px` |

---

## 2. Outer Shell & Window Chrome

### 2.1 Title Bar

| Property | Value |
|---|---|
| Height | `22px` |
| Background | `linear-gradient(to bottom, #5a5a7a, #3a3a5a 40%, #2a2a4a 60%, #1e1e3a)` |
| Border | `1px solid #6a6a9a`, top corners `border-radius: 4px` |
| Overflow | `hidden` |
| Logo badge | `14×14px` SVG, left-aligned. Three amber ellipses + two antenna arcs |
| Grip texture | `repeating-linear-gradient(to right, #4a4a7a 0 2px, transparent 2px 4px)`, `opacity: 0.4`. Spans between logo and window buttons |
| Title text | `"WINAMP"`, centered, `Arial Narrow 11px bold`, `#c8c8d8`, `letter-spacing: 3px`, uppercase |

**Window control buttons** (right-aligned, `gap: 5px`):

| Button | Size | Background | Text color |
|---|---|---|---|
| Minimize `_` | `16×12px` | `linear-gradient(to bottom, #bbb, #888)` | `#333` |
| Maximize `□` | `16×12px` | Same as minimize | `#333` |
| Close `×` | `16×12px` | `linear-gradient(to bottom, #cc6655, #882233)` | `#fff` |

All window buttons: `border: 1px solid #888`, `border-radius: 2px`.

---

### 2.2 Main Body Background

5-stop vertical gradient simulating brushed dark plastic:

```
0%   → #3a3a5a
20%  → #454565
50%  → #505075
80%  → #454565
100% → #3a3a5a
```

| Property | Value |
|---|---|
| Border (sides) | `1px solid #6a6aaa` |
| Border (top junction) | `1px solid #4a4a8a` |

---

### 2.3 Bottom Border Strip

| Property | Value |
|---|---|
| Height | `8px` |
| Background | `linear-gradient(to bottom, #2a2a4a, #1e1e38)` |
| Border | `1px solid #5a5a8a`, `border-radius: 0 0 4px 4px`, no top border |

---

## 3. LCD Display Module

Positioned left side of the top row. Simulates a phosphor dot-matrix LCD.

| Property | Value |
|---|---|
| Width | `190px` |
| Height | `120px` |
| Background | `#0d1a0d` |
| Border | `1px solid #2a4a2a` |
| Border radius | `6px` |
| Overflow | `hidden` — all children clipped to panel bounds |
| Dot matrix overlay | `radial-gradient(circle, #152515 0.7px, transparent 0.7px)`, `background-size: 4px 4px`, `opacity: 0.6`, `z-index` above content, `pointer-events: none` |

---

### 3.1 Sidebar Labels

Narrow vertical strip, leftmost column of the LCD panel.

| Property | Value |
|---|---|
| Position | `absolute; left: 5px; top: 6px; bottom: 6px; width: 12px` |
| Layout | `flex-direction: column; justify-content: space-around; align-items: center` |
| Font | `monospace, 9px` |
| Color | `#39ff14` |
| Text shadow | `0 0 4px #39ff1488` |
| Playing state labels | `C O R T O C` |
| Idle / stopped labels | `O A I T D V` |

---

### 3.2 Playback State Controls

Positioned immediately right of the sidebar, top of the LCD.

| Property | Value |
|---|---|
| Position | `absolute; left: 18px; top: 6px` |
| Layout | `flex-direction: row; align-items: center; gap: 3px` |

**State square** (idle/stopped only):

| Property | Value |
|---|---|
| Size | `9×9px` |
| Background | `#cc3300` |
| Border radius | `1px` |
| Shadow | `0 0 3px #cc330088` |
| Visibility | Hidden during playback |

**Play triangle** (always visible):

| Property | Value |
|---|---|
| Shape | CSS border trick: `border-left: 19px solid #39ff14; border-top: 11px solid transparent; border-bottom: 11px solid transparent` |
| Filter | `drop-shadow(0 0 3px #39ff1488)` |
| Order | State square left, triangle right |

---

## 4. Timer Zone (7-Segment Display)

Top-right area of the LCD panel.

- **Playing:** `MM:SS` (elapsed time)
- **Idle/Stopped:** `-MM:SS` (remaining time, prefixed with minus segment)

| Property | Value |
|---|---|
| Position | `absolute; top: 5px; left: 50px; right: 5px; height: 44px` |
| Layout | `flex-direction: row; align-items: center; gap: 2px` |

### 4.1 Digit Segments

Each digit is a `22×43px` container with 7 absolutely-positioned segment bars:

| Segment | Style | Size | Position |
|---|---|---|---|
| Top | Horizontal | `14×5px` | `top: 0; left: 3px` |
| Top-left | Vertical | `5×13px` | `top: 3px; left: 0` |
| Top-right | Vertical | `5×13px` | `top: 3px; right: 0` |
| Middle | Horizontal | `14×5px` | `top: 19px; left: 3px` |
| Bottom-left | Vertical | `5×13px` | `top: 22px; left: 0` |
| Bottom-right | Vertical | `5×13px` | `top: 22px; right: 0` |
| Bottom | Horizontal | `14×5px` | `top: 38px; left: 3px` |

| Property | Value |
|---|---|
| Segment border-radius | `2px` |
| Active color | `#39ff14`, `box-shadow: 0 0 4px #39ff1466` |
| Inactive color | `#1a2e1a` |

### Segment map per digit

```
Digit  top  tl  tr  mid  bl  br  bot
  0     1    1   1    0   1   1   1
  1     0    0   1    0   0   1   0
  2     1    0   1    1   1   0   1
  3     1    0   1    1   0   1   1
  4     0    1   1    1   0   1   0
  5     1    1   0    1   0   1   1
  6     1    1   0    1   1   1   1
  7     1    0   1    0   0   1   0
  8     1    1   1    1   1   1   1
  9     1    1   1    1   0   1   1
```

### 4.2 Colon Separator

| Property | Value |
|---|---|
| Shape | Two filled circles stacked vertically |
| Size | `4px` radius each |
| Color | `#39ff14`, `box-shadow: 0 0 3px #39ff14aa` |
| Container height | `26px`, `margin-top: 8px` |
| Layout | `flex-direction: column; justify-content: space-around` |

### 4.3 Minus Sign (Idle Mode)

| Property | Value |
|---|---|
| Size | `10×5px` |
| Border radius | `2px` |
| Color | `#39ff14` |
| Vertical alignment | `margin-top: 13px` (centers at middle-segment height) |

---

## 5. Spectrum Analyzer

Lower portion of the LCD panel. Strictly bounded by dotted rules — no bar may touch or cross a rule line.

### 5.1 Zone Boundaries

| Property | Value |
|---|---|
| Position | `absolute; left: 18px; right: 5px; top: 52px; bottom: 5px` |
| Overflow | `hidden` |

### 5.2 Dotted Boundary Rules

| Property | Value |
|---|---|
| Dot size | `3×3px`, `border-radius: 0.5px` |
| Dot pitch | `7px` (3px dot + 4px gap) |
| Color A (even) | `#00aaaa` |
| Color B (odd) | `#008888` |
| Left rule | Vertical, full height of spectrum zone, `left: 0` |
| Bottom rule | Horizontal, full width of spectrum zone, `bottom: 0` |

### 5.3 Bars Container

Inset from the rules so bars never touch them:

| Property | Value |
|---|---|
| Position | `absolute; left: 5px; right: 0; top: 0; bottom: 5px` |
| Layout | `display: flex; align-items: flex-end; gap: 2px` |
| Overflow | `hidden` |
| Bar count | `32` |
| Bar width | `(containerWidth − 31 × 2) / 32`, min `4px` |
| Bar border-radius | `1px` |

### 5.4 Bar Color Zones

Rendered as a continuous CSS gradient from bottom to top, painted only up to the current amplitude height:

| Zone | Amplitude range | Color | Hex |
|---|---|---|---|
| 1 — Low green (bottom) | 0% – 20% | Bright green | `#00cc00` |
| 2 — Yellow-green | 20% – 45% | Yellow-green | `#88cc00` |
| 3 — Amber | 45% – 65% | Amber | `#ffaa00` |
| 4 — Orange | 65% – 82% | Dark orange | `#dd6600` |
| 5 — Red (peak) | 82% – 100% | Red | `#cc3300` |

> Zone 5 only appears on bars with very high amplitude. Most bars at normal levels show zones 1–3.

### 5.5 Peak Hold Markers

| Property | Value |
|---|---|
| Size | `bar-width × 2px` tall |
| Color | `#ffffff` |
| Gap above bar | `3px` |
| Hold duration | ~60 animation frames |
| Decay rate | `1.2px` per frame after hold expires |
| Hidden when | `peakHeight <= 2px` (`opacity: 0`) |

---

## 6. Metadata Panel

Flex-fills the horizontal space to the right of the LCD (`flex: 1; min-width: 0; overflow: hidden`). Four stacked rows.

### 6.1 Scrolling Title

| Property | Value |
|---|---|
| Height | `28px` |
| Background | `#0a120a` |
| Border | `1px solid #1a3a1a`, `border-radius: 3px` |
| Overflow | `hidden` |
| Font | `Courier New monospace, 12px` |
| Color | `#39ff14`, `text-shadow: 0 0 6px #39ff1466` |
| Scroll | `translateX(0 → -50%)`, `12s linear infinite` |
| Loop trick | Text duplicated in a single `white-space: nowrap` container with `padding-left: 100%` — seamless, no visible jump |

### 6.2 Info Row

| Element | Value |
|---|---|
| Bitrate / kHz badges | `background: #0a1a0a; border: 1px solid #1a4a1a; border-radius: 2px; padding: 1px 4px` |
| Badge text | `Courier New 11px, #39ff14` |
| `kbps` / `kHz` labels | `Courier New 10px, #667766` |
| `mono` (inactive) | `#2a4a2a` |
| `stereo` (active) | `#39ff14, text-shadow: 0 0 4px #39ff1455` |

---

## 7. Sliders

### 7.1 Track & Thumb Construction (all sliders)

The thumb must never visually exit the player window. Implementation:

```
[slunit wrapper]
  ├── label ("VOL" / "BAL")
  └── [trk-outer]  ← overflow: hidden; height: 14px
        └── [trk]  ← position: absolute; left: 7px; right: 7px  (half thumb-width inset each side)
              ├── [fill]   ← width: {val}%
              └── [thumb]  ← position: absolute; left: {val}%; transform: translate(-50%, -50%)
```

The `7px` inset on each side equals half the thumb width, so the thumb center travels the full track but the overhanging half is clipped by `trk-outer`.

| Property | Value |
|---|---|
| Track height | `6px` |
| Track background | `#1a2a1a` |
| Track border | `1px solid #2a4a2a`, `border-radius: 3px` |
| Thumb size | `14×14px` |
| Thumb background | `linear-gradient(to bottom, #c0c0d0, #787890 50%, #505060)` |
| Thumb border | `1px solid #999`, `border-radius: 2px` |
| Thumb shadow | `0 1px 3px rgba(0,0,0,0.6)` |
| Thumb grip | Center `2×8px` bar, `linear-gradient(#eee, #aaa)`, `border-radius: 1px` |

### 7.2 Volume Slider Fill Color

Continuous RGB lerp — no hard steps:

```js
function volColor(v) {
  if (v < 0.5) {
    const t = v / 0.5;
    // #ffcc00 → #ff8800
    return `rgb(255, ${Math.round(204 - 44 * t)}, 0)`;
  } else {
    const t = (v - 0.5) / 0.5;
    // #ff8800 → #cc2200
    return `rgb(${Math.round(255 - 51 * t)}, ${Math.round(136 - 136 * t)}, 0)`;
  }
}
```

| Range | Color |
|---|---|
| 0% – 49% | Golden yellow `#ffcc00` → warm orange |
| 50% – 79% | Orange `#ff8800` |
| 80% – 100% | Deep red `#cc2200` |

### 7.3 Balance Slider

| Property | Value |
|---|---|
| Fill color | Always `#1a6a1a` (green) |
| Default position | `50%` (center = balanced) |

### 7.4 Seek / Position Bar

Contained in a `seekrow` div with `overflow: hidden`. Track inset `left: 7px; right: 7px` same as volume/balance.

| Property | Value |
|---|---|
| Track height | `10px` |
| Track background | `#111822` |
| Track border | `1px solid #2a3a4a`, `border-radius: 2px` |
| Fill gradient | `linear-gradient(to right, #554400, #aa8800, #ffcc00)` |
| Thumb size | `22×14px` |
| Thumb background | `linear-gradient(to bottom, #c0c0c8, #888890 60%, #505058)` |
| Thumb grip text | `"|||"`, `7px`, `color: #555`, centered |

### 7.5 EQ / PL Buttons

| Property | Value |
|---|---|
| Height | `22px`, `padding: 1px 5px` |
| Background | `linear-gradient(to bottom, #555577, #333355)` |
| Border | `1px solid #7777aa`, `border-radius: 2px` |
| Text | `Courier New 9px, #aaaaff` |
| LED prefix | `▶` `7px`, `color: #39ff14` |
| Hover | Background `#7777aa → #555588` |

---

## 8. Playback Controls Bar

| Property | Value |
|---|---|
| Height | `44px` |
| Background | `linear-gradient(to bottom, #4a4a6a, #505070 30%, #484868 70%, #3c3c5c)` |
| Border | `1px solid #6a6a9a` (sides), top `#3a3a5a`, bottom `#2a2a4a` |
| Padding | `5px 8px` |
| Gap | `5px` |
| Align | `align-items: center` |

### 8.1 Transport Button Shared Properties

| Property | Value |
|---|---|
| Height | `28px` |
| Border radius | `3px` |
| Border | `1px solid #8888bb` |
| Background (normal) | `linear-gradient(to bottom, #888899, #6a6a7a 30%, #585868 70%, #484858)` |
| Inner highlight | `inset 0 1px 0 rgba(255,255,255,0.15)` |
| Drop shadow | `0 2px 3px rgba(0,0,0,0.4)` |
| Hover background | `linear-gradient(to bottom, #aaaacc, #8888aa 30%, #7070aa 70%, #5c5c8a)` |
| Hover border | `#aaaadd` |
| Active background | Gradient reversed: `#484858 → #6a6a7a` |
| Active shadow | `inset 0 2px 3px rgba(0,0,0,0.5)` |
| Active transform | `translateY(1px)` |
| Transition | `all 0.05s` |

### 8.2 Transport Button Icons (SVG, `viewBox="0 0 16 14"`)

| Button | Width | Icon |
|---|---|---|
| Previous `⏮` | `28px` | `rect 3×12` (left bar) + `polygon` right-facing triangle pointing left |
| Play `▶` | `32px` | `polygon` right-facing triangle `1,1 13,7 1,13`, fill `#39ff14` + `drop-shadow(0 0 2px #39ff1466)` |
| Pause `⏸` | `28px` | Two `rect 4×12` with `3px` gap |
| Stop `⏹` | `28px` | `rect 10×10` centered |
| Next `⏭` | `28px` | Triangle + `rect 3×12` (right bar) |
| Eject `⏏` | `28px` | `polygon` upward triangle + `rect 14×3` beneath |

Default icon fill (all except Play): `#dddde8`

### 8.3 Shuffle & Repeat Toggles

| Property | Value |
|---|---|
| Height | `22px`, `padding: 0 8px` |
| Background | `linear-gradient(to bottom, #555577, #3a3a5a)` |
| Border | `1px solid #7777aa`, `border-radius: 3px` |
| Text | `Courier New 9px bold, #bbbbdd` |
| Shadow | `inset 0 1px 0 rgba(255,255,255,0.1), 0 1px 2px rgba(0,0,0,0.4)` |
| LED size | `7×7px`, `border-radius: 1px` |
| LED active | `background: #39ff14; box-shadow: 0 0 4px #39ff14aa` |
| LED inactive | `background: #555; box-shadow: none` |

### 8.4 Winamp Logo Badge

| Property | Value |
|---|---|
| Size | `34×34px` SVG |
| Ellipses (3 concentric) | Outer `rx12 ry9 fill #774400`, mid `rx10 ry7 fill #cc8800`, inner `rx7 ry5 fill #ffbb00` |
| Antenna arcs | Two `<path>` curves above ellipses: outer `stroke #cc8800 width 2`, inner `stroke #ffcc44 width 1.5` |
| Opacity | `0.85` normal, `1.0` hover |
| Cursor | `pointer` |

---

## 9. States Summary

| Element | Playing | Idle / Stopped |
|---|---|---|
| Timer format | `MM:SS` elapsed | `-MM:SS` remaining |
| State square | Hidden | Visible (`#cc3300`), left of triangle |
| Play triangle | Visible, `#39ff14` | Visible, `#39ff14` |
| Sidebar labels | `C O R T O C` | `O A I T D V` |
| Spectrum bars | Visible, animated | Hidden |
| Left rule | Visible | Visible |
| Bottom rule | Visible | Visible |
| Title scroll | Animating | Static |

---

## 10. Color Reference

| Token | Hex | Usage |
|---|---|---|
| Panel background | `#0d1a0d` | LCD display fill |
| Dot matrix dot | `#152515` | LCD texture circles |
| Panel border | `#2a4a2a` | LCD outer edge |
| Neon green | `#39ff14` | Active segments, labels, play triangle |
| Segment inactive | `#1a2e1a` | Dim 7-seg bars |
| State square / bar zone 5 | `#cc3300` | Stop indicator + spectrum peak |
| Bar zone 1 — low green | `#00cc00` | Spectrum bottom |
| Bar zone 2 — yellow-green | `#88cc00` | Spectrum lower-mid |
| Bar zone 3 — amber | `#ffaa00` | Spectrum mid |
| Bar zone 4 — orange | `#dd6600` | Spectrum upper-mid |
| Peak marker | `#ffffff` | Peak hold dot |
| Rule dot A | `#00aaaa` | Boundary rule teal |
| Rule dot B | `#008888` | Boundary rule dark teal |
| Volume fill low | `#ffcc00` | 0–49% |
| Volume fill mid | `#ff8800` | ~65% |
| Volume fill high | `#cc2200` | 80–100% |
| Balance fill | `#1a6a1a` | Always green |
| Seek fill start | `#554400` | Amber-gold left |
| Seek fill end | `#ffcc00` | Amber-gold right |
| Body gradient top/bottom | `#3a3a5a` | Player body dark ends |
| Body gradient center | `#505075` | Player body midpoint |
| Button border | `#8888bb` | Transport button edge |
| Button bg top | `#888899` | Transport button highlight |
| Button bg bottom | `#484858` | Transport button shadow |
| Title bar top | `#5a5a7a` | Window title bar |
| Title bar bottom | `#1e1e3a` | Window title bar |
| Title bar text | `#c8c8d8` | "WINAMP" label |
| EQ/PL bg | `#555577` | EQ/PL button fill |
| EQ/PL border | `#7777aa` | EQ/PL + toggle border |
| Info badge bg | `#0a1a0a` | kbps/kHz badge fill |
| Info badge border | `#1a4a1a` | kbps/kHz badge edge |
| Close button | `#cc6655` | Window close |
| Logo outer | `#774400` | Winamp badge dark amber |
| Logo mid | `#cc8800` | Winamp badge amber |
| Logo inner | `#ffbb00` | Winamp badge gold |

---

## 11. Interaction Specification

### 11.1 Slider Behavior (Volume, Balance, Seek)

- Click anywhere on track → repositions thumb and fill instantly
- `mousedown` on thumb → begins drag
- `mousemove` (during drag) → updates position, clamped `[0, 1]`
- `mouseup` (anywhere on document) → ends drag
- Fill width always equals `val * 100%`
- Thumb `left` always equals `val * 100%`

### 11.2 Volume Color Transition

```js
// Continuous RGB lerp, no hard-coded step boundaries
function volColor(v) {
  if (v < 0.5) {
    const t = v / 0.5;           // 0→1 across low half
    return `rgb(255, ${Math.round(204 - 44 * t)}, 0)`;
  } else {
    const t = (v - 0.5) / 0.5;  // 0→1 across high half
    return `rgb(${Math.round(255 - 51 * t)}, ${Math.round(136 - 136 * t)}, 0)`;
  }
}
```

### 11.3 Spectrum Animator

```js
const N_BARS = 32;
const DECAY_SPEED  = 0.07;  // fall interpolation rate per frame
const RISE_SPEED   = 0.18;  // rise interpolation rate per frame
const PEAK_HOLD    = 60;    // frames to hold peak marker
const PEAK_DECAY   = 1.2;   // px/frame after hold expires
const TARGET_INTERVAL = [180, 300]; // ms range for new target gen

// Frequency curve: peaks at bass (~15%), low-mid (~55%), high (~85%)
function genTargets() {
  return Array.from({ length: N_BARS }, (_, i) => {
    const x = i / N_BARS;
    const v = 0.30 + 0.45 * Math.exp(-Math.pow(x - 0.15, 2) / 0.04)
                   + 0.25 * Math.exp(-Math.pow(x - 0.55, 2) / 0.06)
                   + 0.15 * Math.exp(-Math.pow(x - 0.85, 2) / 0.03);
    return Math.min(1, v * (0.7 + 0.6 * Math.random()));
  });
}
```

Animation loop runs at `requestAnimationFrame` (60 fps target).

### 11.4 Title Scroll

```css
.scroll-inner {
  display: flex;
  white-space: nowrap;
  padding-left: 100%;            /* starts off-screen right */
  animation: scroll 12s linear infinite;
}
@keyframes scroll {
  0%   { transform: translateX(0); }
  100% { transform: translateX(-50%); }  /* -50% because text is doubled */
}
```

Text content is duplicated inside `.scroll-inner` so the 0%→100% cycle is seamless with no blank gap.

---

## 12. Critical Layout Constraint Rules

1. **LCD panel** has `overflow: hidden`. Nothing renders outside its `190×120px` boundary.

2. **Spectrum zone** (`top: 52px`) and **timer zone** (`top: 5px → height 44px`) must not overlap. Spectrum starts at `top: 52px`.

3. **Bars container** is inset `5px` from both the left rule and the bottom rule. Bars never touch a rule line.

4. **All sliders** use the inset-track technique: `trk-outer` has `overflow: hidden`; the inner `.trk` is inset `left: 7px; right: 7px` (half thumb-width per side). The thumb's overhanging half is clipped — never extends beyond the metadata panel.

5. **Seek bar** row uses the same containment pattern. Its `seekrow` div has `overflow: hidden`.

6. **Total width is fixed at `540px`**. No child may trigger horizontal scroll.

7. **Controls bar** uses `align-items: center`. Transport buttons (`28px`) and toggles (`22px`) are vertically centered within the `44px` bar height.

8. **Metadata panel** (`flex: 1; min-width: 0`) must not push the LCD or overflow the 540px shell. All children need `overflow: hidden` or `flex-shrink: 0` as appropriate.