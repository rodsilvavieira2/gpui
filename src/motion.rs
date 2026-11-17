//! Target-driven motion: transitions, sequences, presence, keyframes.
//!
//! Ported from `longbridge/gpui-kit` (`crates/base/src/motion.rs`) at commit
//! `cfab7b2` (Apache-2.0). Adapted for GPUI 0.2:
//!
//! - The `Spring`/`spring()` API is not ported: it builds on GPUI 0.3's native
//!   `SpringConfig`/`SpringState`/`SpringTarget`, which do not exist here.
//! - `std::time::Instant` is used unconditionally (no `web_time`/wasm target).
//! - The easing helpers live in [`effects`] (sibling here) instead of beside
//!   this module as `animation`.
//! - Docs added for `#![deny(missing_docs)]`.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{App, Bounds, ElementId, Pixels, SharedString, Size, Window};

use effects::Lerp;

mod easing;
mod effects;
mod keyframes;
mod presence;
mod reveal;
mod sequence;
mod stagger;
mod timing;

pub use easing::{Easing, EasingError, LinearStop, StepPosition};
pub use effects::{
    EffectTransition, cubic_bezier, ease_in_cubic, ease_in_out_cubic, ease_out_cubic,
};
pub use keyframes::{Discrete, DiscreteError, Keyframe, KeyframeError, Keyframes};
pub use presence::{Presence, PresencePhase, PresenceSample};
pub use reveal::MotionReveal;
pub use sequence::{Sequence, SequenceSample, SequenceStep};
pub use stagger::{Stagger, StaggerOrigin};
pub use timing::{
    IterationCount, MotionPhase, PlaybackDirection, SignedDuration, Timing, TimingSample,
};

/// A value that can be interpolated between two application-owned targets.
pub trait Interpolate: Clone {
    /// Interpolates from `self` toward `target` at normalized `progress`.
    fn interpolate(&self, target: &Self, progress: f32) -> Self;
}

impl<T: Lerp> Interpolate for T {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        self.lerp(target, progress)
    }
}

impl Interpolate for Size<Pixels> {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Size::new(
            self.width.lerp(&target.width, progress),
            self.height.lerp(&target.height, progress),
        )
    }
}

impl Interpolate for Bounds<Pixels> {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Bounds::new(
            self.origin.lerp(&target.origin, progress),
            self.size.interpolate(&target.size, progress),
        )
    }
}

/// A presentation-neutral bundle for coordinated paint transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionTransform {
    /// Offset from the laid-out position.
    pub translation: gpui::Point<Pixels>,
    /// Horizontal and vertical scale factors.
    pub scale: gpui::Point<f32>,
    /// Clockwise rotation in radians.
    pub rotation_radians: f32,
    /// Opacity multiplier.
    pub opacity: f32,
}

impl MotionTransform {
    /// The identity transform: no translation, unit scale, no rotation, opaque.
    pub fn identity() -> Self {
        Self {
            translation: gpui::point(gpui::px(0.0), gpui::px(0.0)),
            scale: gpui::point(1.0, 1.0),
            rotation_radians: 0.0,
            opacity: 1.0,
        }
    }
}

impl Default for MotionTransform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Interpolate for MotionTransform {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Self {
            translation: self.translation.lerp(&target.translation, progress),
            scale: gpui::point(
                self.scale.x.lerp(&target.scale.x, progress),
                self.scale.y.lerp(&target.scale.y, progress),
            ),
            rotation_radians: self
                .rotation_radians
                .lerp(&target.rotation_radians, progress),
            opacity: self.opacity.lerp(&target.opacity, progress),
        }
    }
}

/// How a value transition responds to the application's reduced-motion setting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReducedMotion {
    /// Adopt the target immediately. Appropriate for spatial motion.
    #[default]
    Finish,
    /// Keep the transition timing, for non-spatial effects such as fades.
    Continue,
}

/// CSS-like timing policy for a target-value transition.
///
/// This type is intentionally separate from [`EffectTransition`],
/// whose legacy interface applies concrete fade, slide, and size effects to an
/// element. A value transition never chooses a visual property for the caller.
#[derive(Clone)]
pub struct Transition {
    duration: Duration,
    delay: SignedDuration,
    easing: Easing,
    reduced_motion: ReducedMotion,
}

impl Transition {
    /// A transition of `duration` with cubic ease-out and `Finish` reduced
    /// motion: spatial motion that adopts its target when reduced motion is on.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: SignedDuration::ZERO,
            easing: Easing::Custom(Rc::new(ease_out_cubic)),
            reduced_motion: ReducedMotion::Finish,
        }
    }

    /// Select reduced-motion behavior for this transition only.
    /// The application's preference and other motion channels are unchanged.
    ///
    /// [`transition`], [`transition_with_status`] and
    /// [`TransitionState::sample`] read it. [`Presence`] still finishes at
    /// once under reduced motion.
    pub fn reduced_motion(mut self, behavior: ReducedMotion) -> Self {
        self.reduced_motion = behavior;
        self
    }

    /// Sets the start delay. A negative delay starts the transition inside
    /// its active interval.
    pub fn delay(mut self, delay: impl Into<SignedDuration>) -> Self {
        self.delay = delay.into();
        self
    }

    /// Sets the easing from a raw progress-mapping function.
    pub fn ease(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Easing::Custom(Rc::new(easing));
        self
    }

    /// Sets the easing policy.
    pub fn easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    fn sample(&self, progress: f32) -> f32 {
        self.easing.sample(progress)
    }

    fn progress(&self, elapsed: Duration, duration: Duration) -> (f32, MotionStatus) {
        let Some(active_elapsed) = self.delay.active_elapsed(elapsed) else {
            return (0.0, MotionStatus::Delayed);
        };
        if duration.is_zero() || active_elapsed >= duration {
            return (1.0, MotionStatus::Finished);
        }
        (
            active_elapsed.as_secs_f32() / duration.as_secs_f32(),
            MotionStatus::Running,
        )
    }

    /// The elapsed time at which [`Self::progress`] over the transition's own
    /// duration first reports `Finished`.
    fn finishes_after(&self) -> Duration {
        match self.delay {
            SignedDuration::Positive(delay) => delay.saturating_add(self.duration),
            SignedDuration::Negative(delay) => self.duration.saturating_sub(delay),
        }
    }
}

impl From<Duration> for SignedDuration {
    fn from(duration: Duration) -> Self {
        Self::positive(duration)
    }
}

/// Identifies one independently transitioning value.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TransitionId(ElementId);

impl From<ElementId> for TransitionId {
    fn from(id: ElementId) -> Self {
        Self(id)
    }
}

impl From<&'static str> for TransitionId {
    fn from(id: &'static str) -> Self {
        Self(id.into())
    }
}

impl From<String> for TransitionId {
    fn from(id: String) -> Self {
        // Port note: GPUI 0.2 has no `ElementId: From<String>`; route
        // through `SharedString`, which it does implement.
        Self(SharedString::from(id).into())
    }
}

impl From<SharedString> for TransitionId {
    fn from(id: SharedString) -> Self {
        Self(id.into())
    }
}

impl From<usize> for TransitionId {
    fn from(id: usize) -> Self {
        Self(id.into())
    }
}

impl From<i32> for TransitionId {
    fn from(id: i32) -> Self {
        Self(id.into())
    }
}

impl From<TransitionId> for ElementId {
    fn from(id: TransitionId) -> Self {
        ElementId::NamedChild(id.0.into(), "__base-transition-state".into())
    }
}

impl<I, C> From<(I, C)> for TransitionId
where
    I: Into<ElementId>,
    C: Into<SharedString>,
{
    fn from((id, channel): (I, C)) -> Self {
        Self(ElementId::NamedChild(id.into().into(), channel.into()))
    }
}

