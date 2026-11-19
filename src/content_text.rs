use crate::flag_text;
use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, SharedString, StyledText, Window,
};

pub(super) fn content_text(text: impl Into<SharedString>) -> AnyElement {
    let text = text.into();
    if flag_text::has_flags(&text) {
        FlagText(text).into_any_element()
    } else {
        text.into_any_element()
    }
}

struct FlagText(SharedString);

impl IntoElement for FlagText {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for FlagText {
    type RequestLayoutState = StyledText;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, StyledText) {
        let runs = flag_text::runs(&self.0, &window.text_style());
        let mut text = StyledText::new(self.0.clone()).with_runs(runs);
        let (layout, ()) = text.request_layout(id, inspector, window, cx);
        (layout, text)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text: &mut StyledText,
        window: &mut Window,
        cx: &mut App,
    ) {
        text.prepaint(id, inspector, bounds, &mut (), window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text: &mut StyledText,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = text.layout();
        let line_height = layout.line_height();
        let color = window.text_style().color;
        let mut origin = bounds.origin;
        let mut index = 0;
        // StyledText 已算好软换行；换到下一个显式换行时跳过原文中的换行符。
        while let Some(line) = layout.line_layout_for_index(index) {
            let _ = flag_text::paint_line(
                &line.unwrapped_layout,
                &line.wrap_boundaries,
                origin,
                line_height,
                color,
                window,
                cx,
            );
            origin.y += line.size(line_height).height;
            index += line.len() + 1;
        }
    }
}
