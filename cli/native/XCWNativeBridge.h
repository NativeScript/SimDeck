#import <Foundation/Foundation.h>

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

NS_ASSUME_NONNULL_BEGIN

typedef struct xcw_native_owned_bytes {
    uint8_t * _Nullable data;
    size_t length;
} xcw_native_owned_bytes;

typedef struct xcw_native_shared_bytes {
    const uint8_t * _Nullable data;
    size_t length;
    const void * _Nullable owner;
} xcw_native_shared_bytes;

typedef struct xcw_native_frame {
    uint64_t frame_sequence;
    uint64_t timestamp_us;
    bool is_keyframe;
    uint32_t width;
    uint32_t height;
    const char * _Nullable codec;
    xcw_native_shared_bytes description;
    xcw_native_shared_bytes data;
} xcw_native_frame;

typedef void (*xcw_native_frame_callback)(const xcw_native_frame * _Nonnull frame, void * _Nullable user_data);

void xcw_native_initialize_app(void);
void xcw_native_run_main_loop_slice(double duration_seconds);

char * _Nullable xcw_native_list_simulators(char * _Nullable * _Nullable error_message);
bool xcw_native_boot_simulator(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_shutdown_simulator(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_toggle_appearance(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_open_url(const char * _Nonnull udid, const char * _Nonnull url, char * _Nullable * _Nullable error_message);
bool xcw_native_launch_bundle(const char * _Nonnull udid, const char * _Nonnull bundle_id, char * _Nullable * _Nullable error_message);
char * _Nullable xcw_native_get_chrome_profile(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
xcw_native_owned_bytes xcw_native_render_chrome_png(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
xcw_native_owned_bytes xcw_native_render_screen_mask_png(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
xcw_native_owned_bytes xcw_native_screenshot_png(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
char * _Nullable xcw_native_recent_logs(const char * _Nonnull udid, double seconds, size_t limit, char * _Nullable * _Nullable error_message);
char * _Nullable xcw_native_accessibility_snapshot(const char * _Nonnull udid, bool has_point, double x, double y, size_t max_depth, char * _Nullable * _Nullable error_message);
bool xcw_native_send_touch(const char * _Nonnull udid, double x, double y, const char * _Nonnull phase, char * _Nullable * _Nullable error_message);
bool xcw_native_send_key(const char * _Nonnull udid, uint16_t key_code, uint32_t modifiers, char * _Nullable * _Nullable error_message);
bool xcw_native_send_key_event(const char * _Nonnull udid, uint16_t key_code, bool down, char * _Nullable * _Nullable error_message);
bool xcw_native_press_home(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_open_app_switcher(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_press_button(const char * _Nonnull udid, const char * _Nonnull button_name, uint32_t duration_ms, char * _Nullable * _Nullable error_message);
bool xcw_native_rotate_right(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_rotate_left(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_erase_simulator(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
bool xcw_native_install_app(const char * _Nonnull udid, const char * _Nonnull app_path, char * _Nullable * _Nullable error_message);
bool xcw_native_uninstall_app(const char * _Nonnull udid, const char * _Nonnull bundle_id, char * _Nullable * _Nullable error_message);
bool xcw_native_set_pasteboard_text(const char * _Nonnull udid, const char * _Nonnull text, char * _Nullable * _Nullable error_message);
char * _Nullable xcw_native_get_pasteboard_text(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);

void * _Nullable xcw_native_input_create(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
void xcw_native_input_destroy(void * _Nullable handle);
bool xcw_native_input_display_size(void * _Nonnull handle, double * _Nullable width, double * _Nullable height);
bool xcw_native_input_send_touch(void * _Nonnull handle, double x, double y, const char * _Nonnull phase, char * _Nullable * _Nullable error_message);
bool xcw_native_input_send_multitouch(void * _Nonnull handle, double x1, double y1, double x2, double y2, const char * _Nonnull phase, char * _Nullable * _Nullable error_message);
bool xcw_native_input_send_key(void * _Nonnull handle, uint16_t key_code, uint32_t modifiers, char * _Nullable * _Nullable error_message);
bool xcw_native_input_send_key_event(void * _Nonnull handle, uint16_t key_code, bool down, char * _Nullable * _Nullable error_message);

void * _Nullable xcw_native_session_create(const char * _Nonnull udid, char * _Nullable * _Nullable error_message);
void xcw_native_session_destroy(void * _Nullable handle);
bool xcw_native_session_start(void * _Nonnull handle, char * _Nullable * _Nullable error_message);
void xcw_native_session_request_refresh(void * _Nonnull handle);
void xcw_native_session_request_keyframe(void * _Nonnull handle);
void xcw_native_session_reconfigure_video_encoder(void * _Nonnull handle);
bool xcw_native_session_send_touch(void * _Nonnull handle, double x, double y, const char * _Nonnull phase, char * _Nullable * _Nullable error_message);
bool xcw_native_session_send_multitouch(void * _Nonnull handle, double x1, double y1, double x2, double y2, const char * _Nonnull phase, char * _Nullable * _Nullable error_message);
bool xcw_native_session_send_key(void * _Nonnull handle, uint16_t key_code, uint32_t modifiers, char * _Nullable * _Nullable error_message);
bool xcw_native_session_press_home(void * _Nonnull handle, char * _Nullable * _Nullable error_message);
bool xcw_native_session_open_app_switcher(void * _Nonnull handle, char * _Nullable * _Nullable error_message);
bool xcw_native_session_rotate_right(void * _Nonnull handle, char * _Nullable * _Nullable error_message);
bool xcw_native_session_rotate_left(void * _Nonnull handle, char * _Nullable * _Nullable error_message);
void xcw_native_session_set_frame_callback(void * _Nonnull handle, xcw_native_frame_callback _Nullable callback, void * _Nullable user_data);

void xcw_native_free_string(char * _Nullable value);
void xcw_native_free_bytes(xcw_native_owned_bytes bytes);
void xcw_native_release_shared_bytes(xcw_native_shared_bytes bytes);

NS_ASSUME_NONNULL_END
