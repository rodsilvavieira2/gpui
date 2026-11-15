use crate::{AssetSource, DevicePixels, IsZero, Result, SharedString, Size};
use resvg::tiny_skia::Pixmap;
use std::{
    hash::Hash,
    sync::{Arc, LazyLock},
};

/// When rendering SVGs, we render them at twice the size to get a higher-quality result.
pub const SMOOTH_SVG_SCALE_FACTOR: f32 = 2.;

#[derive(Clone, PartialEq, Hash, Eq)]
pub(crate) struct RenderSvgParams {
    pub(crate) path: SharedString,
    pub(crate) size: Size<DevicePixels>,
}

#[derive(Clone)]
pub struct SvgRenderer {
    asset_source: Arc<dyn AssetSource>,
    usvg_options: Arc<usvg::Options<'static>>,
}

pub enum SvgSize {
    Size(Size<DevicePixels>),
    ScaleFactor(f32),
}

impl SvgRenderer {
    pub fn new(asset_source: Arc<dyn AssetSource>) -> Self {
        static FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();

            // Sanitize serif: on some Linux systems fontdb maps Serif to StandardSymbolsPS
            // which encodes Greek math symbols on ASCII code points, rendering English text in Greek.
            let serif = db.family_name(&usvg::fontdb::Family::Serif);
            if serif.to_ascii_lowercase().contains("symbol")
                || serif.to_ascii_lowercase().contains("standardsymbols")
            {
                for candidate in ["Noto Serif", "DejaVu Serif", "Liberation Serif", "FreeSerif", "Times New Roman"] {
                    if db.faces().any(|f| f.families.iter().any(|(name, _)| name == candidate)) {
                        db.set_serif_family(candidate);
                        break;
                    }
                }
            }

            // Ensure sans-serif is bound to a high-quality proportional font if present:
            for candidate in ["Inter", "Noto Sans", "DejaVu Sans", "Liberation Sans", "Arial"] {
                if db.faces().any(|f| f.families.iter().any(|(name, _)| name == candidate)) {
                    db.set_sans_serif_family(candidate);
                    break;
                }
            }

            Arc::new(db)
        });

        let font_resolver = Box::new(
            move |font: &usvg::Font, db: &mut Arc<usvg::fontdb::Database>| {
                if db.is_empty() {
                    *db = FONT_DB.clone();
                }

                let mut name_list = Vec::new();
                for family in font.families() {
                    match family {
                        usvg::FontFamily::Serif => name_list.push(usvg::fontdb::Family::Serif),
                        usvg::FontFamily::SansSerif => name_list.push(usvg::fontdb::Family::SansSerif),
                        usvg::FontFamily::Cursive => name_list.push(usvg::fontdb::Family::Cursive),
                        usvg::FontFamily::Fantasy => name_list.push(usvg::fontdb::Family::Fantasy),
                        usvg::FontFamily::Monospace => name_list.push(usvg::fontdb::Family::Monospace),
                        usvg::FontFamily::Named(s) => match s.to_ascii_lowercase().as_str() {
                            "sans-serif" | "sans serif" | "sans" => {
                                name_list.push(usvg::fontdb::Family::SansSerif)
                            }
                            "serif" => name_list.push(usvg::fontdb::Family::Serif),
                            "monospace" | "mono" => {
                                name_list.push(usvg::fontdb::Family::Monospace)
                            }
                            "cursive" => name_list.push(usvg::fontdb::Family::Cursive),
                            "fantasy" => name_list.push(usvg::fontdb::Family::Fantasy),
                            _ => name_list.push(usvg::fontdb::Family::Name(s.as_str())),
                        },
                    }
                }
                // SVG charts and diagrams are predominantly sans-serif; default to SansSerif fallback:
                name_list.push(usvg::fontdb::Family::SansSerif);

                let stretch = match font.stretch() {
                    usvg::FontStretch::UltraCondensed => usvg::fontdb::Stretch::UltraCondensed,
                    usvg::FontStretch::ExtraCondensed => usvg::fontdb::Stretch::ExtraCondensed,
                    usvg::FontStretch::Condensed => usvg::fontdb::Stretch::Condensed,
                    usvg::FontStretch::SemiCondensed => usvg::fontdb::Stretch::SemiCondensed,
                    usvg::FontStretch::Normal => usvg::fontdb::Stretch::Normal,
                    usvg::FontStretch::SemiExpanded => usvg::fontdb::Stretch::SemiExpanded,
                    usvg::FontStretch::Expanded => usvg::fontdb::Stretch::Expanded,
                    usvg::FontStretch::ExtraExpanded => usvg::fontdb::Stretch::ExtraExpanded,
                    usvg::FontStretch::UltraExpanded => usvg::fontdb::Stretch::UltraExpanded,
                };

                let style = match font.style() {
                    usvg::FontStyle::Normal => usvg::fontdb::Style::Normal,
                    usvg::FontStyle::Italic => usvg::fontdb::Style::Italic,
                    usvg::FontStyle::Oblique => usvg::fontdb::Style::Oblique,
                };

                let query = usvg::fontdb::Query {
                    families: &name_list,
                    weight: usvg::fontdb::Weight(font.weight()),
                    stretch,
                    style,
                };

                db.query(&query)
            },
        );

        let default_fallback = usvg::FontResolver::default_fallback_selector();
        let fallback_resolver = Box::new(
            move |c: char, exclude_fonts: &[usvg::fontdb::ID], db: &mut Arc<usvg::fontdb::Database>| {
                if let Some(id) = default_fallback(c, exclude_fonts, db) {
                    if let Some(face) = db.face(id) {
                        let is_symbol = face.families.iter().any(|(name, _)| {
                            let lower = name.to_ascii_lowercase();
                            lower.contains("symbol") || lower.contains("standardsymbols")
                        });
                        if is_symbol {
                            let mut new_excludes = exclude_fonts.to_vec();
                            new_excludes.push(id);
                            return default_fallback(c, &new_excludes, db);
                        }
                    }
                    Some(id)
                } else {
                    None
                }
            },
        );

        let options = usvg::Options {
            font_family: "sans-serif".to_string(),
            font_resolver: usvg::FontResolver {
                select_font: font_resolver,
                select_fallback: fallback_resolver,
            },
            ..Default::default()
        };
        Self {
            asset_source,
            usvg_options: Arc::new(options),
        }
    }

    pub(crate) fn render(
        &self,
        params: &RenderSvgParams,
    ) -> Result<Option<(Size<DevicePixels>, Vec<u8>)>> {
        anyhow::ensure!(!params.size.is_zero(), "can't render at a zero size");

        // Load the tree.
        let Some(bytes) = self.asset_source.load(&params.path)? else {
            return Ok(None);
        };

        let pixmap = self.render_pixmap(&bytes, SvgSize::Size(params.size))?;

        // Convert the pixmap's pixels into an alpha mask.
        let size = Size::new(
            DevicePixels(pixmap.width() as i32),
            DevicePixels(pixmap.height() as i32),
        );
        let alpha_mask = pixmap
            .pixels()
            .iter()
            .map(|p| p.alpha())
            .collect::<Vec<_>>();
        Ok(Some((size, alpha_mask)))
    }

    pub fn render_pixmap(&self, bytes: &[u8], size: SvgSize) -> Result<Pixmap, usvg::Error> {
        let tree = usvg::Tree::from_data(bytes, &self.usvg_options)?;
        let svg_size = tree.size();
        let scale = match size {
            SvgSize::Size(size) => size.width.0 as f32 / svg_size.width(),
            SvgSize::ScaleFactor(scale) => scale,
        };

        // Render the SVG to a pixmap with the specified width and height.
        let mut pixmap = resvg::tiny_skia::Pixmap::new(
            (svg_size.width() * scale) as u32,
            (svg_size.height() * scale) as u32,
        )
        .ok_or(usvg::Error::InvalidSize)?;

        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);

        resvg::render(&tree, transform, &mut pixmap.as_mut());

        Ok(pixmap)
    }
}
