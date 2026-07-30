#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{cell::RefCell, rc::Rc};

use app_core::WindowState;
use slint::ComponentHandle;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let state = Rc::new(RefCell::new(WindowState::default()));
    let ui_weak = ui.as_weak();

    ui.on_toggle_requested(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        let mut state = state.borrow_mut();
        state.toggle();
        ui.set_expanded(state.mode().is_expanded());
    });

    ui.run()
}