#[derive(Clone)]
struct ValueTransition<T> {
    from: T,
    /// Where the running transition logically started, before any reversal
    /// shortened it: what a CSS transition calls its reversing-adjusted start.
    reversing_adjusted_start: T,
    /// The policy the running transition started with, kept until it is
    /// retargeted.
    policy: Transition,
    target: T,
    started_at: Instant,
    reversing_factor: f32,
    duration: Duration,
}

/// The lifecycle status of a sampled motion value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionStatus {
    /// At rest at its target; requests no frame.
    Idle,
    /// Waiting out a positive delay.
    Delayed,
    /// Interpolating toward its target.
    Running,
    /// Settled at its target.
    Finished,
}

/// A sampled motion value with its lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionValue<T> {
    /// The interpolated value for this frame.
    pub value: T,
    /// The lifecycle status of this sample.
    pub status: MotionStatus,
}

/// Caller-owned state for one target-value transition, independent of mounting.
///
/// Keyed state lives only while its element renders, so a virtualized row that
/// scrolls out and back restarts at its target. Keep a `TransitionState` with the
/// row's data instead and sample it while rendering:
///
/// ```ignore
/// let opacity = row.highlight.sample(target, policy, window, cx).value;
/// ```
///
/// Clones share its clock and interruption/reversal history. The first sample
/// adopts its target immediately; later samples use the same algorithm as
/// [`transition_with_status`]. No view or window is retained. A new handle
/// resets it.
#[derive(Clone)]
pub struct TransitionState<T> {
    state: Rc<RefCell<Option<ValueTransition<T>>>>,
}

impl<T> Default for TransitionState<T> {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(None)),
        }
    }
}

impl<T: Interpolate + PartialEq + 'static> TransitionState<T> {
    /// Returns the current value toward `target`, as [`transition_with_status`]
    /// does, and requests the next animation frame while the status is
    /// `Delayed` or `Running`.
    ///
    /// Call this while rendering an element. A target that changes while the
    /// element is not rendered starts transitioning at the next sample.
    pub fn sample(
        &self,
        target: T,
        policy: Transition,
        window: &mut Window,
        cx: &mut App,
    ) -> MotionValue<T> {
        let now = cx.background_executor().now();
        let mut state = self.state.borrow_mut();
        let state = state.get_or_insert_with(|| ValueTransition {
            from: target.clone(),
            reversing_adjusted_start: target.clone(),
            policy: policy.clone(),
            target: target.clone(),
            started_at: now,
            reversing_factor: 1.0,
            duration: policy.duration,
        });
        let snapshot = state.clone();
        let sampled = sample_transition(
            &snapshot,
            target,
            policy,
            now,
            cx.reduce_motion(),
            |update| update(state),
        );
        if matches!(
            sampled.status,
            MotionStatus::Delayed | MotionStatus::Running
        ) {
            window.request_animation_frame();
        }
        sampled
    }
}

/// Returns the current value for a CSS-like transition toward `target`.
///
/// State is keyed by `id`. The first value is adopted immediately; later target
/// changes transition from the value sampled at that instant. Components opt
/// into this function explicitly—base components do not install default motion.
///
/// Call this while rendering an element, where GPUI keyed element state is
/// available. A channel id must identify one value type within that element.
pub fn transition<T>(
    id: impl Into<TransitionId>,
    target: T,
    policy: Transition,
    window: &mut Window,
    cx: &mut App,
) -> T
where
    T: Interpolate + PartialEq + 'static,
{
    transition_with_status(id, target, policy, window, cx).value
}

/// Returns the current value and status for a CSS-like transition toward
/// `target` (see [`transition`]).
pub fn transition_with_status<T>(
    id: impl Into<TransitionId>,
    target: T,
    policy: Transition,
    window: &mut Window,
    cx: &mut App,
) -> MotionValue<T>
where
    T: Interpolate + PartialEq + 'static,
{
    let id: ElementId = id.into().into();
    let now = cx.background_executor().now();
    let state = window.use_keyed_state(id, cx, |_, _| ValueTransition {
        from: target.clone(),
        reversing_adjusted_start: target.clone(),
        policy: policy.clone(),
        target: target.clone(),
        started_at: now,
        reversing_factor: 1.0,
        duration: policy.duration,
    });

    let snapshot = state.read(cx).clone();
    let reduced = cx.reduce_motion();
    let sampled = sample_transition(&snapshot, target, policy, now, reduced, |update| {
        state.update(cx, |state, _| update(state));
    });
    if matches!(
        sampled.status,
        MotionStatus::Delayed | MotionStatus::Running
    ) {
        window.request_animation_frame();
    }
    sampled
}

fn sample_transition<T>(
    snapshot: &ValueTransition<T>,
    target: T,
    policy: Transition,
    now: Instant,
    reduced_motion: bool,
    mut update: impl FnMut(&mut dyn FnMut(&mut ValueTransition<T>)),
) -> MotionValue<T>
where
    T: Interpolate + PartialEq + 'static,
{
    if (reduced_motion && policy.reduced_motion == ReducedMotion::Finish)
        || policy.duration.is_zero()
    {
        if snapshot.from != target || snapshot.target != target {
            update(&mut |state| {
                state.from = target.clone();
                state.reversing_adjusted_start = target.clone();
                state.policy = policy.clone();
                state.target = target.clone();
                state.started_at = now;
                state.reversing_factor = 1.0;
                state.duration = policy.duration;
            });
        }
        return MotionValue {
            value: target,
            status: MotionStatus::Finished,
        };
    }

    let elapsed = now.saturating_duration_since(snapshot.started_at);
    let (progress, status) = snapshot.policy.progress(elapsed, snapshot.duration);
    let sampled = snapshot
        .from
        .interpolate(&snapshot.target, snapshot.policy.sample(progress));

    let (value, status) = if snapshot.target != target {
        // As CSS transitions do: only a running transition sent back to where
        // it logically started is a reversal.
        let reversing = matches!(status, MotionStatus::Running | MotionStatus::Delayed)
            && target == snapshot.reversing_adjusted_start;
        let reversing_factor = if reversing {
            (snapshot.policy.sample(progress) * snapshot.reversing_factor
                + (1.0 - snapshot.reversing_factor))
                .abs()
                .clamp(0.0, 1.0)
        } else {
            1.0
        };
        let duration = policy.duration.mul_f64(f64::from(reversing_factor));
        // A reversal shortens a negative delay by the same factor.
        let mut active_policy = policy.clone();
        if reversing && let SignedDuration::Negative(delay) = policy.delay {
            active_policy.delay =
                SignedDuration::negative(delay.mul_f64(f64::from(reversing_factor)));
        }
        update(&mut |state| {
            state.reversing_adjusted_start = if reversing {
                snapshot.target.clone()
            } else {
                sampled.clone()
            };
            state.policy = active_policy.clone();
            state.from = sampled.clone();
            state.target = target.clone();
            state.started_at = now;
            state.reversing_factor = reversing_factor;
            state.duration = duration;
        });
        let (initial_progress, initial_status) = active_policy.progress(Duration::ZERO, duration);
        (
            sampled.interpolate(&target, active_policy.sample(initial_progress)),
            initial_status,
        )
    } else {
        (
            sampled,
            if snapshot.from == snapshot.target {
                MotionStatus::Idle
            } else {
                status
            },
        )
    };
    MotionValue { value, status }
}

#[derive(Clone, Copy)]
struct KeyframePlayback {
    started_at: Instant,
}

