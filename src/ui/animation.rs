//! Window animation system — ported from Go TUIOS `internal/ui/animation.go`.
//!
//! Animated transitions for window minimize, restore, and snap operations.
//! Uses cubic easing for smooth movement. The VT emulator is not resized
//! during animation — only at completion — to prevent content overflow.

use std::time::{Duration, Instant};

/// The type of animation being performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationType {
    Minimize,
    Restore,
    Snap,
}

/// An animated transition for a window.
#[derive(Debug, Clone)]
pub struct Animation {
    pub ty: AnimationType,
    pub start_time: Instant,
    pub duration: Duration,
    pub start_x: i32,
    pub start_y: i32,
    pub start_width: i32,
    pub start_height: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub end_width: i32,
    pub end_height: i32,
    pub progress: f64,
    pub complete: bool,
    pub initial_resized: bool,
}

impl Animation {
    /// Create a minimize animation.
    pub fn new_minimize(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        dock_x: i32,
        dock_y: i32,
        duration: Duration,
    ) -> Option<Self> {
        if duration.is_zero() {
            return None;
        }
        Some(Self {
            ty: AnimationType::Minimize,
            start_time: Instant::now(),
            duration,
            start_x: x,
            start_y: y,
            start_width: width,
            start_height: height,
            end_x: dock_x,
            end_y: dock_y,
            end_width: 5,
            end_height: 3,
            progress: 0.0,
            complete: false,
            initial_resized: false,
        })
    }

    /// Create a restore animation.
    pub fn new_restore(
        dock_x: i32,
        dock_y: i32,
        target_x: i32,
        target_y: i32,
        target_width: i32,
        target_height: i32,
        duration: Duration,
    ) -> Option<Self> {
        if duration.is_zero() {
            return None;
        }
        Some(Self {
            ty: AnimationType::Restore,
            start_time: Instant::now(),
            duration,
            start_x: dock_x,
            start_y: dock_y,
            start_width: 5,
            start_height: 3,
            end_x: target_x,
            end_y: target_y,
            end_width: target_width,
            end_height: target_height,
            progress: 0.0,
            complete: false,
            initial_resized: false,
        })
    }

    /// Create a snap animation.
    #[allow(clippy::too_many_arguments)]
    pub fn new_snap(
        start_x: i32,
        start_y: i32,
        start_width: i32,
        start_height: i32,
        target_x: i32,
        target_y: i32,
        target_width: i32,
        target_height: i32,
        duration: Duration,
    ) -> Option<Self> {
        if start_x == target_x
            && start_y == target_y
            && start_width == target_width
            && start_height == target_height
        {
            return None;
        }
        if duration.is_zero() {
            return None;
        }
        Some(Self {
            ty: AnimationType::Snap,
            start_time: Instant::now(),
            duration,
            start_x,
            start_y,
            start_width,
            start_height,
            end_x: target_x,
            end_y: target_y,
            end_width: target_width,
            end_height: target_height,
            progress: 0.0,
            complete: false,
            initial_resized: false,
        })
    }

    /// Update the animation progress and return the interpolated position.
    /// Returns `None` if the animation is complete, `Some((x, y, w, h))` otherwise.
    pub fn update(&mut self) -> Option<(i32, i32, i32, i32)> {
        if self.complete {
            return None;
        }

        let elapsed = self.start_time.elapsed();
        let mut progress = elapsed.as_secs_f64() / self.duration.as_secs_f64();

        if progress >= 1.0 {
            progress = 1.0;
            self.complete = true;
        }

        self.progress = ease_in_out_cubic(progress);

        let x = interpolate(self.start_x, self.end_x, self.progress);
        let y = interpolate(self.start_y, self.end_y, self.progress);
        let w = interpolate(self.start_width, self.end_width, self.progress);
        let h = interpolate(self.start_height, self.end_height, self.progress);

        if self.complete {
            return None;
        }

        Some((x, y, w, h))
    }

    /// Returns the final position for the animation.
    pub fn final_position(&self) -> (i32, i32, i32, i32) {
        (self.end_x, self.end_y, self.end_width, self.end_height)
    }
}

/// The default duration for window animations (Go's minimize/restore
/// transitions use ~300ms).
pub const DEFAULT_ANIMATION_DURATION: Duration = Duration::from_millis(300);

/// The default animation duration, respecting a zero/negative override.
pub fn animation_duration() -> Duration {
    DEFAULT_ANIMATION_DURATION
}

/// Cubic easing for smooth transitions.
pub fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let p = 2.0 * t - 2.0;
        1.0 + p * p * p / 2.0
    }
}

/// Linear interpolation between start and end values.
pub fn interpolate(start: i32, end: i32, progress: f64) -> i32 {
    start + ((end - start) as f64 * progress).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_in_out_cubic_endpoints() {
        assert!((ease_in_out_cubic(0.0)).abs() < 1e-10);
        assert!((ease_in_out_cubic(1.0) - 1.0).abs() < 1e-10);
        assert!((ease_in_out_cubic(0.5) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn interpolate_midpoint() {
        assert_eq!(interpolate(0, 100, 0.5), 50);
        assert_eq!(interpolate(10, 20, 0.0), 10);
        assert_eq!(interpolate(10, 20, 1.0), 20);
    }

    #[test]
    fn minimize_animation_creates() {
        let anim = Animation::new_minimize(0, 0, 80, 24, 10, 20, Duration::from_millis(200));
        assert!(anim.is_some());
        let anim = anim.unwrap();
        assert_eq!(anim.ty, AnimationType::Minimize);
        assert_eq!(anim.end_x, 10);
        assert_eq!(anim.end_y, 20);
    }

    #[test]
    fn zero_duration_returns_none() {
        assert!(Animation::new_minimize(0, 0, 80, 24, 10, 20, Duration::ZERO).is_none());
    }

    #[test]
    fn snap_same_position_returns_none() {
        assert!(
            Animation::new_snap(10, 10, 80, 24, 10, 10, 80, 24, Duration::from_millis(200))
                .is_none()
        );
    }

    #[test]
    fn update_completes_after_duration() {
        let mut anim =
            Animation::new_minimize(0, 0, 80, 24, 10, 20, Duration::from_millis(1)).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        let result = anim.update();
        assert!(anim.complete);
        assert!(result.is_none());
    }

    #[test]
    fn update_returns_interpolated_position() {
        let mut anim =
            Animation::new_snap(0, 0, 80, 24, 100, 0, 80, 24, Duration::from_secs(10)).unwrap();
        let result = anim.update();
        assert!(result.is_some());
        let (x, _, _, _) = result.unwrap();
        assert!((0..100).contains(&x));
    }

    #[test]
    fn final_position_returns_end() {
        let anim =
            Animation::new_restore(10, 20, 5, 5, 80, 24, Duration::from_millis(200)).unwrap();
        let (x, y, w, h) = anim.final_position();
        assert_eq!((x, y, w, h), (5, 5, 80, 24));
    }
}
