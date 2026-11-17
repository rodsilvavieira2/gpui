//! Staggered per-item delays for list reveals.
//!
//! Ported from `longbridge/gpui-kit` (`crates/base/src/motion/stagger.rs`) at
//! commit `cfab7b2` (Apache-2.0). No functional changes; docs added for
//! `#![deny(missing_docs)]`.

use std::time::Duration;

/// Which item a [`Stagger`] counts delays outward from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaggerOrigin {
    /// The first item plays first.
    First,
    /// The last item plays first.
    Last,
    /// The middle item plays first.
    Center,
    /// The item at this index plays first.
    Index(usize),
}

/// A fixed per-item delay interval expanding from an origin item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stagger {
    interval: Duration,
    origin: StaggerOrigin,
}

impl Stagger {
    /// A stagger of `interval` per item expanding from `origin`.
    pub const fn new(interval: Duration, origin: StaggerOrigin) -> Self {
        Self { interval, origin }
    }

    /// The delay for `index` of `count` items.
    #[inline]
    pub fn delay(&self, index: usize, count: usize) -> Duration {
        if count == 0 {
            return Duration::ZERO;
        }
        let index = index.min(count - 1);
        let origin = match self.origin {
            StaggerOrigin::First => 0,
            StaggerOrigin::Last => count - 1,
            StaggerOrigin::Center => (count - 1) / 2,
            StaggerOrigin::Index(origin) => origin.min(count - 1),
        };
        self.interval
            .saturating_mul(index.abs_diff(origin).min(u32::MAX as usize) as u32)
    }
}