/// Samples a keyed keyframe playback and requests frames while it is active.
///
/// The stable `id` owns the playback's start time. Re-rendering with the same
/// ID continues that playback; it does not restart when `keyframes` or `timing`
/// is reconstructed. To replay a sequence, include an application-owned
/// generation in the ID, for example `("notification-enter", generation)`.
pub fn animate_keyframes<T>(
    id: impl Into<TransitionId>,
    keyframes: &Keyframes<T>,
    timing: Timing,
    window: &mut Window,
    cx: &mut App,
) -> MotionValue<T>
where
    T: Interpolate + 'static,
{
    let id: TransitionId = id.into();
    let id = ElementId::NamedChild(ElementId::from(id).into(), "__keyframes".into());
    let now = cx.background_executor().now();
    let state = window.use_keyed_state(id, cx, |_, _| KeyframePlayback { started_at: now });
    let started_at = state.read(cx).started_at;

    if cx.reduce_motion() {
        return MotionValue {
            value: keyframes.sample(1.0),
            status: MotionStatus::Finished,
        };
    }

    let sample = timing.sample(now.saturating_duration_since(started_at));
    let status = match sample.phase {
        MotionPhase::Before => MotionStatus::Delayed,
        MotionPhase::Active => MotionStatus::Running,
        MotionPhase::After => MotionStatus::Finished,
    };
    if matches!(status, MotionStatus::Delayed | MotionStatus::Running) {
        window.request_animation_frame();
    }
    MotionValue {
        value: keyframes.sample(sample.directed_progress),
        status,
    }
}

#[cfg(test)]
mod css_timing_tests {
    use super::{
        Easing, IterationCount, LinearStop, MotionPhase, PlaybackDirection, SignedDuration,
        StepPosition, Timing,
    };
    use std::{num::NonZeroU32, sync::Arc, time::Duration};

    #[test]
    fn css_keyword_easing_matches_published_reference_samples() {
        for (easing, samples) in [
            (Easing::Ease, [(0.2, 0.295), (0.5, 0.802), (0.8, 0.976)]),
            (Easing::EaseIn, [(0.2, 0.062), (0.5, 0.315), (0.8, 0.692)]),
            (Easing::EaseOut, [(0.2, 0.308), (0.5, 0.685), (0.8, 0.938)]),
            (Easing::EaseInOut, [(0.2, 0.082), (0.5, 0.5), (0.8, 0.918)]),
        ] {
            for (progress, expected) in samples {
                let actual = easing.sample(progress);
                assert!(
                    (actual - expected).abs() < 0.002,
                    "{easing:?}({progress}) = {actual}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn cubic_bezier_easing_keeps_anticipation_and_overshoot() {
        let anticipate = Easing::cubic_bezier(1.0, -0.4, 0.35, 0.95).unwrap();
        let overshoot = Easing::cubic_bezier(0.175, 0.885, 0.32, 1.275).unwrap();

        assert!(anticipate.sample(0.2) < 0.0);
        assert!(overshoot.sample(0.8) > 1.0);
        for easing in [anticipate, overshoot] {
            assert_eq!(easing.sample(-1.0), 0.0);
            assert_eq!(easing.sample(2.0), 1.0);
        }
    }

    #[test]
    fn step_easing_observes_css_jump_positions() {
        let start = Easing::steps(4, StepPosition::JumpStart).unwrap();
        let end = Easing::steps(4, StepPosition::JumpEnd).unwrap();

        assert_eq!(start.sample(0.0), 0.25);
        assert_eq!(start.sample(0.24), 0.25);
        assert_eq!(start.sample(0.25), 0.5);
        assert_eq!(end.sample(0.0), 0.0);
        assert_eq!(end.sample(0.24), 0.0);
        assert_eq!(end.sample(0.25), 0.25);
        assert!(Easing::steps(0, StepPosition::JumpEnd).is_err());

        let none = Easing::steps(4, StepPosition::JumpNone).unwrap();
        let both = Easing::steps(4, StepPosition::JumpBoth).unwrap();
        assert_eq!(none.sample(0.0), 0.0);
        assert!((none.sample(0.5) - 2.0 / 3.0).abs() < f32::EPSILON);
        assert_eq!(none.sample(1.0), 1.0);
        assert_eq!(both.sample(0.0), 0.2);
        assert_eq!(both.sample(1.0), 1.0);
        assert!(Easing::steps(1, StepPosition::JumpNone).is_err());
    }

    #[test]
    fn linear_stops_fill_omitted_positions_before_sampling() {
        let easing = Easing::linear_stops([
            LinearStop::at(0.0, 0.0),
            LinearStop::new(0.2),
            LinearStop::new(0.8),
            LinearStop::at(1.0, 1.0),
        ])
        .unwrap();

        assert!((easing.sample(1.0 / 3.0) - 0.2).abs() < 1e-6);
        assert!((easing.sample(0.5) - 0.5).abs() < 1e-6);
        assert!(
            Easing::linear_stops([LinearStop::at(0.0, 0.8), LinearStop::at(1.0, 0.2)]).is_err()
        );
    }

    #[test]
    fn curves_built_without_stops_or_jumps_sample_linearly() {
        // Both are rejected by the constructors, but the variants are public.
        let no_stops = Easing::LinearStops(Arc::from([]));
        let no_jumps = Easing::Steps {
            count: NonZeroU32::MIN,
            position: StepPosition::JumpNone,
        };

        for (progress, expected) in [(-1.0, 0.0), (0.25, 0.25), (2.0, 1.0)] {
            assert_eq!(
                no_stops.sample(progress),
                expected,
                "no stops at {progress}"
            );
            assert_eq!(
                no_jumps.sample(progress),
                expected,
                "no jumps at {progress}"
            );
        }
    }

    #[test]
    fn negative_delay_starts_inside_the_active_interval() {
        let timing = Timing::new(Duration::from_millis(100))
            .delay(SignedDuration::negative(Duration::from_millis(25)));
        let sample = timing.sample(Duration::ZERO);

        assert_eq!(sample.phase, MotionPhase::Active);
        assert!((sample.directed_progress - 0.25).abs() < f32::EPSILON);
        assert!(sample.active);
        assert!(!sample.finished);
    }

    #[test]
    fn alternate_direction_reverses_odd_iterations() {
        let timing = Timing::new(Duration::from_millis(100))
            .iterations(IterationCount::Finite(2))
            .direction(PlaybackDirection::Alternate)
            .ease(Easing::Linear);

        let first = timing.sample(Duration::from_millis(25));
        let second = timing.sample(Duration::from_millis(125));
        let finished = timing.sample(Duration::from_millis(200));

        assert_eq!(first.iteration, 0);
        assert_eq!(first.directed_progress, 0.25);
        assert_eq!(second.iteration, 1);
        assert_eq!(second.directed_progress, 0.75);
        assert_eq!(finished.phase, MotionPhase::After);
        assert_eq!(finished.directed_progress, 0.0);
        assert!(finished.finished);
    }
}

#[cfg(test)]
mod motion_track_tests {
    use super::{
        Discrete, Easing, Interpolate as _, Keyframe, KeyframeError, Keyframes, MotionTransform,
        Stagger, StaggerOrigin,
    };
    use gpui::{Bounds, Point, Size, point, px, size};
    use std::time::Duration;

    #[test]
    fn keyframes_validate_offsets_and_sample_each_segments_easing() {
        assert!(matches!(
            Keyframes::try_new([Keyframe::new(0.2, 0.0_f32), Keyframe::new(1.0, 1.0_f32),]),
            Err(KeyframeError::MissingEndpoint)
        ));
        assert!(matches!(
            Keyframes::try_new([
                Keyframe::new(0.0, 0.0_f32),
                Keyframe::new(0.8, 1.0_f32),
                Keyframe::new(0.7, 2.0_f32),
                Keyframe::new(1.0, 3.0_f32),
            ]),
            Err(KeyframeError::OffsetsNotMonotonic)
        ));

        let track = Keyframes::try_new([
            Keyframe::new(0.0, 0.0_f32)
                .ease(Easing::steps(2, super::StepPosition::JumpEnd).unwrap()),
            Keyframe::new(0.5, 10.0_f32).ease(Easing::Linear),
            Keyframe::new(1.0, 20.0_f32),
        ])
        .unwrap();

        assert_eq!(track.sample(0.2), 0.0);
        assert_eq!(track.sample(0.3), 5.0);
        assert_eq!(track.sample(0.75), 15.0);
        assert_eq!(track.sample(1.0), 20.0);
    }

    #[test]
    fn discrete_values_switch_only_at_the_requested_progress() {
        let value = Discrete::new("old", "new").switch_at(0.75).unwrap();
        assert_eq!(value.sample(0.749), "old");
        assert_eq!(value.sample(0.75), "new");
        assert!(Discrete::new(0, 1).switch_at(f32::NAN).is_err());
    }

    #[test]
    fn stagger_origins_produce_stable_delays_without_allocating_a_schedule() {
        let interval = Duration::from_millis(20);
        let first = Stagger::new(interval, StaggerOrigin::First);
        let last = Stagger::new(interval, StaggerOrigin::Last);
        let center = Stagger::new(interval, StaggerOrigin::Center);

        assert_eq!(first.delay(3, 5), Duration::from_millis(60));
        assert_eq!(last.delay(3, 5), Duration::from_millis(20));
        assert_eq!(center.delay(2, 5), Duration::ZERO);
        assert_eq!(center.delay(0, 5), Duration::from_millis(40));
        assert_eq!(first.delay(7, 0), Duration::ZERO);
    }

    #[test]
    fn common_gpui_geometry_interpolates_channel_by_channel() {
        let from_size = size(px(10.0), px(20.0));
        let to_size = size(px(30.0), px(60.0));
        assert_eq!(
            from_size.interpolate(&to_size, 0.25),
            size(px(15.0), px(30.0))
        );

        let from = Bounds::new(point(px(0.0), px(10.0)), from_size);
        let to = Bounds::new(point(px(40.0), px(50.0)), to_size);
        assert_eq!(
            from.interpolate(&to, 0.5),
            Bounds::new(point(px(20.0), px(30.0)), size(px(20.0), px(40.0)))
        );

        let _: Point<gpui::Pixels> = from.origin;
        let _: Size<gpui::Pixels> = from.size;

        let transform = MotionTransform::identity().interpolate(
            &MotionTransform {
                translation: point(px(20.0), px(40.0)),
                scale: point(2.0, 0.5),
                rotation_radians: std::f32::consts::PI,
                opacity: 0.0,
            },
            0.5,
        );
        assert_eq!(transform.translation, point(px(10.0), px(20.0)));
        assert_eq!(transform.scale, point(1.5, 0.75));
        assert_eq!(transform.rotation_radians, std::f32::consts::FRAC_PI_2);
        assert_eq!(transform.opacity, 0.5);
    }
}

#[cfg(test)]
mod tests {
    // Port note: `TestAppContext::open_window(size, _)` does not exist on
    // GPUI 0.2; the fixtures below use `add_window(_)` instead (same views,
    // maximized test bounds — motion sampling is size-independent).
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        time::Duration,
    };

    use gpui::{Empty, IntoElement, Render, TestAppContext, VisualTestContext, WindowHandle};

    use super::*;

    struct StatusView {
        target: Rc<Cell<f32>>,
        policy: Transition,
        samples: Rc<RefCell<Vec<MotionValue<f32>>>>,
    }

    impl Render for StatusView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            self.samples.borrow_mut().push(transition_with_status(
                ("status-test", "value"),
                self.target.get(),
                self.policy.clone(),
                window,
                cx,
            ));
            Empty
        }
    }

