use std::{
    io,
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject},
    },
    core::PCWSTR,
};

const MUTEX_NAME: &str = "Local\\XiaoxiDesktopAssistant.Instance.v1";
const WAKE_EVENT_NAME: &str = "Local\\XiaoxiDesktopAssistant.Wake.v1";

pub enum AcquireResult {
    Primary(PrimaryInstance),
    SecondaryNotified,
}

pub struct PrimaryInstance {
    mutex: KernelHandle,
    wake_event: KernelHandle,
}

impl PrimaryInstance {
    pub fn start_wake_listener<F>(self, on_wake: F) -> io::Result<SingleInstanceRuntime>
    where
        F: Fn() + Send + 'static,
    {
        let (stop_sender, stop_receiver) = mpsc::channel();
        let wake_handle = self.wake_event.raw();
        let thread = thread::Builder::new()
            .name("single-instance-wake".into())
            .spawn(move || {
                let wake_event = self.wake_event;
                loop {
                    let result = unsafe { WaitForSingleObject(wake_event.raw(), INFINITE) };
                    if result != WAIT_OBJECT_0 || stop_receiver.try_recv().is_ok() {
                        break;
                    }
                    on_wake();
                }
                drop(self.mutex);
            })?;

        Ok(SingleInstanceRuntime {
            stop_sender,
            wake_handle,
            thread: Some(thread),
        })
    }
}

pub struct SingleInstanceRuntime {
    stop_sender: Sender<()>,
    wake_handle: HANDLE,
    thread: Option<JoinHandle<()>>,
}

impl Drop for SingleInstanceRuntime {
    fn drop(&mut self) {
        let _ = self.stop_sender.send(());
        let _ = unsafe { SetEvent(self.wake_handle) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn acquire() -> io::Result<AcquireResult> {
    acquire_named(MUTEX_NAME, WAKE_EVENT_NAME)
}

fn acquire_named(mutex_name: &str, wake_event_name: &str) -> io::Result<AcquireResult> {
    let wake_event_name = wide_string(wake_event_name);
    let wake_event = unsafe {
        CreateEventW(None, false, false, PCWSTR(wake_event_name.as_ptr()))
            .map(KernelHandle)
            .map_err(io::Error::other)?
    };

    let mutex_name = wide_string(mutex_name);
    let mutex = unsafe {
        CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()))
            .map(KernelHandle)
            .map_err(io::Error::other)?
    };
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

    if already_exists {
        unsafe { SetEvent(wake_event.raw()) }.map_err(io::Error::other)?;
        return Ok(AcquireResult::SecondaryNotified);
    }

    Ok(AcquireResult::Primary(PrimaryInstance {
        mutex,
        wake_event,
    }))
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct KernelHandle(HANDLE);

impl KernelHandle {
    const fn raw(&self) -> HANDLE {
        self.0
    }
}

unsafe impl Send for KernelHandle {}

impl Drop for KernelHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::{AcquireResult, acquire_named};

    #[test]
    fn second_acquisition_notifies_the_primary_instance() {
        let suffix = format!("{}.{}", std::process::id(), line!());
        let mutex_name = format!("Local\\XiaoxiDesktopAssistant.Test.Mutex.{suffix}");
        let event_name = format!("Local\\XiaoxiDesktopAssistant.Test.Event.{suffix}");
        let AcquireResult::Primary(primary) =
            acquire_named(&mutex_name, &event_name).expect("acquire primary instance")
        else {
            panic!("first acquisition must become the primary instance");
        };
        let (notified_sender, notified_receiver) = mpsc::channel();
        let runtime = primary
            .start_wake_listener(move || {
                let _ = notified_sender.send(());
            })
            .expect("start wake listener");

        assert!(matches!(
            acquire_named(&mutex_name, &event_name).expect("acquire secondary instance"),
            AcquireResult::SecondaryNotified
        ));
        notified_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("primary receives the wake notification");

        drop(runtime);
    }
}
