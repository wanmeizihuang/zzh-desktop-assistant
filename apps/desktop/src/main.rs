#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{cell::RefCell, rc::Rc};

use app_core::{
    DragDecision, DragGesture, PhysicalPosition as CorePhysicalPosition,
    PhysicalSize as CorePhysicalSize, PointerPosition, ScreenBounds, WindowState,
    clamp_window_position,
};
use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;

    let ui = AppWindow::new()?;
    let state = Rc::new(RefCell::new(WindowState::default()));
    let gesture = Rc::new(RefCell::new(DragGesture::default()));

    register_window_bounds(&ui);
    register_drag_callbacks(&ui, Rc::clone(&gesture));

    let ui_weak = ui.as_weak();
    let gesture_for_click = Rc::clone(&gesture);
    let state_for_click = Rc::clone(&state);

    ui.on_mascot_clicked(move || {
        if !gesture_for_click.borrow_mut().take_click() {
            return;
        }

        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        toggle_window(&ui, &state_for_click);
    });

    let ui_weak = ui.as_weak();

    ui.on_toggle_requested(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        toggle_window(&ui, &state);
    });

    ui.run()
}

fn register_drag_callbacks(ui: &AppWindow, gesture: Rc<RefCell<DragGesture>>) {
    let gesture_for_press = Rc::clone(&gesture);
    ui.on_drag_pointer_pressed(move |x, y| {
        gesture_for_press
            .borrow_mut()
            .press(PointerPosition { x, y });
    });

    let ui_weak = ui.as_weak();
    ui.on_drag_pointer_moved(move |x, y| {
        let decision = gesture.borrow_mut().move_to(PointerPosition { x, y });
        if decision != DragDecision::BeginDrag {
            return;
        }

        let Some(ui) = ui_weak.upgrade() else {
            return;
        };

        ui.window().with_winit_window(|window| {
            if let Err(error) = window.drag_window() {
                eprintln!("failed to start native window drag: {error}");
            }
        });
    });
}

fn register_window_bounds(ui: &AppWindow) {
    ui.window().on_winit_window_event(|slint_window, event| {
        let winit::event::WindowEvent::Moved(position) = event else {
            return EventResult::Propagate;
        };

        slint_window.with_winit_window(|window| {
            let Some(monitor) = window
                .current_monitor()
                .or_else(|| window.primary_monitor())
            else {
                return;
            };

            let monitor_position = monitor.position();
            let monitor_size = monitor.size();
            let window_size = window.outer_size();
            let clamped = clamp_window_position(
                CorePhysicalPosition {
                    x: position.x,
                    y: position.y,
                },
                CorePhysicalSize {
                    width: window_size.width,
                    height: window_size.height,
                },
                ScreenBounds {
                    position: CorePhysicalPosition {
                        x: monitor_position.x,
                        y: monitor_position.y,
                    },
                    size: CorePhysicalSize {
                        width: monitor_size.width,
                        height: monitor_size.height,
                    },
                },
                32,
            );

            if clamped.x != position.x || clamped.y != position.y {
                window.set_outer_position(winit::dpi::PhysicalPosition::new(clamped.x, clamped.y));
            }
        });

        EventResult::Propagate
    });
}

fn toggle_window(ui: &AppWindow, state: &RefCell<WindowState>) {
    let mut state = state.borrow_mut();
    state.toggle();
    ui.set_expanded(state.mode().is_expanded());
}
