//! Native presentation assets. The clients speak only the public Application API.
pub const GTK_CLIENT: &str = include_str!("../linux/shuttli.py");
pub const TRAY_CLIENT: &str = include_str!("../linux/tray.py");
pub const TRAY_MODEL: &str = include_str!("../linux/tray_model.py");
#[cfg(target_os = "macos")]
pub const MACOS_CLIENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shuttli-ui"));
pub mod i18n;
pub const BRAND_CLIENT: &str = include_str!("../linux/brand.py");
pub const I18N_CLIENT: &str = include_str!("../linux/i18n.py");
pub const LOCALES: [(&str, &str); 4] = [
    ("en", include_str!("../locales/en.json")),
    ("nl", include_str!("../locales/nl.json")),
    ("de", include_str!("../locales/de.json")),
    ("fr", include_str!("../locales/fr.json")),
];

pub const UI_MODEL: &str = include_str!("../linux/ui_model.py");
pub const UI_STYLE: &str = include_str!("../linux/style.css");
pub const UI_MARK: &str = include_str!("../assets/mark.svg");

pub const TRAY_ICONS: &str = include_str!("../linux/tray_icons.py");
