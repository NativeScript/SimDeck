use std::ffi::{c_char, c_void};

#[repr(C)]
pub struct xcw_native_owned_bytes {
    pub data: *mut u8,
    pub length: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct xcw_native_shared_bytes {
    pub data: *const u8,
    pub length: usize,
    pub owner: *const c_void,
}

#[repr(C)]
pub struct xcw_native_frame {
    pub frame_sequence: u64,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub width: u32,
    pub height: u32,
    pub codec: *const c_char,
    pub description: xcw_native_shared_bytes,
    pub data: xcw_native_shared_bytes,
}

#[allow(non_camel_case_types)]
pub type xcw_native_frame_callback =
    unsafe extern "C" fn(frame: *const xcw_native_frame, user_data: *mut c_void);

unsafe extern "C" {
    pub fn xcw_native_initialize_app();
    pub fn xcw_native_run_main_loop_slice(duration_seconds: f64);

    pub fn xcw_native_list_simulators(error_message: *mut *mut c_char) -> *mut c_char;
    pub fn xcw_native_boot_simulator(udid: *const c_char, error_message: *mut *mut c_char) -> bool;
    pub fn xcw_native_shutdown_simulator(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_toggle_appearance(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_open_url(
        udid: *const c_char,
        url: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_launch_bundle(
        udid: *const c_char,
        bundle_id: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_get_chrome_profile(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> *mut c_char;
    pub fn xcw_native_render_chrome_png(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> xcw_native_owned_bytes;
    pub fn xcw_native_render_screen_mask_png(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> xcw_native_owned_bytes;
    pub fn xcw_native_screenshot_png(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> xcw_native_owned_bytes;
    pub fn xcw_native_recent_logs(
        udid: *const c_char,
        seconds: f64,
        limit: usize,
        error_message: *mut *mut c_char,
    ) -> *mut c_char;
    pub fn xcw_native_accessibility_snapshot(
        udid: *const c_char,
        has_point: bool,
        x: f64,
        y: f64,
        max_depth: usize,
        error_message: *mut *mut c_char,
    ) -> *mut c_char;
    pub fn xcw_native_send_touch(
        udid: *const c_char,
        x: f64,
        y: f64,
        phase: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_send_key(
        udid: *const c_char,
        key_code: u16,
        modifiers: u32,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_press_home(udid: *const c_char, error_message: *mut *mut c_char) -> bool;
    pub fn xcw_native_open_app_switcher(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_press_button(
        udid: *const c_char,
        button_name: *const c_char,
        duration_ms: u32,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_rotate_right(udid: *const c_char, error_message: *mut *mut c_char) -> bool;
    pub fn xcw_native_rotate_left(udid: *const c_char, error_message: *mut *mut c_char) -> bool;
    pub fn xcw_native_erase_simulator(udid: *const c_char, error_message: *mut *mut c_char)
        -> bool;
    pub fn xcw_native_install_app(
        udid: *const c_char,
        app_path: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_uninstall_app(
        udid: *const c_char,
        bundle_id: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_set_pasteboard_text(
        udid: *const c_char,
        text: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_get_pasteboard_text(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> *mut c_char;

    pub fn xcw_native_input_create(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> *mut c_void;
    pub fn xcw_native_input_destroy(handle: *mut c_void);
    pub fn xcw_native_input_display_size(
        handle: *mut c_void,
        width: *mut f64,
        height: *mut f64,
    ) -> bool;
    pub fn xcw_native_input_send_touch(
        handle: *mut c_void,
        x: f64,
        y: f64,
        phase: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_input_send_multitouch(
        handle: *mut c_void,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        phase: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_input_send_key(
        handle: *mut c_void,
        key_code: u16,
        modifiers: u32,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_input_send_key_event(
        handle: *mut c_void,
        key_code: u16,
        down: bool,
        error_message: *mut *mut c_char,
    ) -> bool;

    pub fn xcw_native_session_create(
        udid: *const c_char,
        error_message: *mut *mut c_char,
    ) -> *mut c_void;
    pub fn xcw_native_session_destroy(handle: *mut c_void);
    pub fn xcw_native_session_start(handle: *mut c_void, error_message: *mut *mut c_char) -> bool;
    pub fn xcw_native_session_request_refresh(handle: *mut c_void);
    pub fn xcw_native_session_request_keyframe(handle: *mut c_void);
    pub fn xcw_native_session_reconfigure_video_encoder(handle: *mut c_void);
    pub fn xcw_native_session_set_frame_callback(
        handle: *mut c_void,
        callback: Option<xcw_native_frame_callback>,
        user_data: *mut c_void,
    );
    pub fn xcw_native_session_send_touch(
        handle: *mut c_void,
        x: f64,
        y: f64,
        phase: *const c_char,
        error_message: *mut *mut c_char,
    ) -> bool;
    pub fn xcw_native_session_send_key(
        handle: *mut c_void,
        key_code: u16,
        modifiers: u32,
        error_message: *mut *mut c_char,
    ) -> bool;

    pub fn xcw_native_free_string(value: *mut c_char);
    pub fn xcw_native_free_bytes(bytes: xcw_native_owned_bytes);
    pub fn xcw_native_release_shared_bytes(bytes: xcw_native_shared_bytes);
}
