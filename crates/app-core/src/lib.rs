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

#[derive(Debug, Eq, PartialEq)]
pub struct WindowState {
    mode: WindowMode,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            mode: WindowMode::Collapsed,
        }
    }
}

impl WindowState {
    pub const fn mode(&self) -> WindowMode {
        self.mode
    }

    pub fn toggle(&mut self) -> WindowLayout {
        self.mode = self.mode.toggled();
        self.mode.layout()
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowLayout, WindowMode, WindowState};

    #[test]
    fn starts_collapsed_at_the_mascot_size() {
        let state = WindowState::default();

        assert_eq!(state.mode(), WindowMode::Collapsed);
        assert_eq!(
            state.mode().layout(),
            WindowLayout {
                width: 200,
                height: 200,
            }
        );
    }

    #[test]
    fn first_toggle_expands_to_the_stable_panel_size() {
        let mut state = WindowState::default();

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
        state.toggle();

        let layout = state.toggle();

        assert_eq!(state.mode(), WindowMode::Collapsed);
        assert_eq!(layout, WindowMode::COLLAPSED_LAYOUT);
    }
}
