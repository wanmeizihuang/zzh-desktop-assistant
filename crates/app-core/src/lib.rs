pub mod config;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowMode {
    Collapsed,
    Expanded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowLayout {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerPosition {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragDecision {
    None,
    BeginDrag,
}

#[derive(Debug)]
pub struct DragGesture {
    press_position: Option<PointerPosition>,
    drag_started: bool,
    threshold: f32,
}

impl DragGesture {
    pub const DEFAULT_THRESHOLD: f32 = 5.0;

    pub const fn new(threshold: f32) -> Self {
        Self {
            press_position: None,
            drag_started: false,
            threshold,
        }
    }

    pub fn press(&mut self, position: PointerPosition) {
        self.press_position = Some(position);
        self.drag_started = false;
    }

    pub fn move_to(&mut self, position: PointerPosition) -> DragDecision {
        let Some(press_position) = self.press_position else {
            return DragDecision::None;
        };

        if self.drag_started {
            return DragDecision::None;
        }

        let x_delta = position.x - press_position.x;
        let y_delta = position.y - press_position.y;
        if x_delta * x_delta + y_delta * y_delta < self.threshold * self.threshold {
            return DragDecision::None;
        }

        self.drag_started = true;
        DragDecision::BeginDrag
    }

    pub fn take_click(&mut self) -> bool {
        let is_click = self.press_position.is_some() && !self.drag_started;
        self.press_position = None;
        self.drag_started = false;
        is_click
    }

    pub fn cancel(&mut self) {
        self.press_position = None;
        self.drag_started = false;
    }
}

impl Default for DragGesture {
    fn default() -> Self {
        Self::new(Self::DEFAULT_THRESHOLD)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PhysicalPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenBounds {
    pub position: PhysicalPosition,
    pub size: PhysicalSize,
}

pub fn clamp_window_position(
    position: PhysicalPosition,
    window_size: PhysicalSize,
    screen: ScreenBounds,
    minimum_visible: u32,
) -> PhysicalPosition {
    let window_width = u32_to_i32(window_size.width);
    let window_height = u32_to_i32(window_size.height);
    let screen_width = u32_to_i32(screen.size.width);
    let screen_height = u32_to_i32(screen.size.height);
    let visible_x = u32_to_i32(
        minimum_visible
            .min(window_size.width)
            .min(screen.size.width),
    );
    let visible_y = u32_to_i32(
        minimum_visible
            .min(window_size.height)
            .min(screen.size.height),
    );

    let minimum_x = screen
        .position
        .x
        .saturating_sub(window_width.saturating_sub(visible_x));
    let maximum_x = screen
        .position
        .x
        .saturating_add(screen_width)
        .saturating_sub(visible_x);
    let minimum_y = screen
        .position
        .y
        .saturating_sub(window_height.saturating_sub(visible_y));
    let maximum_y = screen
        .position
        .y
        .saturating_add(screen_height)
        .saturating_sub(visible_y);

    PhysicalPosition {
        x: position.x.clamp(minimum_x, maximum_x),
        y: position.y.clamp(minimum_y, maximum_y),
    }
}

fn u32_to_i32(value: u32) -> i32 {
    value.min(i32::MAX as u32) as i32
}

impl WindowMode {
    pub const COLLAPSED_LAYOUT: WindowLayout = WindowLayout {
        width: 200,
        height: 200,
    };

    pub const EXPANDED_LAYOUT: WindowLayout = WindowLayout {
        width: 412,
        height: 640,
    };

    pub const fn layout(self) -> WindowLayout {
        match self {
            Self::Collapsed => Self::COLLAPSED_LAYOUT,
            Self::Expanded => Self::EXPANDED_LAYOUT,
        }
    }

    pub const fn is_expanded(self) -> bool {
        matches!(self, Self::Expanded)
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Collapsed => Self::Expanded,
            Self::Expanded => Self::Collapsed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationPhase {
    Starting,
    Collapsed,
    Expanded,
    Hidden,
    Exiting,
}

#[derive(Debug, Eq, PartialEq)]
pub struct WindowState {
    phase: ApplicationPhase,
    last_visible_mode: WindowMode,
    always_on_top: bool,
    position_locked: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            phase: ApplicationPhase::Starting,
            last_visible_mode: WindowMode::Collapsed,
            always_on_top: true,
            position_locked: false,
        }
    }
}

impl WindowState {
    pub const fn with_behavior(always_on_top: bool, position_locked: bool) -> Self {
        Self {
            phase: ApplicationPhase::Starting,
            last_visible_mode: WindowMode::Collapsed,
            always_on_top,
            position_locked,
        }
    }

    pub const fn phase(&self) -> ApplicationPhase {
        self.phase
    }

    pub const fn mode(&self) -> WindowMode {
        self.last_visible_mode
    }

    pub fn start(&mut self) -> WindowLayout {
        if self.phase == ApplicationPhase::Starting {
            self.phase = ApplicationPhase::Collapsed;
            self.last_visible_mode = WindowMode::Collapsed;
        }
        self.last_visible_mode.layout()
    }

    pub fn toggle(&mut self) -> WindowLayout {
        match self.phase {
            ApplicationPhase::Collapsed | ApplicationPhase::Expanded => {
                self.last_visible_mode = self.last_visible_mode.toggled();
                self.phase = match self.last_visible_mode {
                    WindowMode::Collapsed => ApplicationPhase::Collapsed,
                    WindowMode::Expanded => ApplicationPhase::Expanded,
                };
            }
            ApplicationPhase::Starting | ApplicationPhase::Hidden | ApplicationPhase::Exiting => {}
        }
        self.last_visible_mode.layout()
    }

    pub fn collapse(&mut self) -> Option<WindowLayout> {
        if self.phase != ApplicationPhase::Expanded {
            return None;
        }

        self.phase = ApplicationPhase::Collapsed;
        self.last_visible_mode = WindowMode::Collapsed;
        Some(WindowMode::COLLAPSED_LAYOUT)
    }

    pub fn hide(&mut self) -> bool {
        match self.phase {
            ApplicationPhase::Collapsed | ApplicationPhase::Expanded => {
                self.phase = ApplicationPhase::Hidden;
                true
            }
            ApplicationPhase::Starting | ApplicationPhase::Hidden | ApplicationPhase::Exiting => {
                false
            }
        }
    }

    pub fn restore(&mut self) -> Option<WindowLayout> {
        if self.phase != ApplicationPhase::Hidden {
            return None;
        }

        self.phase = match self.last_visible_mode {
            WindowMode::Collapsed => ApplicationPhase::Collapsed,
            WindowMode::Expanded => ApplicationPhase::Expanded,
        };
        Some(self.last_visible_mode.layout())
    }

    pub fn exit(&mut self) -> bool {
        if self.phase == ApplicationPhase::Exiting {
            return false;
        }

        self.phase = ApplicationPhase::Exiting;
        true
    }

    pub const fn always_on_top(&self) -> bool {
        self.always_on_top
    }

    pub fn toggle_always_on_top(&mut self) -> bool {
        self.always_on_top = !self.always_on_top;
        self.always_on_top
    }

    pub const fn position_locked(&self) -> bool {
        self.position_locked
    }

    pub fn toggle_position_locked(&mut self) -> bool {
        self.position_locked = !self.position_locked;
        self.position_locked
    }

    pub const fn can_drag(&self) -> bool {
        !self.position_locked
            && matches!(
                self.phase,
                ApplicationPhase::Collapsed | ApplicationPhase::Expanded
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApplicationPhase, DragDecision, DragGesture, PhysicalPosition, PhysicalSize,
        PointerPosition, ScreenBounds, WindowLayout, WindowMode, WindowState,
        clamp_window_position,
    };

    #[test]
    fn starts_collapsed_at_the_mascot_size() {
        let mut state = WindowState::default();

        assert_eq!(state.phase(), ApplicationPhase::Starting);
        let layout = state.start();

        assert_eq!(state.phase(), ApplicationPhase::Collapsed);
        assert_eq!(state.mode(), WindowMode::Collapsed);
        assert_eq!(
            layout,
            WindowLayout {
                width: 200,
                height: 200
            }
        );
    }

    #[test]
    fn first_toggle_expands_to_the_stable_panel_size() {
        let mut state = WindowState::default();
        state.start();

        let layout = state.toggle();

        assert_eq!(state.mode(), WindowMode::Expanded);
        assert_eq!(
            layout,
            WindowLayout {
                width: 412,
                height: 640,
            }
        );
    }

    #[test]
    fn second_toggle_returns_to_the_collapsed_size() {
        let mut state = WindowState::default();
        state.start();
        state.toggle();

        let layout = state.toggle();

        assert_eq!(state.mode(), WindowMode::Collapsed);
        assert_eq!(layout, WindowMode::COLLAPSED_LAYOUT);
    }

    #[test]
    fn collapse_only_changes_the_expanded_phase() {
        let mut state = WindowState::default();
        state.start();

        assert_eq!(state.collapse(), None);
        state.toggle();
        assert_eq!(state.collapse(), Some(WindowMode::COLLAPSED_LAYOUT));
        assert_eq!(state.phase(), ApplicationPhase::Collapsed);
    }

    #[test]
    fn hiding_and_restoring_preserves_the_last_visible_mode() {
        let mut state = WindowState::default();
        state.start();
        state.toggle();

        assert!(state.hide());
        assert_eq!(state.phase(), ApplicationPhase::Hidden);
        assert_eq!(state.restore(), Some(WindowMode::EXPANDED_LAYOUT));
        assert_eq!(state.phase(), ApplicationPhase::Expanded);
    }

    #[test]
    fn exiting_is_terminal() {
        let mut state = WindowState::default();
        state.start();

        assert!(state.exit());
        assert_eq!(state.phase(), ApplicationPhase::Exiting);
        assert!(!state.exit());
        assert!(!state.hide());
        assert_eq!(state.restore(), None);
    }

    #[test]
    fn desktop_behavior_defaults_are_work_friendly_and_toggleable() {
        let mut state = WindowState::default();

        assert!(state.always_on_top());
        assert!(!state.position_locked());
        assert!(!state.toggle_always_on_top());
        assert!(state.toggle_position_locked());
        assert!(!state.can_drag());
    }

    #[test]
    fn desktop_behavior_can_be_restored_before_startup() {
        let mut state = WindowState::with_behavior(false, true);

        state.start();

        assert!(!state.always_on_top());
        assert!(state.position_locked());
        assert!(!state.can_drag());
    }

    #[test]
    fn small_pointer_movement_remains_a_click() {
        let mut gesture = DragGesture::default();
        gesture.press(PointerPosition { x: 10.0, y: 10.0 });

        assert_eq!(
            gesture.move_to(PointerPosition { x: 13.0, y: 13.0 }),
            DragDecision::None
        );
        assert!(gesture.take_click());
    }

    #[test]
    fn crossing_the_threshold_begins_drag_once_and_suppresses_click() {
        let mut gesture = DragGesture::default();
        gesture.press(PointerPosition { x: 10.0, y: 10.0 });

        assert_eq!(
            gesture.move_to(PointerPosition { x: 15.0, y: 10.0 }),
            DragDecision::BeginDrag
        );
        assert_eq!(
            gesture.move_to(PointerPosition { x: 30.0, y: 10.0 }),
            DragDecision::None
        );
        assert!(!gesture.take_click());
    }

    #[test]
    fn a_new_press_resets_the_previous_drag() {
        let mut gesture = DragGesture::default();
        gesture.press(PointerPosition { x: 0.0, y: 0.0 });
        gesture.move_to(PointerPosition { x: 8.0, y: 0.0 });

        gesture.press(PointerPosition { x: 20.0, y: 20.0 });

        assert!(gesture.take_click());
    }

    #[test]
    fn cancelling_a_gesture_suppresses_the_pending_click() {
        let mut gesture = DragGesture::default();
        gesture.press(PointerPosition { x: 10.0, y: 10.0 });

        gesture.cancel();

        assert!(!gesture.take_click());
    }

    #[test]
    fn window_position_stays_unchanged_while_enough_is_visible() {
        let screen = ScreenBounds {
            position: PhysicalPosition { x: 0, y: 0 },
            size: PhysicalSize {
                width: 1920,
                height: 1080,
            },
        };

        assert_eq!(
            clamp_window_position(
                PhysicalPosition { x: 120, y: 80 },
                PhysicalSize {
                    width: 300,
                    height: 300,
                },
                screen,
                32,
            ),
            PhysicalPosition { x: 120, y: 80 }
        );
    }

    #[test]
    fn window_position_keeps_a_visible_strip_on_every_edge() {
        let screen = ScreenBounds {
            position: PhysicalPosition { x: 100, y: 50 },
            size: PhysicalSize {
                width: 1000,
                height: 700,
            },
        };
        let window = PhysicalSize {
            width: 300,
            height: 400,
        };

        assert_eq!(
            clamp_window_position(PhysicalPosition { x: -500, y: -500 }, window, screen, 32),
            PhysicalPosition { x: -168, y: -318 }
        );
        assert_eq!(
            clamp_window_position(PhysicalPosition { x: 2000, y: 2000 }, window, screen, 32),
            PhysicalPosition { x: 1068, y: 718 }
        );
    }

    #[test]
    fn position_clamping_handles_a_screen_smaller_than_the_visible_strip() {
        let result = clamp_window_position(
            PhysicalPosition { x: -100, y: -100 },
            PhysicalSize {
                width: 10,
                height: 10,
            },
            ScreenBounds {
                position: PhysicalPosition { x: 0, y: 0 },
                size: PhysicalSize {
                    width: 1,
                    height: 1,
                },
            },
            32,
        );

        assert_eq!(result, PhysicalPosition { x: -9, y: -9 });
    }
}
