use crate::flag_text;
use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, ShapedLine, SharedString, Style, Window, px, relative, size,
};
use std::{cell::RefCell, rc::Rc};
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct ConfigName(pub(super) SharedString);

fn wrap_name(name: &str, limit: Pixels, shape: impl Fn(&str) -> ShapedLine) -> Vec<ShapedLine> {
    // 每行取实际能放下的最长前缀，断点只落在完整字素之间。
    let ends: Vec<_> = name
        .grapheme_indices(true)
        .map(|(i, g)| i + g.len())
        .collect();
    let mut remaining = ends.as_slice();
    let mut start = 0;
    let mut lines = Vec::new();
    while !remaining.is_empty() {
        let prefix = |end: usize| shape(&name[start..end]);
        let count = remaining
            .partition_point(|&end| prefix(end).width <= limit)
            .max(1);
        let end = remaining[count - 1];
        lines.push(prefix(end));
        start = end;
        remaining = &remaining[count..];
    }
    lines
}

impl IntoElement for ConfigName {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ConfigName {
    type RequestLayoutState = (Rc<RefCell<Vec<ShapedLine>>>, Pixels);
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let text_system = window.text_system().clone();
        let has_flags = flag_text::has_flags(&self.0);
        let shape = move |text: &str| {
            let runs = has_flags.then(|| flag_text::runs(text, &text_style));
            text_system.shape_line(
                SharedString::new(text),
                font_size,
                runs.as_deref().unwrap_or(&[text_style.to_run(text.len())]),
                None,
            )
        };
        let full = shape(&self.0);
        let scale = window.scale_factor();
        let natural_width = (full.width * scale).ceil() / scale;
        let mut style = Style::default();
        style.size.width = natural_width.into();
        style.min_size.width = px(0.).into();
        style.max_size.width = relative(1.).into();
        style.flex_shrink = 0.;

        let name = self.0.clone();
        let line_height = window.line_height();
        let lines = Rc::new(RefCell::new(vec![full.clone()]));
        let measured_lines = lines.clone();
        let mut measured_width = natural_width;
        let layout = window.request_measured_layout(style, move |known, available, _, _| {
            let width = known
                .width
                .unwrap_or(match available.width {
                    AvailableSpace::Definite(width) => width,
                    _ => natural_width,
                })
                .min(natural_width);
            if width != measured_width {
                *measured_lines.borrow_mut() = if full.width <= width {
                    vec![full.clone()]
                } else {
                    wrap_name(&name, width, &shape)
                };
                measured_width = width;
            }
            size(width, line_height * measured_lines.borrow().len() as f32)
        });
        (layout, (lines, line_height))
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (lines, line_height): &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut origin = bounds.origin;
        let color = window.text_style().color;
        for line in lines.borrow().iter() {
            if flag_text::has_flags(&line.text) {
                let _ = flag_text::paint_line(line, &[], origin, *line_height, color, window, cx);
            } else {
                let _ = line.paint(origin, *line_height, window, cx);
            }
            origin.y += *line_height;
        }
    }
}
