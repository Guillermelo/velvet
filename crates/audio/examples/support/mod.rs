//! Standalone editor examples own their Windows message loop (eframe owns the app's loop).
pub fn pump_messages() {
    #[cfg(windows)]
    unsafe {
        use std::ffi::c_void;
        #[repr(C)]
        struct Message {
            hwnd: *mut c_void,
            message: u32,
            wparam: usize,
            lparam: isize,
            time: u32,
            point: [i32; 2],
            private: u32,
        }
        #[link(name = "user32")]
        extern "system" {
            fn PeekMessageW(
                message: *mut Message,
                hwnd: *mut c_void,
                min: u32,
                max: u32,
                remove: u32,
            ) -> i32;
            fn TranslateMessage(message: *const Message) -> i32;
            fn DispatchMessageW(message: *const Message) -> isize;
        }
        let mut message: Message = std::mem::zeroed();
        // Bound each poll so an editor continuously posting messages cannot starve DSP capture.
        for _ in 0..256 {
            if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, 1) == 0 {
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
