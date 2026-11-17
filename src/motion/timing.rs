//! CSS-style timing: signed delays, iteration counts, playback direction.
//!
//! Ported from `longbridge/gpui-kit` (`crates/base/src/motion/timing.rs`) at
//! commit `cfab7b2` (Apache-2.0). No functional changes; docs added for
//! `#![deny(missing_docs)]`.

use std::time::Duration;

use super::Easing;

/// A delay that can hold playback (positive) or start it inside its active
/// interval (negative).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignedDuration {
    /// Hold playback for this long before starting.
    Positive(Duration),
    /// Start playback as if it began this long ago.
    Negative(Duration),
}

impl SignedDuration {
    /// Zero delay.
    pub const ZERO: Self = Self::Positive(Duration::ZERO);

    /// A holding delay: playback starts after `duration`.
    pub const fn positive(duration: Duration) -> Self {
        Self::Positive(duration)
    }

    /// A starting-inside-the-interval delay: playback starts as if it began
    /// `duration` ago.
    pub const fn negative(duration: Duration) -> Self {
        Self::Negative(duration)
    }

    pub(crate) fn active_elapsed(self, elapsed: Duration) -> Option<Duration> {
        match self {
            Self::Positive(delay) => elapsed.checked_sub(delay),
            Self::Negative(delay) => Some(elapsed.saturating_add(delay)),
        }
    }
}

impl Default for SignedDuration {
    fn default() -> Self {
        Self::ZERO
    }
}

/// How many times a timed playback repeats.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IterationCount {
    /// Run exactly this many iterations, then finish.
    Finite(u64),
    /// Run forever.
    Infinite,
}

impl Default for IterationCount {
    fn default() -> Self {
        Self::Finite(1)
    }
}

/// Which way each iteration of a timed playback runs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PlaybackDirection {
    /// Every iteration runs from 0 to 1.
    #[default]
    Normal,
    /// Every iteration runs from 1 to 0.
    Reverse,
    /// Odd iterations run reversed.
    Alternate,
    /// Even iterations run reversed.
    AlternateReverse,
}

/// Where a sample falls relative to a timed playback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionPhase {
    /// Before the delay has elapsed.
    Before,
    /// Inside an iteration.
    Active,
    /// After the last iteration.
    After,
}

/// One sample of a [`Timing`] policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimingSample {
    /// Where the sample falls relative to the playback.
    pub phase: MotionPhase,
    /// Eased progress in the current direction.
    pub directed_progress: f32,
    /// The iteration being sampled.
    pub iteration: u64,
    /// Whether the playback is currently active.
    pub active: bool,
    /// Whether the playback has finished.
    pub finished: bool,
}

/// A CSS-like timing policy: delay, duration, iterations, direction, easing.
#[derive(Clone, Debug)]
pub struct Timing {
    delay: SignedDuration,
    duration: Duration,
    iterations: IterationCount,
    direction: PlaybackDirection,
    easing: Easing,
}

impl Timing {
    /// A single normal-direction run of `duration` with linear easing.
    pub fn new(duration: Duration) -> Self {
        Self {
            delay: SignedDuration::ZERO,
            duration,
            iterations: IterationCount::Finite(1),
            direction: PlaybackDirection::Normal,
            easing: Easing::Linear,
        }
    }

    /// Sets the start delay (see [`SignedDuration`]).
    pub fn delay(mut self, delay: SignedDuration) -> Self {
        self.delay = delay;
        self
    }

    /// Sets how many times the playback repeats.
    pub fn iterations(mut self, iterations: IterationCount) -> Self {
        self.iterations = iterations;
        self
    }

    /// Sets which way each iteration runs.
    pub fn direction(mut self, direction: PlaybackDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Sets the easing applied to directed progress.
    pub fn ease(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Samples the playback at `elapsed` since its start.
    pub fn sample(&self, elapsed: Duration) -> TimingSample {
        let Some(active_elapsed) = self.delay.active_elapsed(elapsed) else {
            return TimingSample {
                phase: MotionPhase::Before,
                directed_progress: self.easing.sample(self.directed(0, 0.0)),
                iteration: 0,
                active: false,
                finished: false,
            };
        };

        let finite_iterations = match self.iterations {
            IterationCount::Finite(count) => Some(count),
            IterationCount::Infinite => None,
        };
        if self.duration.is_zero() || finite_iterations == Some(0) {
            return self.after_sample(finite_iterations.unwrap_or(1));
        }

        let duration_nanos = self.duration.as_nanos();
        let elapsed_nanos = active_elapsed.as_nanos();
        if let Some(count) = finite_iterations
            && elapsed_nanos >= duration_nanos.saturating_mul(count as u128)
        {
            return self.after_sample(count);
        }

        let iteration = (elapsed_nanos / duration_nanos).min(u64::MAX as u128) as u64;
        let iteration_nanos = elapsed_nanos % duration_nanos;
        let progress = iteration_nanos as f64 / duration_nanos as f64;
        TimingSample {
            phase: MotionPhase::Active,
            directed_progress: self
                .easing
                .sample(self.directed(iteration, progress as f32)),
            iteration,
            active: true,
            finished: false,
        }
    }

    fn after_sample(&self, count: u64) -> TimingSample {
        let iteration = count.saturating_sub(1);
        TimingSample {
            phase: MotionPhase::After,
            directed_progress: self.easing.sample(self.directed(iteration, 1.0)),
            iteration,
            active: false,
            finished: true,
        }
    }

    fn directed(&self, iteration: u64, progress: f32) -> f32 {
        let reverse = match self.direction {
            PlaybackDirection::Normal => false,
            PlaybackDirection::Reverse => true,
            PlaybackDirection::Alternate => !iteration.is_multiple_of(2),
            PlaybackDirection::AlternateReverse => iteration.is_multiple_of(2),
        };
        if reverse { 1.0 - progress } else { progress }
    }
}
