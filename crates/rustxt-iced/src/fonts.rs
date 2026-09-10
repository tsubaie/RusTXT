//! Keep fallback text consistent with the desktop's font selection.
use iced_graphics::text::cosmic_text::{Fallback, PlatformFallback};

struct DesktopFallback {
    common: Vec<&'static str>,
}
impl Fallback for DesktopFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }
    fn forbidden_fallback(&self) -> &[&'static str] {
        PlatformFallback.forbidden_fallback()
    }
    fn script_fallback(&self, script: unicode_script::Script, locale: &str) -> &[&'static str] {
        if script == unicode_script::Script::Arabic {
            &["Noto Naskh Arabic", "Noto Sans Arabic"]
        } else {
            PlatformFallback.script_fallback(script, locale)
        }
    }
}

pub fn configure(sans: Option<String>) {
    let mut common = Vec::new();
    if let Some(sans) = sans {
        common.push(&*Box::leak(sans.into_boxed_str()));
    }
    common.extend_from_slice(PlatformFallback.common_fallback());
    let mut system = iced_graphics::text::font_system()
        .write()
        .expect("font system");
    let raw = system.raw();
    let locale = raw.locale().to_owned();
    let db = raw.db().clone();
    *raw = iced_graphics::text::cosmic_text::FontSystem::new_with_locale_and_db_and_fallback(
        locale,
        db,
        DesktopFallback { common },
    );
}
