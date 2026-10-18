//! Explicit opt-in terminal. The GUI executable itself never allocates a console implicitly.
#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::sync::Mutex;
    static VISIBLE: Mutex<bool> = Mutex::new(false);
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AllocConsole() -> i32;
        fn FreeConsole() -> i32;
        fn SetConsoleTitleW(title: *const u16) -> i32;
        fn GetConsoleWindow() -> *mut c_void;
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn WriteConsoleW(handle: *mut c_void, text: *const u16, count: u32, written: *mut u32, reserved: *mut c_void) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetSystemMenu(window: *mut c_void, revert: i32) -> *mut c_void;
        fn EnableMenuItem(menu: *mut c_void, item: u32, flags: u32) -> i32;
    }
    pub fn set_visible(enabled: bool) {
        let mut visible = VISIBLE.lock().unwrap_or_else(|p| p.into_inner());
        if enabled == *visible { return; }
        unsafe {
            if enabled {
                if AllocConsole() == 0 { return; }
                let title: Vec<u16> = "unzip-rs — 右键任务日志".encode_utf16().chain(Some(0)).collect();
                SetConsoleTitleW(title.as_ptr());
                // Closing a console terminates its process; use application controls to stop.
                let menu = GetSystemMenu(GetConsoleWindow(), 0);
                if !menu.is_null() { EnableMenuItem(menu, 0xF060, 0x0001); }
                *visible = true;
            } else { FreeConsole(); *visible = false; }
        }
    }
    pub fn log(message: &str) {
        let visible = VISIBLE.lock().unwrap_or_else(|p| p.into_inner());
        if !*visible { return; }
        let text: Vec<u16> = format!("{message}\r\n").encode_utf16().collect();
        let mut written = 0;
        unsafe { WriteConsoleW(GetStdHandle(-11i32 as u32), text.as_ptr(), text.len() as u32, &mut written, std::ptr::null_mut()); }
    }
}
#[cfg(windows)]
pub use platform::{log, set_visible};
#[cfg(not(windows))]
pub fn set_visible(_: bool) {}
#[cfg(not(windows))]
pub fn log(_: &str) {}
