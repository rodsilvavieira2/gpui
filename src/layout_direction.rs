//! The application's layout direction.
//!
//! Ported from `longbridge/gpui-kit` (`crates/base/src/layout_direction.rs`)
//! at commit `cfab7b2` (Apache-2.0). Adapted for GPUI 0.2:
//!
//! - Test-scheduler detection uses 0.2's
//!   `BackgroundExecutor.dispatcher.as_test()` instead of the 0.3 scheduler
//!   chain (same approach as [`crate::reduce_motion`]).
//! - Tests call the local `init` instead of the kit's crate-wide init.
//!
//! Right-to-left support is one global setting, not a switch on each
//! component: [`LayoutDirection::global`] says whether the interface reads
//! left to right or right to left, and every component that has a
//! horizontal sense — arrow-key navigation, which side a submenu opens on,
//! logical paddings and borders — reads it from there.
//!
//! An application derives the direction from the system's preferred languages
//! (Arabic, Hebrew, Persian, Urdu, … read right to left) via
//! [`LayoutDirection::system`], or follows its own language setting by
//! overriding it with [`LayoutDirection::set_global`], which redraws every
//! window.
//!
//! Layout written with the logical helpers of [`LogicalStyledExt`]
//! (`ps`/`pe`, `ms`/`me`, `border_s`/`border_e`, `rounded_s`/`rounded_e`,
//! `inset_s`/`inset_e`, `flex_row_dir`, `justify_flex_start`/`justify_flex_end`)
//! mirrors with it; physical helpers (`pl`, `left`, `flex_row`, `justify_start`,
//! …) keep their physical meaning.

use gpui::{AbsoluteLength, App, DefiniteLength, Global, JustifyContent, Length, Styled};

/// The horizontal direction an interface is laid out and navigated in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum LayoutDirection {
    /// Left to right, the default.
    #[default]
    Ltr,
    /// Right to left, for Arabic, Hebrew, Persian, Urdu and similar scripts.
    Rtl,
}

#[derive(Clone, Copy)]
struct GlobalLayoutDirection(LayoutDirection);

impl Global for GlobalLayoutDirection {}

impl LayoutDirection {
    /// Whether this is [`LayoutDirection::Rtl`].
    pub fn is_rtl(self) -> bool {
        self == Self::Rtl
    }

    /// The direction the application is laid out in.
    pub fn global(cx: &App) -> Self {
        cx.try_global::<GlobalLayoutDirection>()
            .map(|direction| direction.0)
            .unwrap_or_default()
    }

    /// Sets the direction for the whole application and redraws its windows.
    pub fn set_global(self, cx: &mut App) {
        if cx.try_global::<GlobalLayoutDirection>().map(|d| d.0) == Some(self) {
            return;
        }
        cx.set_global(GlobalLayoutDirection(self));
        cx.refresh_windows();
    }

    /// The direction of a BCP 47 locale such as `ar-EG`, `he` or `en_US`.
    pub fn from_locale(locale: &str) -> Self {
        const RTL: [&str; 12] = [
            "ar", "ckb", "dv", "fa", "he", "iw", "ks", "ps", "sd", "ug", "ur", "yi",
        ];
        let language = locale.split(['-', '_', '.']).next().unwrap_or_default();
        if RTL.iter().any(|rtl| language.eq_ignore_ascii_case(rtl)) {
            Self::Rtl
        } else {
            Self::Ltr
        }
    }

    /// The direction of the system's most preferred language.
    pub fn system() -> Self {
        sys_locale::get_locale()
            .map(|locale| Self::from_locale(&locale))
            .unwrap_or_default()
    }

    /// Maps a logical edge to the physical one: `start` is left in LTR and
    /// right in RTL.
    pub(crate) fn start_is_left(self) -> bool {
        self == Self::Ltr
    }
}

