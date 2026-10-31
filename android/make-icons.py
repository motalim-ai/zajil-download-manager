#!/usr/bin/env python3
"""Writes the app's icons from the approved bird of assets/icon (README there):
the launcher's adaptive icon (the tile's two blues behind the white bird, and
a monochrome bird for themed icons) and the notification icon. The shapes are
the SVG's own paths, scaled; nothing is redrawn. Run it again if the icon
changes; never edit its output by hand.

    python3 android/make-icons.py
"""
import pathlib
import re

root = pathlib.Path(__file__).resolve().parent.parent
paths = re.findall(r'<path d="([^"]+)"', (root / "assets/icon/zajil-mark.svg").read_text())
# The mark's viewBox: its left, top and size.
x0, y0, size = 44.85, 44.40, 478.12
res = root / "android/app/src/main/res"
header = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<!-- Generated from assets/icon/zajil-mark.svg by android/make-icons.py. Do not edit by hand. -->\n"
)


def bird(canvas, box, fill):
    """The bird, `box` dp wide, in the middle of a `canvas` dp square."""
    scale = box / size
    offset = (canvas - box) / 2
    shapes = "".join(
        f'\n        <path android:fillColor="{fill}" android:pathData="{p}" />' for p in paths
    )
    return (
        f'<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
        f'    android:width="{canvas}dp" android:height="{canvas}dp"\n'
        f'    android:viewportWidth="{canvas}" android:viewportHeight="{canvas}">\n'
        f'    <group android:scaleX="{scale:.6f}" android:scaleY="{scale:.6f}"'
        f' android:translateX="{offset - x0 * scale:.4f}" android:translateY="{offset - y0 * scale:.4f}">'
        f"{shapes}\n    </group>\n</vector>\n"
    )


# 56 of 108 dp: inside the 66 dp that every launcher mask keeps.
(res / "drawable/ic_launcher_foreground.xml").write_text(header + bird(108, 56, "#FFFFFFFF"))
(res / "drawable/ic_launcher_monochrome.xml").write_text(header + bird(108, 56, "#FF000000"))
(res / "drawable/ic_stat_zajil.xml").write_text(header + bird(24, 22, "#FFFFFFFF"))
