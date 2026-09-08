# Logo assets

`logo.svg` is the editable vector source for the app icon. `logo-mark.svg` is the
transparent, single-color symbol for the menu bar and other compact placements.
Both use paths and strokes only, with no fonts, linked images, or filters.

The symbol redraws the supplied diagonal mouse reference as smooth vector paths,
including its separate mouse buttons, scroll wheel, and side contour. The keyboard
section preserves the reference's long key above four small keys, including their
original perspective and staggered arrangement.
Both SVGs have transparent backgrounds. The primary logo is white; the menu bar
symbol is charcoal `#181A1D`.

The app embeds SVG directly and renders it at the size required by native icon
APIs. Linux installs the SVG itself. Mac packaging renders the current SVG before
creating its required ICNS file. Checked-in PNG exports are previews only.
The Mac menu bar renders the transparent SVG as a template image, allowing macOS
to choose the appropriate color for its background.

Regenerate PNG exports on macOS:

```sh
swift scripts/render-logo.swift assets/logo.svg assets/logo.png 1024
swift scripts/render-logo.swift assets/logo-mark.svg assets/logo-mark.png 128
```

Regenerate all preview exports and the Windows installer/shortcut icon:

```sh
python3 scripts/export-icons.py
```

The ICO includes 16, 24, 32, 48, 64, 128, and 256 px images rendered individually
from SVG. Runtime window/settings images use 256 px; the Mac menu bar uses a
72 px template rendered into its native 18-point slot for Retina sharpness.