/// Applies the system direction, unless the application already chose one or
/// GPUI's test scheduler is running. Call once at startup.
pub fn init(cx: &mut App) {
    if cx.has_global::<GlobalLayoutDirection>() {
        return;
    }
    let is_test = crate::reduce_motion::is_test_scheduler(cx);
    let direction = if is_test {
        LayoutDirection::Ltr
    } else {
        LayoutDirection::system()
    };
    cx.set_global(GlobalLayoutDirection(direction));
}

macro_rules! logical_edge {
    ($(#[$doc:meta])* $name:ident, $start:ident, $end:ident, $len:ty) => {
        $(#[$doc])*
        fn $name(self, length: impl Into<$len> + Clone, cx: &App) -> Self {
            if LayoutDirection::global(cx).start_is_left() {
                self.$start(length)
            } else {
                self.$end(length)
            }
        }
    };
}

/// Styling helpers that follow [`LayoutDirection::global`]: `s` (start) is
/// the left edge in LTR and the right edge in RTL, `e` (end) the opposite.
pub trait LogicalStyledExt: Styled + Sized {
    logical_edge!(
        /// Padding on the start edge.
        ps, pl, pr, DefiniteLength
    );
    logical_edge!(
        /// Padding on the end edge.
        pe, pr, pl, DefiniteLength
    );
    logical_edge!(
        /// Margin on the start edge.
        ms, ml, mr, Length
    );
    logical_edge!(
        /// Margin on the end edge.
        me, mr, ml, Length
    );
    logical_edge!(
        /// Inset from the start edge of the positioned ancestor.
        inset_s, left, right, Length
    );
    logical_edge!(
        /// Inset from the end edge of the positioned ancestor.
        inset_e, right, left, Length
    );

    /// Border width on the start edge.
    fn border_s(mut self, width: impl Into<AbsoluteLength>, cx: &App) -> Self {
        let width = Some(width.into());
        let widths = &mut self.style().border_widths;
        if LayoutDirection::global(cx).start_is_left() {
            widths.left = width;
        } else {
            widths.right = width;
        }
        self
    }

    /// Border width on the end edge.
    fn border_e(mut self, width: impl Into<AbsoluteLength>, cx: &App) -> Self {
        let width = Some(width.into());
        let widths = &mut self.style().border_widths;
        if LayoutDirection::global(cx).start_is_left() {
            widths.right = width;
        } else {
            widths.left = width;
        }
        self
    }

    /// Corner radius of both start corners.
    fn rounded_s(self, radius: impl Into<AbsoluteLength> + Clone, cx: &App) -> Self {
        // Port note: 0.2 has no `rounded_l`/`rounded_r` shorthands.
        if LayoutDirection::global(cx).start_is_left() {
            self.rounded_tl(radius.clone()).rounded_bl(radius)
        } else {
            self.rounded_tr(radius.clone()).rounded_br(radius)
        }
    }

    /// Corner radius of both end corners.
    fn rounded_e(self, radius: impl Into<AbsoluteLength> + Clone, cx: &App) -> Self {
        if LayoutDirection::global(cx).start_is_left() {
            self.rounded_tr(radius.clone()).rounded_br(radius)
        } else {
            self.rounded_tl(radius.clone()).rounded_bl(radius)
        }
    }

    /// A horizontal flex row whose first child sits on the start edge:
    /// `flex_row` in LTR, `flex_row_reverse` in RTL.
    fn flex_row_dir(self, cx: &App) -> Self {
        if LayoutDirection::global(cx).is_rtl() {
            self.flex().flex_row_reverse()
        } else {
            self.flex().flex_row()
        }
    }

    /// Packs a row's children against the edge its first child sits on — the
    /// start edge of a [`flex_row_dir`](Self::flex_row_dir) row.
    ///
    /// `justify_start` is the physical left edge even in a reversed row, so it
    /// does not follow the layout direction; this is CSS `flex-start`.
    fn justify_flex_start(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::FlexStart);
        self
    }

    /// Packs a row's children against the edge its last child sits on — the
    /// end edge of a [`flex_row_dir`](Self::flex_row_dir) row. This is CSS
    /// `flex-end`; `justify_end` stays on the physical right edge.
    fn justify_flex_end(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::FlexEnd);
        self
    }

    /// Aligns text to the start edge: `text_left` in LTR, `text_right` in RTL.
    fn text_start(self, cx: &App) -> Self {
        if LayoutDirection::global(cx).start_is_left() {
            self.text_left()
        } else {
            self.text_right()
        }
    }

    /// Aligns text to the end edge: `text_right` in LTR, `text_left` in RTL.
    fn text_end(self, cx: &App) -> Self {
        if LayoutDirection::global(cx).start_is_left() {
            self.text_right()
        } else {
            self.text_left()
        }
    }
}

