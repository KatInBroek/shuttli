# Approved desktop artwork

Source: issue #57 design delivery v1, **Linked sheets (2a)**.
The mark and app icon are copied from the supplied SVG assets, unchanged.
The dark-panel tray PNGs are the supplied T01–T05 artwork at 16/24/32 px.

- `mark.svg`: compact brand mark; the display name beside it comes from `branding/name.txt`.
- `app-icon.svg`: launcher icon; installed under the stable `org.shuttli.Control` ID.
- `tray/`: outgoing + incoming arrows, outgoing only, incoming only, pause bars,
  dashed unavailable outline. Shape communicates state independently of color.
- `python3 tools/render_tray_icons.py` regenerates the small cached ARGB pixmaps
  embedded in the tray. Pillow is only a development dependency.

Current fallback pixmaps target dark system panels, as validated on GNOME.
Automatic adaptation to arbitrary light third-party panels remains unverified.
The old product wordmark from the design package is intentionally not embedded;
product text uses the central brand setting.