    struct StatusFixture {
        window: WindowHandle<StatusView>,
        target: Rc<Cell<f32>>,
        samples: Rc<RefCell<Vec<MotionValue<f32>>>>,
    }

    impl StatusFixture {
        fn open(cx: &mut TestAppContext, policy: Transition) -> Self {
            let target = Rc::new(Cell::new(0.0));
            let samples = Rc::new(RefCell::new(Vec::new()));
            let window = cx.add_window({
                let target = target.clone();
                let samples = samples.clone();
                move |_, _| StatusView {
                    target,
                    policy,
                    samples,
                }
            });
            cx.run_until_parked();
            Self {
                window,
                target,
                samples,
            }
        }

        fn render(&self, cx: &mut TestAppContext, target: f32) -> MotionValue<f32> {
            self.target.set(target);
            self.window
                .update(cx, |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            *self.samples.borrow().last().unwrap()
        }
    }

    #[gpui::test]
    fn status_transition_reports_delay_running_and_finished(cx: &mut TestAppContext) {
        let fixture = StatusFixture::open(
            cx,
            Transition::new(Duration::from_millis(100)).delay(Duration::from_millis(20)),
        );
        assert_eq!(fixture.render(cx, 1.0).status, MotionStatus::Delayed);

        cx.executor().advance_clock(Duration::from_millis(20));
        assert_eq!(fixture.render(cx, 1.0).status, MotionStatus::Running);
        cx.executor().advance_clock(Duration::from_millis(100));
        assert_eq!(fixture.render(cx, 1.0).status, MotionStatus::Finished);
    }

    #[gpui::test]
    fn negative_delay_samples_a_target_change_inside_its_interval(cx: &mut TestAppContext) {
        let fixture = StatusFixture::open(
            cx,
            Transition::new(Duration::from_millis(100))
                .delay(SignedDuration::negative(Duration::from_millis(25)))
                .ease(|t| t),
        );
        let sample = fixture.render(cx, 1.0);
        assert_eq!(sample.status, MotionStatus::Running);
        assert_eq!(sample.value, 0.25);
    }

    #[gpui::test]
    fn a_direct_reversal_shortens_the_return_transition(cx: &mut TestAppContext) {
        let fixture =
            StatusFixture::open(cx, Transition::new(Duration::from_millis(100)).ease(|t| t));
        assert_eq!(fixture.render(cx, 1.0).value, 0.0);
        cx.executor().advance_clock(Duration::from_millis(50));
        assert_eq!(fixture.render(cx, 0.0).value, 0.5);
        cx.executor().advance_clock(Duration::from_millis(25));
        assert_eq!(fixture.render(cx, 0.0).value, 0.25);
    }

