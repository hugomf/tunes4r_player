# Winamp Classic — Spectrum Display Specification

## Overview

The Winamp classic player display is a compact LCD-style panel divided into two zones: a **timer zone** (top) and a **spectrum analyzer zone** (bottom). The entire panel sits on a dark dot-matrix background that simulates a physical phosphor LCD screen.

---

## Panel

| Property | Value |
|---|---|
| Background color | `#0d1a0d` |
| Border | `1px solid #2a4a2a` |
| Border radius | `6px` |
| Dot matrix texture | 4×4px repeating pattern; each cell has a `#152515` circle (r=0.7px) centered on `#0d1a0d` fill |

---

## Zone 1 — Left Column (Labels + Controls)

A narrow vertical strip on the left side of the panel.

### Sidebar letter labels

Stacked single characters in monospace green font, evenly spaced vertically. The letters change depending on the active mode.

| Property | Value |
|---|---|
| Font | monospace |
| Font size | 11px |
| Color | `#39ff14` (neon green) |
| Playing labels | `C O R T O C` |
| Idle labels | `O A I T D V` |

### Playback controls

Two elements side by side, positioned just above the labels. **Order: state square first (left), then play triangle (right).**

| Element | Shape | Size | Color | Visible when |
|---|---|---|---|---|
| State square | Filled rect, rx=1 | 9×9px | `#cc3300` dark red | Stopped / idle only |
| Play triangle | Filled triangle pointing right | ~20px wide × 22px tall | `#39ff14` neon green | Always |

> The play triangle height is slightly less than the timer digit height. The state square sits directly to the left of the triangle.

---

## Zone 2 — Timer (Top Right)

A 7-segment LCD display occupying the top-right area of the panel.

### Format

- **Playing**: `MM:SS` — e.g. `02:02` (elapsed time)
- **Idle / stopped**: `-MM:SS` — e.g. `-05:25` (remaining time, prefixed with a minus segment)

### 7-Segment Digit Construction

Each digit is built from 7 rectangular bar segments:

| Segment | Orientation | Size |
|---|---|---|
| Top | Horizontal | 22px wide × 5px tall |
| Top-left | Vertical | 5px wide × 18px tall |
| Top-right | Vertical | 5px wide × 18px tall |
| Middle | Horizontal | 22px wide × 5px tall |
| Bottom-left | Vertical | 5px wide × 18px tall |
| Bottom-right | Vertical | 5px wide × 18px tall |
| Bottom | Horizontal | 22px wide × 5px tall |

| Property | Value |
|---|---|
| Border radius | 2px |
| Active segment color | `#39ff14` neon green |
| Inactive segment color | `#1a2e1a` very dark green |

### Colon Separator

| Property | Value |
|---|---|
| Shape | Two filled circles stacked vertically |
| Radius | 4px |
| Color | `#39ff14` |

### Minus Sign (idle/remaining mode)

| Property | Value |
|---|---|
| Size | 12px wide × 5px tall |
| Border radius | 2px |
| Color | `#39ff14` |
| Position | Left of first digit, vertically centered at middle-segment height |

---

## Zone 3 — Spectrum Analyzer (Bottom area)

Occupies the lower portion of the panel. Empty when idle; populated with bars when playing.

### Boundary Rules

Two dotted teal lines mark the edges of the spectrum area:

| Rule | Orientation | Position |
|---|---|---|
| Left rule | Vertical | Left edge of spectrum area, full height of spectrum zone |
| Bottom rule | Horizontal | Bottom edge of spectrum area, full width of spectrum zone |

**Dot properties:**

| Property | Value |
|---|---|
| Dot size | 3×3px, rx=0.5 |
| Dot pitch | 7px (3px dot + 4px gap) |
| Color A | `#00aaaa` teal |
| Color B | `#008888` darker teal |
| Pattern | Alternating A / B |

### Spectrum Bars (playing state only)

32–38 vertical frequency bars spanning bass (left) to treble (right).

| Property | Value |
|---|---|
| Bar width | 9px |
| Gap between bars | 3px |
| Bar border radius | 1px |

### Bar Color Zones

Each bar is divided into **5 color zones** stacked from bottom to top, painted only up to the current amplitude level:

| Zone | Level | Color | Hex |
|---|---|---|---|
| 1 — Low green | Bottom (loudest portion) | Bright green | `#00cc00` |
| 2 — Yellow-green | Lower-mid | Yellow-green | `#88cc00` |
| 3 — Amber | Mid | Amber | `#ffaa00` |
| 4 — Orange | Upper-mid | Dark orange | `#dd6600` |
| 5 — Red | Peak / top | Red | `#cc3300` |

> Zones 5 (red) only appears on bars with very high amplitude. Most bars at normal levels show zones 1–3 or 1–4 only.

### Peak Hold Markers

| Property | Value |
|---|---|
| Size | 9px wide × 3px tall |
| Color | `#ffffff` white |
| Position | ~2px gap above the top of the current bar |
| Behavior | Holds at highest recent position, then slowly falls |

---

## States Summary

| Element | Playing | Idle / Stopped |
|---|---|---|
| Timer format | `MM:SS` elapsed | `-MM:SS` remaining |
| State square | Hidden | Visible (left of triangle) |
| Play triangle | Visible | Visible |
| Spectrum bars | Visible (32–38 bars) | Hidden |
| Left rule | Visible | Visible |
| Bottom rule | Visible | Visible |
| Sidebar labels | `C O R T O C` | `O A I T D V` |

---

## Color Reference

| Token | Hex | Usage |
|---|---|---|
| Panel background | `#0d1a0d` | Main bg |
| Dot matrix dot | `#152515` | Texture overlay |
| Panel border | `#2a4a2a` | Outer edge |
| Segment active | `#39ff14` | Timer digits, labels, triangle |
| Segment inactive | `#1a2e1a` | Dim 7-seg bars |
| State square | `#cc3300` | Stop indicator |
| Bar zone 1 — low green | `#00cc00` | Spectrum bottom |
| Bar zone 2 — yellow-green | `#88cc00` | Spectrum lower-mid |
| Bar zone 3 — amber | `#ffaa00` | Spectrum mid |
| Bar zone 4 — orange | `#dd6600` | Spectrum upper-mid |
| Bar zone 5 — red | `#cc3300` | Spectrum peak |
| Peak marker | `#ffffff` | Floating peak dot |
| Rule dot A | `#00aaaa` | Boundary rule primary |
| Rule dot B | `#008888` | Boundary rule alternate |