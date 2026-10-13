# TalkCraft app icon

**A talking parrot**, as an engraving-style head-and-shoulders portrait in the Crafting Apps'
framing: a full-bleed colour field, the bird in profile facing left with its hooked bill open,
feather scallops and cross-hatching, three sound arcs leaving the bill (the arcs come from the
SoundCraft icon this app forked from).

## Palette

Exactly three colours:

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | Line work, hatching and contour |
| Paper | `#efe9dc` | The bird and the sound arcs |
| TalkCraft green (app colour) | `#2f9e63` | The full-bleed background field |

## Geometry

A 512 × 512 viewBox, clipped to a rounded square with `rx=112`. Windows and Linux use the full
tile. The macOS icons (`soundcraft.icns`, `soundcraft-macos-512.png`) sit on Apple's grid: an
824/1024 body with a transparent margin.

## Provenance and licence

Generated from scratch by `source/parrot.py` (hand-placed paths plus programmatic hatching and
feather scallops, helpers in `source/lib.py`); no source image was traced or copied. Edit the
script, run `python3 source/parrot.py` to rewrite `icon.svg`, then `packaging/icons.sh`. MIT OR Apache-2.0, like the code (see `ATTRIBUTION.md`).

## Files

| File | What it is |
|---|---|
| `icon.svg` | The canonical vector |
| `soundcraft-1024.png` | 1024 px render |
| `soundcraft-macos-512.png` | Runtime window/Dock icon on macOS (Apple grid margin) |
| `soundcraft.icns` | macOS bundle icon |
| `soundcraft.ico` | Windows icon (16–256 px), embedded in `soundcraft.exe` and used by the MSI |
| `hicolor/<size>x<size>/apps/ai.storyteller.soundcraft.png` | Linux/FreeBSD icon theme, 16–512 px, plus `scalable/` |

Regenerate everything with `packaging/icons.sh` (needs `resvg`, and `iconutil` on macOS for the
`.icns`).
