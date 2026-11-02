# Quayflow brand assets

The files in this folder are the assets the project uses. They are raster exports (there are no vector masters), so use each file at or below its native size.

| File | Use |
|---|---|
| `quayflow-logo-light.png` | Horizontal logo for light backgrounds (README, docs) |
| `quayflow-logo-dark.png` | Horizontal logo for dark backgrounds |
| `quayflow-mark.png` | Standalone harbor mark for light backgrounds |
| `quayflow-mark-reversed.png` | Standalone harbor mark for dark backgrounds |
| `quayflow.ico` | Windows application icon (multi-size); embedded in `quayflow.exe` and `quayflow-bg.exe` by `crates/quayflow/build.rs` |
| `quayflow-app-icon-512.png`, `quayflow-app-icon-128.png` | Application icon as PNG (avatars, listings) |
| `quayflow-social-preview.png` | 1280×640 repository social preview (set in the repository settings) |
| `palette.css` | Brand colors as CSS custom properties |

## Palette

| Name | Hex |
|---|---|
| Charcoal | `#0F1720` |
| Mint | `#34D399` |
| Seafoam | `#A7F3D0` |
| Off-white | `#F8FAF9` |

## Usage

- Use the light-background logo on light surfaces and the dark-background logo on dark surfaces. In Markdown, a `<picture>` element with a `prefers-color-scheme: dark` source switches between them (see the top of the repository README).
- Do not recolor, stretch, crop or add effects to the logo or icon.
- The harbor logo and the lighthouse application icon are the two marks in use. Do not combine them into a new lockup.
