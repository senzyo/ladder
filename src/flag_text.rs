use anyhow::Result;
use gpui::{
    App, Bounds, Font, FontFeatures, GlyphId, Hsla, LineLayout, Pixels, Point, TextRun, TextStyle,
    Window, WrapBoundary, font, point, px, rgba,
};
use std::{borrow::Cow, collections::HashMap, sync::LazyLock};
use ttf_parser::{
    Face, colr,
    gsub::{LigatureSet, SubstitutionSubtable},
};
use unicode_segmentation::UnicodeSegmentation;

const FONT_DATA: &[u8] = include_bytes!("../assets/ladder-flags.ttf");
static FACE: LazyLock<Face<'static>> =
    LazyLock::new(|| Face::parse(FONT_DATA, 0).expect("invalid bundled flag font"));
static FLAG_FONT: LazyLock<Font> = LazyLock::new(|| Font {
    features: FontFeatures(vec![("ccmp".into(), 1)].into()),
    ..font("Ladder Flags")
});

pub(crate) fn register(cx: &mut App) -> Result<()> {
    cx.text_system().add_fonts(vec![Cow::Borrowed(FONT_DATA)])
}

// 子集用单个 ccmp 连字表记录所有支持的国旗及地区旗。
fn flag_candidates(first: char) -> Option<LigatureSet<'static>> {
    let table = FACE.tables().gsub?.lookups.get(0)?.subtables.get(0)?;
    let SubstitutionSubtable::Ligature(table) = table else {
        return None;
    };
    let index = table.coverage.get(FACE.glyph_index(first)?)?;
    table.ligature_sets.get(index)
}

fn flag_glyph(cluster: &str) -> Option<ttf_parser::GlyphId> {
    let mut chars = cluster.strip_suffix('\u{fe0f}').unwrap_or(cluster).chars();
    let Some(first @ ('🇦'..='🇿' | '🏴')) = chars.next() else {
        return None;
    };
    flag_candidates(first)?
        .into_iter()
        .find(|ligature| {
            chars
                .clone()
                .map(|ch| FACE.glyph_index(ch))
                .eq(ligature.components.into_iter().map(Some))
        })
        .map(|ligature| ligature.glyph)
}

fn flag_spans(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.grapheme_indices(true)
        .filter(|(_, cluster)| flag_glyph(cluster).is_some())
}

pub(crate) fn has_flags(text: &str) -> bool {
    text.as_bytes().contains(&0xf0) && flag_spans(text).next().is_some()
}

pub(crate) fn runs(text: &str, style: &TextStyle) -> Vec<TextRun> {
    if !text.as_bytes().contains(&0xf0) {
        return vec![style.to_run(text.len())];
    }
    let mut runs = Vec::new();
    let mut end = 0;
    for (start, cluster) in flag_spans(text) {
        if end < start {
            runs.push(style.to_run(start - end));
        }
        let mut run = style.to_run(cluster.len());
        run.font = FLAG_FONT.clone();
        // GPUI 0.2.2 的 shape_line 会合并相邻同色同装饰的段，即使字体不同。
        // 国旗自带调色板，透明前景将它与普通文字的字体段隔开。
        run.color = gpui::transparent_black();
        runs.push(run);
        end = start + cluster.len();
    }
    if end < text.len() || runs.is_empty() {
        runs.push(style.to_run(text.len() - end));
    }
    runs
}

// GPUI 0.2.2 的 GlyphId 是 #[repr(C)] 的单字段 u32。这里只转换同一字体的
// 有效 OpenType 字形编号，交给 GPUI 的字形图集时仍使用原来的字体。
fn gpui_glyph(id: ttf_parser::GlyphId) -> GlyphId {
    unsafe { std::mem::transmute::<u32, GlyphId>(u32::from(id.0)) }
}

#[derive(Default)]
struct Layers(Vec<(GlyphId, Option<Hsla>)>);

