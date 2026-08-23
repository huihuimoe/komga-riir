use std::path::Path;

use komga_application::operational::FontPort;

use crate::filesystem::fonts;

#[derive(Clone, Default)]
pub struct FontAccess;

impl FontPort for FontAccess {
    fn list_font_families(&self, path: &Path) -> anyhow::Result<Vec<String>> {
        fonts::list_font_families(path)
    }

    fn load_font_family_css(&self, path: &Path, family: &str) -> anyhow::Result<Option<String>> {
        fonts::load_font_family_css(path, family)
    }

    fn load_font_file(
        &self,
        path: &Path,
        family: &str,
        file: &str,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        fonts::load_font_file(path, family, file)
    }
}
