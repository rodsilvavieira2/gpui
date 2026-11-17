//! Keyframe tracks and discrete switches over normalized progress.
//!
//! Ported from `longbridge/gpui-kit` (`crates/base/src/motion/keyframes.rs`) at
//! commit `cfab7b2` (Apache-2.0). No functional changes; docs added for
//! `#![deny(missing_docs)]`.

use std::sync::Arc;

use super::{Easing, Interpolate};

/// One offset, value, and segment easing of a [`Keyframes`] track.
#[derive(Clone, Debug)]
pub struct Keyframe<T> {
    /// Normalized position of this frame in `0..=1`.
    pub offset: f32,
    /// The value at this frame.
    pub value: T,
    /// The easing into the following segment.
    pub easing: Easing,
}

impl<T> Keyframe<T> {
    /// A keyframe at `offset` (0 to 1) holding `value`, with linear easing.
    pub fn new(offset: f32, value: T) -> Self {
        Self {
            offset,
            value,
            easing: Easing::Linear,
        }
    }

    /// Sets the easing into the following segment.
    pub fn ease(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }
}

/// Why [`Keyframes::try_new`] rejected a frame list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyframeError {
    /// Fewer than two frames.
    TooFewFrames,
    /// A non-finite offset.
    OffsetNotFinite,
    /// An offset outside `0..=1`.
    OffsetOutOfRange,
    /// Offsets that do not rise monotonically.
    OffsetsNotMonotonic,
    /// Missing 0 or 1 endpoint.
    MissingEndpoint,
}

/// A validated track of [`Keyframe`]s sampled by normalized progress.
#[derive(Clone, Debug)]
pub struct Keyframes<T> {
    frames: Arc<[Keyframe<T>]>,
}

impl<T: Interpolate> Keyframes<T> {
    /// Validates and builds a track: at least two frames, finite offsets in
    /// `0..=1`, monotonically non-decreasing, anchored at 0 and 1.
    pub fn try_new(frames: impl IntoIterator<Item = Keyframe<T>>) -> Result<Self, KeyframeError> {
        let frames: Vec<_> = frames.into_iter().collect();
        if frames.len() < 2 {
            return Err(KeyframeError::TooFewFrames);
        }
        if frames.iter().any(|frame| !frame.offset.is_finite()) {
            return Err(KeyframeError::OffsetNotFinite);
        }
        if frames
            .iter()
            .any(|frame| !(0.0..=1.0).contains(&frame.offset))
        {
            return Err(KeyframeError::OffsetOutOfRange);
        }
        if frames
            .windows(2)
            .any(|pair| pair[0].offset > pair[1].offset)
        {
            return Err(KeyframeError::OffsetsNotMonotonic);
        }
        if frames.first().unwrap().offset != 0.0 || frames.last().unwrap().offset != 1.0 {
            return Err(KeyframeError::MissingEndpoint);
        }
        Ok(Self {
            frames: frames.into(),
        })
    }

    /// Interpolates the value at `progress` (clamped to `0..=1`).
    #[inline]
    pub fn sample(&self, progress: f32) -> T {
        let progress = progress.clamp(0.0, 1.0);
        let upper = self
            .frames
            .partition_point(|frame| frame.offset <= progress);
        if upper == 0 {
            return self.frames[0].value.clone();
        }
        if upper == self.frames.len() {
            return self.frames[self.frames.len() - 1].value.clone();
        }
        let from = &self.frames[upper - 1];
        let to = &self.frames[upper];
        if from.offset == to.offset {
            return to.value.clone();
        }
        let segment = (progress - from.offset) / (to.offset - from.offset);
        from.value
            .interpolate(&to.value, from.easing.sample(segment))
    }

    /// The number of frames in the track.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether the track holds no frames (only possible when built without
    /// [`Self::try_new`]).
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// Why [`Discrete::switch_at`] rejected a switch point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscreteError {
    /// A non-finite switch point, or one outside `0..=1`.
    InvalidSwitchPoint,
}

/// A two-value switch at one progress point (no interpolation).
#[derive(Clone, Debug)]
pub struct Discrete<T> {
    from: T,
    to: T,
    switch_at: f32,
}

impl<T> Discrete<T> {
    /// Switches from `from` to `to`, by default halfway through.
    pub fn new(from: T, to: T) -> Self {
        Self {
            from,
            to,
            switch_at: 0.5,
        }
    }

    /// Sets the progress at which the value switches.
    pub fn switch_at(mut self, progress: f32) -> Result<Self, DiscreteError> {
        if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
            return Err(DiscreteError::InvalidSwitchPoint);
        }
        self.switch_at = progress;
        Ok(self)
    }
}

impl<T: Clone> Discrete<T> {
    /// Returns `from` before the switch point, `to` at and after it.
    #[inline]
    pub fn sample(&self, progress: f32) -> T {
        if progress < self.switch_at {
            self.from.clone()
        } else {
            self.to.clone()
        }
    }
}