impl colr::Painter for Layers {
    fn outline(&mut self, id: ttf_parser::GlyphId) {
        self.0.push((gpui_glyph(id), None));
    }

    fn paint_foreground(&mut self) {}

    fn paint_color(&mut self, color: ttf_parser::RgbaColor) {
        let value = u32::from_be_bytes([color.red, color.green, color.blue, color.alpha]);
        self.0.last_mut().unwrap().1 = Some(rgba(value).into());
    }
}

static COLOR_LAYERS: LazyLock<HashMap<GlyphId, Layers>> = LazyLock::new(|| {
    (0..FACE.number_of_glyphs())
        .filter_map(|id| {
            let id = ttf_parser::GlyphId(id);
            let mut layers = Layers::default();
            FACE.paint_color_glyph(id, 0, &mut layers)?;
            Some((gpui_glyph(id), layers))
        })
        .collect()
});

pub(crate) fn paint_line(
    layout: &LineLayout,
    wraps: &[WrapBoundary],
    origin: Point<Pixels>,
    line_height: Pixels,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let text = cx.text_system();
    let flags = text.resolve_font(&FLAG_FONT);
    let baseline = (line_height - layout.ascent - layout.descent) / 2. + layout.ascent;
    let clip = window.content_mask().bounds;
    let mut row_y = origin.y;
    let mut row_start = px(0.);
    let mut wraps = wraps.iter().peekable();
    for (run_ix, run) in layout.runs.iter().enumerate() {
        let glyph_size = text.bounding_box(run.font_id, layout.font_size).size;
        for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
            if wraps
                .next_if(|boundary| **boundary == (WrapBoundary { run_ix, glyph_ix }))
                .is_some()
            {
                row_y += line_height;
                row_start = glyph.position.x;
            }
            let top = point(origin.x + glyph.position.x - row_start, row_y);
            let bounds = Bounds::new(top, glyph_size);
            if !bounds.intersects(&clip) {
                continue;
            }
            let pen = point(top.x, top.y + baseline);
            if run.font_id == flags
                && let Some(layers) = COLOR_LAYERS.get(&glyph.id)
            {
                // 每层单独排序，避免 GPUI 按图集位置重排重叠的彩色轮廓。
                for &(id, ink) in &layers.0 {
                    let ink = ink.map_or(color, |ink| ink.opacity(color.a));
                    window.paint_layer(bounds, |window| {
                        window.paint_glyph(pen, run.font_id, id, layout.font_size, ink)
                    })?;
                }
            } else if glyph.is_emoji {
                window.paint_emoji(pen, run.font_id, glyph.id, layout.font_size)?;
            } else {
                window.paint_glyph(pen, run.font_id, glyph.id, layout.font_size, color)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_flags_supported_by_the_font_use_its_color_layers() {
        let england = "🏴\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}";
        for sample in ["🇨🇳", "前🇺🇸🇯🇵后", "🇪🇺", england] {
            assert!(has_flags(sample), "{sample:?}");
        }
        for sample in [
            "US",
            "🇯",
            "🇿🇿",
            "🏴",
            "🏳️‍🌈",
            england.trim_end_matches('\u{e007f}'),
        ] {
            assert!(!has_flags(sample), "{sample:?}");
        }
        let style = TextStyle::default();
        let spans = runs("中🇨🇳a🇬🇧", &style);
        assert_eq!(
            spans.iter().map(|run| run.len).collect::<Vec<_>>(),
            [3, 8, 1, 8]
        );
        assert_eq!(spans[0].font, style.font());
        assert_eq!(spans[1].font, *FLAG_FONT);
        assert_eq!(spans[2].font, style.font());
    }

    #[test]
    fn flag_glyph_paints_distinct_palette_layers() {
        let flag = gpui_glyph(flag_glyph("🇯🇵").unwrap());
        let layers = &COLOR_LAYERS[&flag].0;
        assert!(layers.windows(2).any(|pair| pair[0].1 != pair[1].1));
    }
}