impl<T: Styled> LogicalStyledExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, div, px};

    #[test]
    fn locale_direction() {
        for rtl in ["ar", "ar-EG", "he_IL", "fa", "ur-PK", "HE", "yi.UTF-8"] {
            assert_eq!(
                LayoutDirection::from_locale(rtl),
                LayoutDirection::Rtl,
                "{rtl}"
            );
        }
        for ltr in ["en", "en-US", "zh-CN", "", "C", "arn"] {
            assert_eq!(
                LayoutDirection::from_locale(ltr),
                LayoutDirection::Ltr,
                "{ltr}"
            );
        }
    }

    #[gpui::test]
    fn logical_edges_follow_global_direction(cx: &mut TestAppContext) {
        cx.update(|cx| {
            init(cx);
            assert_eq!(LayoutDirection::global(cx), LayoutDirection::Ltr);

            let mut ltr = div().ps(px(4.), cx).me(px(8.), cx).border_s(px(1.), cx);
            let style = ltr.style();
            assert_eq!(style.padding.left, Some(px(4.).into()));
            assert_eq!(style.margin.right, Some(px(8.).into()));
            assert_eq!(style.border_widths.left, Some(px(1.).into()));

            LayoutDirection::Rtl.set_global(cx);
            let mut rtl = div().ps(px(4.), cx).me(px(8.), cx).border_s(px(1.), cx);
            let style = rtl.style();
            assert_eq!(style.padding.right, Some(px(4.).into()));
            assert_eq!(style.padding.left, None);
            assert_eq!(style.margin.left, Some(px(8.).into()));
            assert_eq!(style.border_widths.right, Some(px(1.).into()));
        });
    }

    #[gpui::test]
    fn flex_justification_follows_row_direction(cx: &mut TestAppContext) {
        use gpui::{
            Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Window,
        };

        struct Row;
        impl Render for Row {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .flex_row_dir(cx)
                    .w(px(100.))
                    .justify_flex_end()
                    .child(div().size(px(10.)).debug_selector(|| "item".into()))
            }
        }

        cx.update(init);
        for (direction, expected_left) in [
            (LayoutDirection::Ltr, px(90.)),
            (LayoutDirection::Rtl, px(0.)),
        ] {
            cx.update(|cx| direction.set_global(cx));
            let (_, window) = cx.add_window_view(|_, _| Row);
            window.update(|window, cx| window.draw(cx).clear());
            let item = window.debug_bounds("item").expect("item not rendered");
            assert_eq!(item.left(), expected_left, "{direction:?}");
        }
    }

    #[gpui::test]
    fn text_alignment_follows_global_direction(cx: &mut TestAppContext) {
        use gpui::{Div, TextAlign};

        // Port note: 0.2 keeps text style in an `Option`, unlike 0.3's inline
        // refinement; read through the accessor instead of a `.text` field.
        fn aligned(el: &mut Div) -> Option<TextAlign> {
            el.style().text.as_ref().and_then(|text| text.text_align)
        }

        cx.update(|cx| {
            init(cx);
            assert_eq!(aligned(&mut div().text_start(cx)), Some(TextAlign::Left));
            assert_eq!(aligned(&mut div().text_end(cx)), Some(TextAlign::Right));

            LayoutDirection::Rtl.set_global(cx);
            assert_eq!(aligned(&mut div().text_start(cx)), Some(TextAlign::Right));
            assert_eq!(aligned(&mut div().text_end(cx)), Some(TextAlign::Left));
        });
    }
}
