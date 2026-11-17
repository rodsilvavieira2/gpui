//! Enter/exit presence with interruption-safe reversing.
//!
//! Ported from `longbridge/gpui-kit` (`crates/base/src/motion/presence.rs`) at
//! commit `cfab7b2` (Apache-2.0). No functional changes; docs added for
//! `#![deny(missing_docs)]`.

use std::time::Duration;

use gpui::{App, ElementId, Window};

use super::{MotionStatus, Transition, TransitionId};

/// Where a [`Presence`] is in its enter/exit lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresencePhase {
    /// Animating from absent toward present.
    Entering,
    /// Settled at present.
    Present,
    /// Animating from present toward absent.
    Exiting,
    /// Settled at absent; renders nothing.
    Absent,
}

/// One sample of a [`Presence`]: phase, progress, and motion status.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresenceSample {
    /// The lifecycle phase of this sample.
    pub phase: PresencePhase,
    /// Normalized progress from absent (0) to present (1).
    pub progress: f32,
    /// The motion status of this sample.
    pub status: MotionStatus,
}

impl PresenceSample {
    /// Whether the element should render this frame (every phase but absent).
    pub const fn should_render(self) -> bool {
        !matches!(self.phase, PresencePhase::Absent)
    }
}

/// Tracks whether content is present, entering, exiting, or absent.
pub struct Presence {
    id: TransitionId,
    present: bool,
    transition: Transition,
}

impl Presence {
    /// Tracks `present` under `id`, with a zero-duration transition until
    /// [`Self::transition`] sets the policy.
    pub fn new(id: impl Into<TransitionId>, present: bool) -> Self {
        Self {
            id: id.into(),
            present,
            transition: Transition::new(Duration::ZERO),
        }
    }

    /// Sets the transition policy for entering and exiting.
    pub fn transition(mut self, transition: Transition) -> Self {
        self.transition = transition;
        self
    }

    /// Samples the presence and requests a frame while entering or exiting.
    pub fn sample(self, window: &mut Window, cx: &mut App) -> PresenceSample {
        let id = ElementId::NamedChild(self.id.0.into(), "__presence".into());
        let now = cx.background_executor().now();
        let target = if self.present { 1.0 } else { 0.0 };
        let state = window.use_keyed_state(id, cx, |_, _| PresenceState {
            from: 0.0,
            target,
            started_at: now,
            reversing_factor: 1.0,
            duration: self.transition.duration,
        });
        let snapshot = *state.read(cx);

        if cx.reduce_motion() || self.transition.duration.is_zero() {
            if snapshot.from != target || snapshot.target != target {
                state.update(cx, |state, _| {
                    state.from = target;
                    state.target = target;
                    state.started_at = now;
                    state.reversing_factor = 1.0;
                    state.duration = self.transition.duration;
                });
            }
            return stable_sample(self.present);
        }

        let elapsed = now.saturating_duration_since(snapshot.started_at);
        let (progress, status) = self.transition.progress(elapsed, snapshot.duration);
        let sampled = interpolate(
            snapshot.from,
            snapshot.target,
            self.transition.sample(progress),
        );

        let (progress, status) = if snapshot.target != target {
            let reversing = target == snapshot.from;
            let reversing_factor = if reversing {
                (self.transition.sample(progress) * snapshot.reversing_factor
                    + (1.0 - snapshot.reversing_factor))
                    .clamp(0.0, 1.0)
            } else {
                1.0
            };
            let duration = self
                .transition
                .duration
                .mul_f64(f64::from(reversing_factor));
            state.update(cx, |state, _| {
                state.from = sampled;
                state.target = target;
                state.started_at = now;
                state.reversing_factor = reversing_factor;
                state.duration = duration;
            });
            let (initial, status) = self.transition.progress(Duration::ZERO, duration);
            (
                interpolate(sampled, target, self.transition.sample(initial)),
                status,
            )
        } else {
            (sampled, status)
        };

        if matches!(status, MotionStatus::Delayed | MotionStatus::Running) {
            window.request_animation_frame();
        }

        if status == MotionStatus::Finished {
            stable_sample(self.present)
        } else {
            PresenceSample {
                phase: if self.present {
                    PresencePhase::Entering
                } else {
                    PresencePhase::Exiting
                },
                progress,
                status,
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PresenceState {
    from: f32,
    target: f32,
    started_at: super::Instant,
    reversing_factor: f32,
    duration: Duration,
}

fn stable_sample(present: bool) -> PresenceSample {
    PresenceSample {
        phase: if present {
            PresencePhase::Present
        } else {
            PresencePhase::Absent
        },
        progress: if present { 1.0 } else { 0.0 },
        status: MotionStatus::Finished,
    }
}

fn interpolate(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}