    #[gpui::test]
    fn repeated_reversals_keep_the_logical_start(cx: &mut TestAppContext) {
        let fixture =
            StatusFixture::open(cx, Transition::new(Duration::from_millis(1000)).ease(|t| t));
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(400));
        assert!((fixture.render(cx, 0.0).value - 0.4).abs() < 1e-6);
        cx.executor().advance_clock(Duration::from_millis(100));
        assert!((fixture.render(cx, 1.0).value - 0.3).abs() < 1e-6);
        cx.executor().advance_clock(Duration::from_millis(100));
        assert!((fixture.render(cx, 0.0).value - 0.4).abs() < 1e-6);
        cx.executor().advance_clock(Duration::from_millis(401));
        assert_eq!(fixture.render(cx, 0.0).status, MotionStatus::Finished);
        assert_eq!(fixture.render(cx, 0.0).value, 0.0);
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(500));
        assert!((fixture.render(cx, 1.0).value - 0.5).abs() < 1e-6);
    }

    #[gpui::test]
    fn reversal_scales_negative_delay(cx: &mut TestAppContext) {
        let fixture = StatusFixture::open(
            cx,
            Transition::new(Duration::from_millis(1000))
                .delay(SignedDuration::negative(Duration::from_millis(200)))
                .ease(|t| t),
        );
        assert_eq!(fixture.render(cx, 1.0).value, 0.2);
        cx.executor().advance_clock(Duration::from_millis(200));
        // At 0.4, reverse with duration 400ms and delay -80ms.
        assert!((fixture.render(cx, 0.0).value - 0.32).abs() < 1e-6);
        cx.executor().advance_clock(Duration::from_millis(321));
        assert_eq!(fixture.render(cx, 0.0).status, MotionStatus::Finished);
    }

    #[gpui::test]
    fn reversal_uses_absolute_shortening_for_anticipation(cx: &mut TestAppContext) {
        let fixture = StatusFixture::open(
            cx,
            Transition::new(Duration::from_millis(1000)).ease(|t| if t < 0.5 { -t } else { t }),
        );
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(250));
        assert_eq!(fixture.render(cx, 0.0).status, MotionStatus::Running);
        cx.executor().advance_clock(Duration::from_millis(250));
        assert_eq!(fixture.render(cx, 0.0).status, MotionStatus::Finished);
    }

    #[gpui::test]
    fn non_reversing_retarget_uses_full_duration(cx: &mut TestAppContext) {
        let fixture =
            StatusFixture::open(cx, Transition::new(Duration::from_millis(1000)).ease(|t| t));
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(400));
        fixture.render(cx, 0.0);
        cx.executor().advance_clock(Duration::from_millis(100));
        fixture.render(cx, 2.0);
        cx.executor().advance_clock(Duration::from_millis(500));
        assert!((fixture.render(cx, 2.0).value - 1.15).abs() < 1e-6);
    }

    #[gpui::test]
    fn active_policy_is_retained_until_retarget(cx: &mut TestAppContext) {
        let fixture =
            StatusFixture::open(cx, Transition::new(Duration::from_millis(1000)).ease(|t| t));
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(400));
        fixture
            .window
            .update(cx, |view, _, _| {
                view.policy = Transition::new(Duration::from_millis(1000))
                    .ease(|t| t * t)
                    .delay(SignedDuration::negative(Duration::from_millis(200)));
            })
            .unwrap();
        assert!((fixture.render(cx, 1.0).value - 0.4).abs() < 1e-6);
        // Old linear sample 0.4; new quadratic curve starts at 0.2^2.
        assert!((fixture.render(cx, 2.0).value - 0.464).abs() < 1e-6);
    }

    #[gpui::test]
    fn reversal_keeps_positive_delay(cx: &mut TestAppContext) {
        let fixture = StatusFixture::open(
            cx,
            Transition::new(Duration::from_millis(1000))
                .delay(Duration::from_millis(200))
                .ease(|t| t),
        );
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(600));
        assert!((fixture.render(cx, 0.0).value - 0.4).abs() < 1e-6);
        cx.executor().advance_clock(Duration::from_millis(100));
        assert_eq!(fixture.render(cx, 0.0).status, MotionStatus::Delayed);
        cx.executor().advance_clock(Duration::from_millis(501));
        assert_eq!(fixture.render(cx, 0.0).status, MotionStatus::Finished);
    }

    #[gpui::test]
    fn effects_opt_in_keeps_timing_under_reduced_motion(cx: &mut TestAppContext) {
        let fixture = StatusFixture::open(
            cx,
            Transition::new(Duration::from_millis(1000))
                .ease(|t| t)
                .reduced_motion(ReducedMotion::Continue),
        );
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.update(|cx| cx.set_reduce_motion(true));
        assert!((fixture.render(cx, 1.0).value - 0.2).abs() < 1e-6);
        cx.update(|cx| assert!(cx.reduce_motion()));
        cx.executor().advance_clock(Duration::from_millis(800));
        assert_eq!(fixture.render(cx, 1.0).status, MotionStatus::Finished);
        fixture.render(cx, 0.0);
        cx.executor().advance_clock(Duration::from_millis(500));
        assert!((fixture.render(cx, 0.0).value - 0.5).abs() < 1e-6);
        cx.update(|cx| cx.set_reduce_motion(false));
        assert!((fixture.render(cx, 0.0).value - 0.5).abs() < 1e-6);
    }

    #[gpui::test]
    fn current_reduced_motion_policy_and_zero_duration_take_effect(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let fixture = StatusFixture::open(
            cx,
            Transition::new(Duration::from_millis(1000))
                .ease(|t| t)
                .reduced_motion(ReducedMotion::Continue),
        );
        fixture.render(cx, 1.0);
        cx.executor().advance_clock(Duration::from_millis(200));
        fixture
            .window
            .update(cx, |view, _, _| {
                view.policy = view.policy.clone().reduced_motion(ReducedMotion::Finish);
            })
            .unwrap();
        assert_eq!(fixture.render(cx, 1.0).value, 1.0);
        fixture
            .window
            .update(cx, |view, _, _| {
                view.policy = view.policy.clone().reduced_motion(ReducedMotion::Continue);
            })
            .unwrap();
        assert_eq!(fixture.render(cx, 0.0).value, 1.0);
        cx.executor().advance_clock(Duration::from_millis(200));
        assert!((fixture.render(cx, 0.0).value - 0.8).abs() < 1e-6);
        fixture
            .window
            .update(cx, |view, _, _| {
                view.policy =
                    Transition::new(Duration::ZERO).reduced_motion(ReducedMotion::Continue);
            })
            .unwrap();
        assert_eq!(fixture.render(cx, 0.0).value, 0.0);
    }

    struct PairView {
        target: f32,
        policy: Transition,
        state: TransitionState<f32>,
        samples: Rc<RefCell<Vec<(MotionValue<f32>, MotionValue<f32>)>>>,
    }

    impl Render for PairView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let keyed = transition_with_status(
                ("pair-test", "value"),
                self.target,
                self.policy.clone(),
                window,
                cx,
            );
            let owned = self
                .state
                .sample(self.target, self.policy.clone(), window, cx);
            self.samples.borrow_mut().push((keyed, owned));
            Empty
        }
    }

    #[gpui::test]
    fn transition_state_matches_keyed_transition_retarget_reverse_and_delay(
        cx: &mut TestAppContext,
    ) {
        for policy in [
            Transition::new(Duration::from_millis(100)).easing(Easing::Linear),
            Transition::new(Duration::from_millis(100))
                .easing(Easing::Ease)
                .delay(SignedDuration::negative(Duration::from_millis(10))),
        ] {
            let samples = Rc::new(RefCell::new(Vec::new()));
            let (view, cx) = cx.add_window_view(|_, _| PairView {
                target: 0.,
                policy,
                state: TransitionState::default(),
                samples: samples.clone(),
            });
            for (millis, target) in [(0, 1.), (40, 1.), (0, 0.), (20, 0.), (0, 1.), (100, 1.)] {
                cx.executor().advance_clock(Duration::from_millis(millis));
                cx.update(|window, cx| {
                    view.update(cx, |view, _| view.target = target);
                    window.refresh();
                    window.draw(cx).clear();
                });
                let (expected, actual) = *samples.borrow().last().unwrap();
                assert_eq!(actual.status, expected.status);
                assert!((actual.value - expected.value).abs() < 0.00001);
            }
        }
    }

    struct PersistentTransitionView {
        state: TransitionState<f32>,
        policy: Transition,
        mounted: bool,
        target: f32,
        sampled: Rc<Cell<Option<MotionValue<f32>>>>,
    }

    impl Render for PersistentTransitionView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            if self.mounted {
                self.sampled.set(Some(self.state.sample(
                    self.target,
                    self.policy.clone(),
                    window,
                    cx,
                )));
            }
            Empty
        }
    }

    struct Persistent {
        view: gpui::Entity<PersistentTransitionView>,
        sampled: Rc<Cell<Option<MotionValue<f32>>>>,
    }

    impl Persistent {
        fn open(cx: &mut TestAppContext, policy: Transition) -> (Self, &mut VisualTestContext) {
            let sampled = Rc::new(Cell::new(None));
            let (view, cx) = cx.add_window_view(|_, _| PersistentTransitionView {
                state: TransitionState::default(),
                policy,
                mounted: true,
                target: 0.,
                sampled: sampled.clone(),
            });
            (Self { view, sampled }, cx)
        }

        /// Draws once with `update` applied, returning the sample (if mounted)
        /// and how many animation frames that draw requested.
        fn draw(
            &self,
            cx: &mut VisualTestContext,
            update: impl FnOnce(&mut PersistentTransitionView),
        ) -> (Option<MotionValue<f32>>, usize) {
            self.sampled.set(None);
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
                self.view.update(cx, |view, _| update(view));
                window.refresh();
                window.draw(cx).clear();
                (self.sampled.get(), window.simulate_next_frame(cx))
            })
        }
    }

    #[gpui::test]
    fn transition_state_requests_frames_only_while_delayed_or_running(cx: &mut TestAppContext) {
        let (fixture, cx) = Persistent::open(
            cx,
            Transition::new(Duration::from_millis(100)).delay(Duration::from_millis(20)),
        );
        let (adopted, frames) = fixture.draw(cx, |_| {});
        assert_eq!(adopted.unwrap().status, MotionStatus::Idle);
        assert_eq!(frames, 0, "the first sample adopts its target");

        let (delayed, frames) = fixture.draw(cx, |view| view.target = 1.);
        assert_eq!(delayed.unwrap().status, MotionStatus::Delayed);
        assert!(frames > 0, "a delayed transition requests the next frame");

        cx.executor().advance_clock(Duration::from_millis(70));
        let (running, frames) = fixture.draw(cx, |_| {});
        assert_eq!(running.unwrap().status, MotionStatus::Running);
        assert!(frames > 0, "a running transition requests the next frame");

        cx.executor().advance_clock(Duration::from_millis(50));
        let (finished, frames) = fixture.draw(cx, |_| {});
        assert_eq!(finished.unwrap().status, MotionStatus::Finished);
        assert_eq!(frames, 0, "a settled transition requests no frame");
    }

    #[gpui::test]
    fn transition_state_survives_unmount_and_reverses_on_remount(cx: &mut TestAppContext) {
        let (fixture, cx) = Persistent::open(
            cx,
            Transition::new(Duration::from_millis(100)).easing(Easing::Linear),
        );
        fixture.draw(cx, |view| view.target = 1.);
        cx.executor().advance_clock(Duration::from_millis(40));
        let (sample, _) = fixture.draw(cx, |_| {});
        assert!((sample.unwrap().value - 0.4).abs() < 0.00001);

        let (sample, frames) = fixture.draw(cx, |view| view.mounted = false);
        assert_eq!(sample, None);
        assert_eq!(frames, 0, "an unmounted state requests no frames");

        // The target changes while the element is not rendered; the clock keeps
        // the forward transition, and the reversal starts at the next sample.
        cx.executor().advance_clock(Duration::from_millis(30));
        fixture.draw(cx, |view| view.target = 0.);
        let (sample, frames) = fixture.draw(cx, |view| view.mounted = true);
        let sample = sample.unwrap();
        assert_eq!(sample.status, MotionStatus::Running);
        assert!((sample.value - 0.7).abs() < 0.00001);
        assert!(frames > 0);

        cx.executor().advance_clock(Duration::from_millis(35));
        let (sample, _) = fixture.draw(cx, |_| {});
        assert!(
            (sample.unwrap().value - 0.35).abs() < 0.00001,
            "the 70ms shortened reversal continues after remount"
        );
        cx.executor().advance_clock(Duration::from_millis(35));
        let (sample, frames) = fixture.draw(cx, |_| {});
        assert_eq!(
            sample.unwrap(),
            MotionValue {
                value: 0.,
                status: MotionStatus::Finished
            }
        );
        assert_eq!(frames, 0);
    }

    struct KeyframeView {
        track: Keyframes<f32>,
        timing: Timing,
        samples: Rc<RefCell<Vec<MotionValue<f32>>>>,
    }

    impl Render for KeyframeView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            self.samples.borrow_mut().push(animate_keyframes(
                "keyframe-test",
                &self.track,
                self.timing.clone(),
                window,
                cx,
            ));
            Empty
        }
    }

    #[gpui::test]
    fn keyed_keyframes_follow_timing_and_stop_after_completion(cx: &mut TestAppContext) {
        let samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let samples = samples.clone();
            move |_, _| KeyframeView {
                track: Keyframes::try_new([Keyframe::new(0.0, 0.0), Keyframe::new(1.0, 10.0)])
                    .unwrap(),
                timing: Timing::new(Duration::from_millis(100)),
                samples,
            }
        });
        cx.run_until_parked();
        assert_eq!(samples.borrow().last().unwrap().value, 0.0);
        assert_eq!(
            samples.borrow().last().unwrap().status,
            MotionStatus::Running
        );

        cx.executor().advance_clock(Duration::from_millis(50));
        assert_eq!(
            window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            1
        );
        cx.run_until_parked();
        assert_eq!(samples.borrow().last().unwrap().value, 5.0);

        cx.executor().advance_clock(Duration::from_millis(50));
        window.update(cx, |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();
        assert_eq!(
            samples.borrow().last().unwrap().status,
            MotionStatus::Finished
        );
        window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            0
        );
    }

    struct PresenceView {
        present: Rc<Cell<bool>>,
        samples: Rc<RefCell<Vec<PresenceSample>>>,
    }

    impl Render for PresenceView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            self.samples.borrow_mut().push(
                Presence::new("presence-test", self.present.get())
                    .transition(Transition::new(Duration::from_millis(100)).ease(|t| t))
                    .sample(window, cx),
            );
            Empty
        }
    }

    struct PresenceFixture {
        window: WindowHandle<PresenceView>,
        present: Rc<Cell<bool>>,
        samples: Rc<RefCell<Vec<PresenceSample>>>,
    }

    impl PresenceFixture {
        fn open(cx: &mut TestAppContext, initially_present: bool) -> Self {
            let present = Rc::new(Cell::new(initially_present));
            let samples = Rc::new(RefCell::new(Vec::new()));
            let window = cx.add_window({
                let present = present.clone();
                let samples = samples.clone();
                move |_, _| PresenceView { present, samples }
            });
            cx.run_until_parked();
            Self {
                window,
                present,
                samples,
            }
        }

        fn render(&self, cx: &mut TestAppContext, present: bool) -> PresenceSample {
            self.present.set(present);
            self.window
                .update(cx, |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            *self.samples.borrow().last().unwrap()
        }
    }

    #[gpui::test]
    fn presence_enters_exits_and_only_unmounts_after_exit(cx: &mut TestAppContext) {
        let fixture = PresenceFixture::open(cx, true);
        let entering = *fixture.samples.borrow().last().unwrap();
        assert_eq!(entering.phase, PresencePhase::Entering);
        assert_eq!(entering.progress, 0.0);
        assert!(entering.should_render());

        cx.executor().advance_clock(Duration::from_millis(100));
        let present = fixture.render(cx, true);
        assert_eq!(present.phase, PresencePhase::Present);
        assert_eq!(present.progress, 1.0);

        let exiting = fixture.render(cx, false);
        assert_eq!(exiting.phase, PresencePhase::Exiting);
        assert_eq!(exiting.progress, 1.0);
        assert!(exiting.should_render());

        cx.executor().advance_clock(Duration::from_millis(100));
        let absent = fixture.render(cx, false);
        assert_eq!(absent.phase, PresencePhase::Absent);
        assert_eq!(absent.progress, 0.0);
        assert!(!absent.should_render());
    }

    #[gpui::test]
    fn presence_reentry_reverses_from_the_exit_sample(cx: &mut TestAppContext) {
        let fixture = PresenceFixture::open(cx, true);
        cx.executor().advance_clock(Duration::from_millis(100));
        fixture.render(cx, true);
        fixture.render(cx, false);
        cx.executor().advance_clock(Duration::from_millis(40));
        let reentering = fixture.render(cx, true);

        assert_eq!(reentering.phase, PresencePhase::Entering);
        assert_eq!(reentering.progress, 0.6);
    }

    #[gpui::test]
    fn reduced_motion_resolves_presence_without_a_pending_frame(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let fixture = PresenceFixture::open(cx, true);
        assert_eq!(
            fixture.samples.borrow().last().unwrap().phase,
            PresencePhase::Present
        );
        assert_eq!(
            fixture
                .window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            0
        );
        assert_eq!(fixture.render(cx, false).phase, PresencePhase::Absent);
    }

    struct SequenceView {
        steps: Rc<RefCell<Vec<SequenceStep<f32>>>>,
        samples: Rc<RefCell<Vec<SequenceSample<f32>>>>,
    }

    impl Render for SequenceView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            self.samples.borrow_mut().push(
                Sequence::new("sequence-test", 0.0)
                    .with_steps(self.steps.borrow().iter().cloned())
                    .sample(window, cx),
            );
            Empty
        }
    }

    struct SequenceFixture {
        window: WindowHandle<SequenceView>,
        steps: Rc<RefCell<Vec<SequenceStep<f32>>>>,
        samples: Rc<RefCell<Vec<SequenceSample<f32>>>>,
    }

    impl SequenceFixture {
        fn open(cx: &mut TestAppContext, steps: Vec<SequenceStep<f32>>) -> Self {
            let steps = Rc::new(RefCell::new(steps));
            let samples = Rc::new(RefCell::new(Vec::new()));
            let window = cx.add_window({
                let steps = steps.clone();
                let samples = samples.clone();
                move |_, _| SequenceView { steps, samples }
            });
            cx.run_until_parked();
            Self {
                window,
                steps,
                samples,
            }
        }

        fn linear(millis: u64) -> Transition {
            Transition::new(Duration::from_millis(millis)).ease(|t| t)
        }

        fn last(&self) -> SequenceSample<f32> {
            *self.samples.borrow().last().unwrap()
        }

        fn render(&self, cx: &mut TestAppContext) -> SequenceSample<f32> {
            self.window
                .update(cx, |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            self.last()
        }

        fn advance(&self, cx: &mut TestAppContext, millis: u64) -> SequenceSample<f32> {
            cx.executor().advance_clock(Duration::from_millis(millis));
            self.render(cx)
        }

        fn pending_frame(&self, cx: &mut TestAppContext) -> usize {
            self.window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap()
        }
    }

    fn assert_sample(sample: SequenceSample<f32>, value: f32, step: usize, status: MotionStatus) {
        assert_eq!(
            (*sample.value(), sample.step(), sample.status()),
            (value, step, status)
        );
    }

    #[gpui::test]
    fn sequence_steps_advance_in_order_at_their_boundaries(cx: &mut TestAppContext) {
        let fixture = SequenceFixture::open(
            cx,
            vec![
                SequenceStep::new(10.0, SequenceFixture::linear(100)),
                SequenceStep::new(
                    20.0,
                    SequenceFixture::linear(100).delay(Duration::from_millis(50)),
                ),
            ],
        );
        assert_sample(fixture.last(), 0.0, 0, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 50), 5.0, 0, MotionStatus::Running);

        // The first step ends and the second begins within one frame; nothing
        // reports the first step as finished in between.
        assert_sample(fixture.advance(cx, 50), 10.0, 1, MotionStatus::Delayed);
        assert_sample(fixture.advance(cx, 50), 10.0, 1, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 50), 15.0, 1, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 50), 20.0, 1, MotionStatus::Finished);
    }

    #[gpui::test]
    fn a_sequence_starts_its_next_step_where_the_previous_ended_not_at_the_frame(
        cx: &mut TestAppContext,
    ) {
        let fixture = SequenceFixture::open(
            cx,
            vec![
                SequenceStep::new(10.0, SequenceFixture::linear(100)),
                SequenceStep::new(20.0, SequenceFixture::linear(100)),
            ],
        );
        // One frame at 50 ms, the next at 175 ms: the boundary at 100 ms fell
        // between them, and the second step is sampled as if it started there.
        fixture.advance(cx, 50);
        assert_sample(fixture.advance(cx, 125), 17.5, 1, MotionStatus::Running);
    }

    #[gpui::test]
    fn zero_duration_steps_complete_within_the_frame_that_reaches_them(cx: &mut TestAppContext) {
        let fixture = SequenceFixture::open(
            cx,
            vec![
                SequenceStep::new(5.0, SequenceFixture::linear(0)),
                SequenceStep::new(6.0, SequenceFixture::linear(0)),
                SequenceStep::new(10.0, SequenceFixture::linear(100)),
            ],
        );
        assert_sample(fixture.last(), 6.0, 2, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 50), 8.0, 2, MotionStatus::Running);
    }

    #[gpui::test]
    fn a_sequence_reports_finished_once_and_then_stops_requesting_frames(cx: &mut TestAppContext) {
        let fixture = SequenceFixture::open(
            cx,
            vec![
                SequenceStep::new(10.0, SequenceFixture::linear(100)),
                SequenceStep::new(20.0, SequenceFixture::linear(100)),
            ],
        );
        assert_eq!(fixture.pending_frame(cx), 1);
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_millis(100));
        assert_eq!(fixture.pending_frame(cx), 1);
        cx.run_until_parked();
        assert_eq!(fixture.last().status(), MotionStatus::Running);
        assert!(
            fixture
                .samples
                .borrow()
                .iter()
                .all(|sample| !sample.is_finished()),
            "the first step's end must not read as the sequence finishing"
        );

        assert_sample(fixture.advance(cx, 100), 20.0, 1, MotionStatus::Finished);
        fixture.pending_frame(cx);
        cx.run_until_parked();
        assert_eq!(fixture.pending_frame(cx), 0);
        assert_sample(fixture.advance(cx, 1_000), 20.0, 1, MotionStatus::Finished);
    }

    #[gpui::test]
    fn reduced_motion_adopts_a_sequences_last_target_without_requesting_a_frame(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let fixture = SequenceFixture::open(
            cx,
            vec![
                SequenceStep::new(10.0, SequenceFixture::linear(100)),
                SequenceStep::new(20.0, SequenceFixture::linear(100)),
            ],
        );
        assert_sample(fixture.last(), 20.0, 1, MotionStatus::Finished);
        assert_eq!(fixture.pending_frame(cx), 0);
    }

    #[gpui::test]
    fn a_changed_step_target_restarts_the_sequence_from_the_sampled_value(cx: &mut TestAppContext) {
        let fixture = SequenceFixture::open(
            cx,
            vec![
                SequenceStep::new(10.0, SequenceFixture::linear(100)),
                SequenceStep::new(20.0, SequenceFixture::linear(100)),
            ],
        );
        fixture.advance(cx, 150);
        assert_sample(fixture.last(), 15.0, 1, MotionStatus::Running);

        *fixture.steps.borrow_mut() = vec![
            SequenceStep::new(10.0, SequenceFixture::linear(100)),
            SequenceStep::new(35.0, SequenceFixture::linear(100)),
        ];
        assert_sample(fixture.render(cx), 15.0, 0, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 50), 12.5, 0, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 100), 22.5, 1, MotionStatus::Running);

        // Fewer steps than the one being played is a different sequence too.
        *fixture.steps.borrow_mut() = vec![SequenceStep::new(0.0, SequenceFixture::linear(100))];
        assert_sample(fixture.render(cx), 22.5, 0, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 100), 0.0, 0, MotionStatus::Finished);
    }

    #[gpui::test]
    fn a_change_to_a_step_not_being_played_does_not_restart_the_sequence(cx: &mut TestAppContext) {
        let fixture = SequenceFixture::open(
            cx,
            vec![
                SequenceStep::new(10.0, SequenceFixture::linear(100)),
                SequenceStep::new(20.0, SequenceFixture::linear(100)),
            ],
        );
        fixture.advance(cx, 50);
        *fixture.steps.borrow_mut() = vec![
            SequenceStep::new(10.0, SequenceFixture::linear(100)),
            SequenceStep::new(30.0, SequenceFixture::linear(100)),
        ];
        assert_sample(fixture.render(cx), 5.0, 0, MotionStatus::Running);
        assert_sample(fixture.advance(cx, 100), 20.0, 1, MotionStatus::Running);
    }

    struct SingleStepView {
        armed: Rc<Cell<bool>>,
        samples: Rc<RefCell<Vec<(SequenceSample<f32>, MotionValue<f32>)>>>,
    }

    impl Render for SingleStepView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let policy = Transition::new(Duration::from_millis(100))
                .delay(Duration::from_millis(20))
                .easing(Easing::EaseInOut);
            // A plain transition adopts its first target where a sequence
            // plays from `from` at once, so the transition is primed at 0.0
            // and both leave for 10.0 on the frame that arms the view.
            if !self.armed.get() {
                transition_with_status("plain", 0.0, policy, window, cx);
                return Empty;
            }
            let sequence = Sequence::new("single-step", 0.0)
                .with_step(10.0, policy.clone())
                .sample(window, cx);
            let plain = transition_with_status("plain", 10.0, policy, window, cx);
            self.samples.borrow_mut().push((sequence, plain));
            Empty
        }
    }

    #[gpui::test]
    fn a_single_step_sequence_matches_a_plain_transition(cx: &mut TestAppContext) {
        let armed = Rc::new(Cell::new(false));
        let samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let armed = armed.clone();
            let samples = samples.clone();
            move |_, _| SingleStepView { armed, samples }
        });
        cx.run_until_parked();
        armed.set(true);
        for millis in [0, 10, 30, 70, 120] {
            cx.executor().advance_clock(Duration::from_millis(millis));
            window.update(cx, |_, window, _| window.refresh()).unwrap();
            cx.run_until_parked();
            let (sequence, plain) = *samples.borrow().last().unwrap();
            assert_eq!(*sequence.value(), plain.value, "value after {millis} ms");
            assert_eq!(sequence.status(), plain.status, "status after {millis} ms");
            assert_eq!(sequence.step(), 0);
        }
    }

    #[gpui::test]
    fn an_empty_sequence_idles_at_its_starting_value(cx: &mut TestAppContext) {
        let fixture = SequenceFixture::open(cx, Vec::new());
        assert_sample(fixture.last(), 0.0, 0, MotionStatus::Idle);
        assert_eq!(fixture.pending_frame(cx), 0);
    }

    #[test]
    fn transition_ids_accept_element_like_scalars_and_named_channels() {
        assert_eq!(
            TransitionId::from("opacity"),
            TransitionId::from(ElementId::from("opacity"))
        );
        assert_ne!(
            TransitionId::from(("terms", "fill")),
            TransitionId::from(("terms", "mark-opacity"))
        );
        let _: TransitionId = 7usize.into();
        let _: TransitionId = 7i32.into();
    }

    struct TestView {
        target: Rc<Cell<f32>>,
        duration: Duration,
        samples: Rc<RefCell<Vec<f32>>>,
    }

    impl Render for TestView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            self.samples.borrow_mut().push(transition(
                ("test", "value"),
                self.target.get(),
                Transition::new(self.duration).ease(|t| t),
                window,
                cx,
            ));
            Empty
        }
    }

    struct DelayedView {
        target: Rc<Cell<f32>>,
        samples: Rc<RefCell<Vec<f32>>>,
    }

    impl Render for DelayedView {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            self.samples.borrow_mut().push(transition(
                ("delayed-test", "value"),
                self.target.get(),
                Transition::new(Duration::from_millis(100))
                    .delay(Duration::from_millis(50))
                    .ease(|t| t),
                window,
                cx,
            ));
            Empty
        }
    }

    struct Fixture {
        window: WindowHandle<TestView>,
        target: Rc<Cell<f32>>,
        samples: Rc<RefCell<Vec<f32>>>,
    }

    impl Fixture {
        fn open(cx: &mut TestAppContext, duration: Duration) -> Self {
            let target = Rc::new(Cell::new(0.0));
            let samples = Rc::new(RefCell::new(Vec::new()));
            let window = cx.add_window({
                let target = target.clone();
                let samples = samples.clone();
                move |_, _| TestView {
                    target,
                    duration,
                    samples,
                }
            });
            cx.run_until_parked();
            Self {
                window,
                target,
                samples,
            }
        }

        fn render(&self, cx: &mut TestAppContext, target: f32) -> f32 {
            self.target.set(target);
            self.window
                .update(cx, |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            *self.samples.borrow().last().unwrap()
        }

        fn pending_frame(&self, cx: &mut TestAppContext) -> usize {
            self.window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap()
        }
    }

    #[gpui::test]
    fn a_zero_duration_target_change_is_immediate(cx: &mut TestAppContext) {
        let fixture = Fixture::open(cx, Duration::ZERO);
        assert_eq!(fixture.render(cx, 1.0), 1.0);
    }

    #[gpui::test]
    fn a_changed_target_transitions_over_time(cx: &mut TestAppContext) {
        let duration = Duration::from_millis(100);
        let fixture = Fixture::open(cx, duration);
        assert_eq!(fixture.render(cx, 10.0), 0.0);

        cx.executor().advance_clock(Duration::from_millis(50));
        assert_eq!(fixture.render(cx, 10.0), 5.0);
    }

    #[gpui::test]
    fn requested_animation_frames_resample_without_manual_refresh(cx: &mut TestAppContext) {
        let duration = Duration::from_millis(100);
        let fixture = Fixture::open(cx, duration);
        assert_eq!(fixture.render(cx, 10.0), 0.0);

        cx.executor().advance_clock(Duration::from_millis(50));
        assert_eq!(fixture.pending_frame(cx), 1);
        cx.run_until_parked();

        assert_eq!(*fixture.samples.borrow().last().unwrap(), 5.0);
    }

    #[gpui::test]
    fn reversing_uses_the_current_sample_and_shortens_the_return(cx: &mut TestAppContext) {
        let duration = Duration::from_millis(100);
        let fixture = Fixture::open(cx, duration);
        assert_eq!(fixture.render(cx, 10.0), 0.0);

        cx.executor().advance_clock(Duration::from_millis(50));
        assert_eq!(fixture.render(cx, 0.0), 5.0);
        cx.executor().advance_clock(Duration::from_millis(25));
        assert_eq!(fixture.render(cx, 0.0), 2.5);
    }

    #[gpui::test]
    fn delay_holds_the_previous_value_before_interpolation(cx: &mut TestAppContext) {
        let target = Rc::new(Cell::new(0.0));
        let samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let target = target.clone();
            let samples = samples.clone();
            move |_, _| DelayedView { target, samples }
        });
        cx.run_until_parked();

        target.set(10.0);
        window.update(cx, |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();
        assert_eq!(*samples.borrow().last().unwrap(), 0.0);

        cx.executor().advance_clock(Duration::from_millis(50));
        window.update(cx, |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();
        assert_eq!(*samples.borrow().last().unwrap(), 0.0);

        cx.executor().advance_clock(Duration::from_millis(50));
        window.update(cx, |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();
        assert_eq!(*samples.borrow().last().unwrap(), 5.0);
    }

    #[gpui::test]
    fn a_completed_transition_stops_requesting_frames(cx: &mut TestAppContext) {
        let duration = Duration::from_millis(100);
        let fixture = Fixture::open(cx, duration);
        fixture.render(cx, 1.0);
        assert_eq!(fixture.pending_frame(cx), 1);

        cx.executor().advance_clock(duration);
        assert_eq!(fixture.render(cx, 1.0), 1.0);
        fixture.pending_frame(cx);
        cx.run_until_parked();
        assert_eq!(fixture.pending_frame(cx), 0);
    }

    #[gpui::test]
    fn reduced_motion_adopts_the_target_without_requesting_a_frame(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let duration = Duration::from_millis(100);
        let fixture = Fixture::open(cx, duration);
        assert_eq!(fixture.render(cx, 1.0), 1.0);
        assert_eq!(fixture.pending_frame(cx), 0);
    }
}
