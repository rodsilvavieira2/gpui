//! Small layout, style, and debug helpers over [`Styled`].
//!
//! Ported from `longbridge/gpui-kit` (`crates/base/src/styled.rs`) at commit
//! `cfab7b2` (Apache-2.0). Adapted for GPUI 0.2:
//!
//! - The `Role`/`RoleOverride` accessibility integration is dropped: 0.2 has
//!   no `Role` API.
//! - `box_shadow` drops the `inset` field, which 0.2's `BoxShadow` lacks.
//! - The `font_weight!` macro is expanded into documented methods
//!   (`#![deny(missing_docs)]` covers macro-generated items).
//! - The inspector-reflection helper is dropped (0.3-only API).
//! - Docs added throughout for `#![deny(missing_docs)]`.

use gpui::{
    App, BoxShadow, Corners, DefiniteLength, Div, Edges, FocusHandle, Hsla, Pixels,
    Refineable as _, StyleRefinement, Styled, Window, div, hsla, point,
};

/// A row that centers its children on the cross axis.
///
/// See [`StyledExt::h_flex`] for the cross-axis rule, which is not symmetric
/// with [`v_flex`].
pub fn h_flex() -> Div {
    div().h_flex()
}

/// A column whose children stretch across the cross axis.
///
/// See [`StyledExt::v_flex`] for the cross-axis rule, which is not symmetric
/// with [`h_flex`].
pub fn v_flex() -> Div {
    div().v_flex()
}

/// Builds a drop shadow from offset, blur, spread, and color.
pub fn box_shadow(
    x: impl Into<Pixels>,
    y: impl Into<Pixels>,
    blur: impl Into<Pixels>,
    spread: impl Into<Pixels>,
    color: Hsla,
) -> BoxShadow {
    BoxShadow {
        offset: point(x.into(), y.into()),
        blur_radius: blur.into(),
        spread_radius: spread.into(),
        color,
    }
}

#[cfg_attr(
    any(feature = "inspector", debug_assertions),
    gpui_macros::derive_inspector_reflection
)]
/// Convenience builders over [`Styled`].
pub trait StyledExt: Styled + Sized {
    /// Merges a style refinement into this element's style.
    fn refine_style(mut self, style: &StyleRefinement) -> Self {
        self.style().refine(style);
        self
    }

    /// Lays children out in a row, centered on the cross axis.
    ///
    /// The centering is the desktop default for a row of controls — an icon
    /// beside its label lines up without either side asking for it — but it is
    /// **not** the mirror image of [`Self::v_flex`], which leaves the cross axis
    /// stretching. A column placed in a row therefore does not take the row's
    /// height: it takes its content's height and is centered inside the row.
    /// When its content is taller than the row, it overflows equally above and
    /// below, so the column's header is pushed off the top edge and clipped.
    ///
    /// Give a full-height column `h_full()` (or the row `items_start()` /
    /// `items_stretch()`) whenever the child owns a header, a footer, or a
    /// scroll region that has to resolve against the row's height.
    ///
    /// ```
    /// use gpui::{ParentElement as _, Styled as _, div};
    /// use gpui::styled_ext::StyledExt as _;
    ///
    /// // A sidebar beside a detail pane, both spanning the full height.
    /// div().h_flex().size_full().child(div().w_64().h_full());
    /// ```
    fn h_flex(self) -> Self {
        self.flex().flex_row().items_center()
    }

    /// Lays children out in a column, stretching them across the cross axis.
    ///
    /// Unlike [`Self::h_flex`] this installs no cross-axis alignment, so a child
    /// without a width fills the column. See `h_flex` for the asymmetry.
    fn v_flex(self) -> Self {
        self.flex().flex_col()
    }

    /// Sets all four paddings from one edge bundle.
    fn paddings<L>(self, paddings: impl Into<Edges<L>>) -> Self
    where
        L: Into<DefiniteLength> + Clone + Default + std::fmt::Debug + PartialEq,
    {
        let paddings = paddings.into();
        self.pt(paddings.top.into())
            .pb(paddings.bottom.into())
            .pl(paddings.left.into())
            .pr(paddings.right.into())
    }

    /// Sets all four margins from one edge bundle.
    fn margins<L>(self, margins: impl Into<Edges<L>>) -> Self
    where
        L: Into<DefiniteLength> + Clone + Default + std::fmt::Debug + PartialEq,
    {
        let margins = margins.into();
        self.mt(margins.top.into())
            .mb(margins.bottom.into())
            .ml(margins.left.into())
            .mr(margins.right.into())
    }

