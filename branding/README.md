# Product display name

`name.txt` is the single manually edited source of the display name: one UTF-8 line, 1-128 bytes, no control characters or leading/trailing whitespace. The name is identical across UI languages.

After changing it:

```sh
python3 tools/render_branding.py
cargo build --release -p shuttli-host --locked
python3 tools/install.py
```

Restart resident processes to use the rebuilt name. `shuttli --print-product-name` reads the build-time name without contacting the daemon or loading a profile.

Rust uses shuttli-brand; Python resources receive the same name from the host; Swift gets a build-generated UTF-8 constant. Translators interpolate `{app_name}`. The installer queries the built artifact and encodes desktop/plist metadata safely. README uses generated name markers. Tests cover name validation, resource loading, Unicode/literal interpolation and installer metadata.

The display name is a presentation dependency; core, model, ports, application and runtime do not depend on it. Technical identifiers (`shuttli`, `SHUTTLI_*`, crate/package names, protocol namespaces, platform IDs and profile paths) are interfaces requiring coordinated changes and tests, not localized copy. Graphic wordmarks remain editable design assets.
