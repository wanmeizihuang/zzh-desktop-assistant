use std::{io, mem::size_of};

use app_core::{PhysicalPosition, PhysicalSize, ScreenBounds};
use slint::winit_030::winit::{
    self,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
};
use windows::{
    Win32::{
        Foundation::{ERROR_SUCCESS, GetLastError, HWND, LPARAM, RECT, SetLastError},
        Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_EX_APPWINDOW,
            WS_EX_TOOLWINDOW,
        },
    },
    core::BOOL,
};

pub fn set_window_position(
    window: &winit::window::Window,
    position: PhysicalPosition,
) -> io::Result<()> {
    let hwnd = native_window_handle(window)?;

    unsafe {
        SetWindowPos(
            hwnd,
            None,
            position.x,
            position.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .map_err(|error| io::Error::other(error.to_string()))
}

pub fn enforce_tool_window(window: &winit::window::Window) -> io::Result<()> {
    let hwnd = native_window_handle(window)?;
    let current_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let tool_style = (current_style & !WS_EX_APPWINDOW.0) | WS_EX_TOOLWINDOW.0;

    unsafe { SetLastError(ERROR_SUCCESS) };
    let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, tool_style as isize) };
    if previous == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(error.0 as i32));
        }
    }

    unsafe {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    }
    .map_err(|error| io::Error::other(error.to_string()))
}

fn native_window_handle(window: &winit::window::Window) -> io::Result<HWND> {
    let window_handle = window
        .window_handle()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let RawWindowHandle::Win32(window_handle) = window_handle.as_raw() else {
        return Err(io::Error::other("expected a Win32 window handle"));
    };
    Ok(HWND(window_handle.hwnd.get() as *mut core::ffi::c_void))
}

pub fn work_areas() -> io::Result<Vec<ScreenBounds>> {
    let mut state = EnumerationState::default();
    let result = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_work_area),
            LPARAM((&raw mut state).cast::<()>() as isize),
        )
    };
    if !result.as_bool() {
        return Err(io::Error::last_os_error());
    }
    if state.failed {
        return Err(io::Error::last_os_error());
    }
    if state.work_areas.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Windows did not report any monitor work areas",
        ));
    }

    Ok(state.work_areas)
}

#[derive(Default)]
struct EnumerationState {
    work_areas: Vec<ScreenBounds>,
    failed: bool,
}

unsafe extern "system" fn collect_work_area(
    monitor: HMONITOR,
    _device_context: HDC,
    _monitor_rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let state = unsafe { &mut *(data.0 as *mut EnumerationState) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        state.failed = true;
        return true.into();
    }

    if let Some(bounds) = bounds_from_rect(info.rcWork) {
        state.work_areas.push(bounds);
    }
    true.into()
}

fn bounds_from_rect(rect: RECT) -> Option<ScreenBounds> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    if width <= 0 || height <= 0 {
        return None;
    }

    Some(ScreenBounds {
        position: PhysicalPosition {
            x: rect.left,
            y: rect.top,
        },
        size: PhysicalSize {
            width: width as u32,
            height: height as u32,
        },
    })
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::RECT;

    use super::{bounds_from_rect, work_areas};

    #[test]
    fn windows_reports_at_least_one_valid_work_area() {
        let work_areas = work_areas().expect("enumerate monitor work areas");

        assert!(!work_areas.is_empty());
        assert!(
            work_areas
                .iter()
                .all(|area| area.size.width > 0 && area.size.height > 0)
        );
    }

    #[test]
    fn rect_conversion_preserves_negative_monitor_coordinates() {
        let bounds = bounds_from_rect(RECT {
            left: -1920,
            top: -120,
            right: 0,
            bottom: 960,
        })
        .expect("valid work area");

        assert_eq!(bounds.position.x, -1920);
        assert_eq!(bounds.position.y, -120);
        assert_eq!(bounds.size.width, 1920);
        assert_eq!(bounds.size.height, 1080);
    }

    #[test]
    fn invalid_rect_is_ignored() {
        assert!(
            bounds_from_rect(RECT {
                left: 100,
                top: 0,
                right: 100,
                bottom: 800,
            })
            .is_none()
        );
    }
}