    /// Outlines the element in red (debug builds only).
    fn debug_red(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(0., 72.2, 50.6))
        } else {
            self
        }
    }

    /// Outlines the element in blue (debug builds only).
    fn debug_blue(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(217.2, 91.2, 59.8))
        } else {
            self
        }
    }

    /// Outlines the element in yellow (debug builds only).
    fn debug_yellow(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(47.9, 95.8, 53.1))
        } else {
            self
        }
    }

    /// Outlines the element in green (debug builds only).
    fn debug_green(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(142.1, 70.6, 45.3))
        } else {
            self
        }
    }

    /// Outlines the element in pink (debug builds only).
    fn debug_pink(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(330.4, 81.2, 60.4))
        } else {
            self
        }
    }

    /// Outlines the element in blue when `focus_handle` is focused (debug
    /// builds only).
    fn debug_focused(self, focus_handle: &FocusHandle, window: &Window, cx: &App) -> Self {
        if cfg!(debug_assertions) && focus_handle.contains_focused(window, cx) {
            self.debug_blue()
        } else {
            self
        }
    }

    /// Thin (100) font weight.
    fn font_thin(self) -> Self {
        self.font_weight(gpui::FontWeight::THIN)
    }

    /// Extra-light (200) font weight.
    fn font_extralight(self) -> Self {
        self.font_weight(gpui::FontWeight::EXTRA_LIGHT)
    }

    /// Light (300) font weight.
    fn font_light(self) -> Self {
        self.font_weight(gpui::FontWeight::LIGHT)
    }

    /// Normal (400) font weight.
    fn font_normal(self) -> Self {
        self.font_weight(gpui::FontWeight::NORMAL)
    }

    /// Medium (500) font weight.
    fn font_medium(self) -> Self {
        self.font_weight(gpui::FontWeight::MEDIUM)
    }

    /// Semi-bold (600) font weight.
    fn font_semibold(self) -> Self {
        self.font_weight(gpui::FontWeight::SEMIBOLD)
    }

    /// Bold (700) font weight.
    fn font_bold(self) -> Self {
        self.font_weight(gpui::FontWeight::BOLD)
    }

    /// Extra-bold (800) font weight.
    fn font_extrabold(self) -> Self {
        self.font_weight(gpui::FontWeight::EXTRA_BOLD)
    }

    /// Black (900) font weight.
    fn font_black(self) -> Self {
        self.font_weight(gpui::FontWeight::BLACK)
    }

    /// Sets every corner radius from one corner bundle.
    fn corner_radii(self, radius: Corners<Pixels>) -> Self {
        self.rounded_tl(radius.top_left)
            .rounded_tr(radius.top_right)
            .rounded_bl(radius.bottom_left)
            .rounded_br(radius.bottom_right)
    }
}

impl<E: Styled> StyledExt for E {}

fn hsl(hue: f32, saturation: f32, lightness: f32) -> Hsla {
    hsla(hue / 360., saturation / 100., lightness / 100., 1.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, TestAppContext,
        px,
    };

    fn column(selector: &'static str, height: f32) -> Div {
        div()
            .w(px(20.))
            .h(px(height))
            .flex_shrink_0()
            .debug_selector(move || selector.to_string())
    }

    struct CrossAxisTest;

    impl Render for CrossAxisTest {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(200.))
                .h(px(100.))
                .child(
                    h_flex()
                        .size_full()
                        .child(column("row-child", 40.))
                        .child(column("row-overflowing-child", 140.))
                        .child(
                            div()
                                .w(px(20.))
                                .debug_selector(|| "row-stretch".to_string()),
                        ),
                )
                .child(
                    v_flex().size_full().child(
                        div()
                            .h(px(20.))
                            .debug_selector(|| "col-stretch".to_string()),
                    ),
                )
        }
    }

    /// `h_flex` centers on the cross axis while `v_flex` leaves the default
    /// stretch. The asymmetry is deliberate but easy to trip over, so lock it.
    #[gpui::test]
    fn h_flex_centers_and_v_flex_stretches_on_the_cross_axis(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| CrossAxisTest);
        cx.run_until_parked();
        cx.update(|window, cx| {
            _ = window.draw(cx);
        });

        // A shorter child of a 100px row sits at (100 - 40) / 2, not at the top,
        // and a height-less child keeps its content height instead of filling.
        let child = cx.debug_bounds("row-child").unwrap();
        assert_eq!(child.top(), px(30.));
        assert_eq!(cx.debug_bounds("row-stretch").unwrap().size.height, px(0.));

        // And a child taller than the row is centered too, so its top — a
        // column's header, in a real layout — is pushed off the top edge.
        let overflowing = cx.debug_bounds("row-overflowing-child").unwrap();
        assert_eq!(overflowing.top(), px(-20.));

        // A width-less child of a column does fill the cross axis.
        assert_eq!(cx.debug_bounds("col-stretch").unwrap().size.width, px(200.));
    }
}
