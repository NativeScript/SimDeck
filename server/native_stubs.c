#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef struct {
  uint8_t *data;
  uintptr_t length;
} xcw_native_owned_bytes;

typedef struct {
  const uint8_t *data;
  uintptr_t length;
  const void *owner;
} xcw_native_shared_bytes;

typedef struct {
  uint64_t frame_sequence;
  uint64_t timestamp_us;
  bool is_keyframe;
  uint32_t width;
  uint32_t height;
  const char *codec;
  xcw_native_shared_bytes description;
  xcw_native_shared_bytes data;
} xcw_native_frame;

typedef void (*xcw_native_frame_callback)(const xcw_native_frame *frame,
                                          void *user_data);

static char *xcw_strdup(const char *value) {
  if (value == NULL) {
    value = "";
  }
  size_t length = strlen(value);
  char *copy = (char *)malloc(length + 1);
  if (copy != NULL) {
    memcpy(copy, value, length + 1);
  }
  return copy;
}

static void xcw_set_error(char **error_message, const char *message) {
  if (error_message != NULL) {
    *error_message = xcw_strdup(message);
  }
}

static bool xcw_unsupported(char **error_message) {
  xcw_set_error(error_message,
                "iOS simulator native bridge is only available on macOS.");
  return false;
}

static xcw_native_owned_bytes xcw_empty_bytes(char **error_message) {
  xcw_unsupported(error_message);
  xcw_native_owned_bytes bytes = {0};
  return bytes;
}

void xcw_native_initialize_app(void) {}

void xcw_native_run_main_loop_slice(double duration_seconds) {
  if (duration_seconds <= 0.0) {
    return;
  }
  time_t seconds = (time_t)duration_seconds;
  long nanos = (long)((duration_seconds - (double)seconds) * 1000000000.0);
  if (nanos < 0) {
    nanos = 0;
  }
  struct timespec delay = {.tv_sec = seconds, .tv_nsec = nanos};
  nanosleep(&delay, NULL);
}

char *xcw_native_list_simulators(char **error_message) {
  (void)error_message;
  return xcw_strdup("{\"simulators\":[]}");
}

char *xcw_native_simulator_creation_options(char **error_message) {
  (void)error_message;
  return xcw_strdup("{\"deviceTypes\":[],\"runtimes\":[]}");
}

char *xcw_native_create_simulator(const char *name,
                                  const char *device_type_identifier,
                                  const char *runtime_identifier,
                                  const char *paired_watch_name,
                                  const char *paired_watch_device_type_identifier,
                                  const char *paired_watch_runtime_identifier,
                                  char **error_message) {
  (void)name;
  (void)device_type_identifier;
  (void)runtime_identifier;
  (void)paired_watch_name;
  (void)paired_watch_device_type_identifier;
  (void)paired_watch_runtime_identifier;
  xcw_unsupported(error_message);
  return NULL;
}

bool xcw_native_boot_simulator(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_shutdown_simulator(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_toggle_appearance(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_open_url(const char *udid, const char *url,
                         char **error_message) {
  (void)udid;
  (void)url;
  return xcw_unsupported(error_message);
}

bool xcw_native_launch_bundle(const char *udid, const char *bundle_id,
                              char **error_message) {
  (void)udid;
  (void)bundle_id;
  return xcw_unsupported(error_message);
}

char *xcw_native_get_chrome_profile(const char *udid, char **error_message) {
  (void)udid;
  xcw_unsupported(error_message);
  return NULL;
}

xcw_native_owned_bytes xcw_native_render_chrome_png(const char *udid,
                                                    bool include_buttons,
                                                    char **error_message) {
  (void)udid;
  (void)include_buttons;
  return xcw_empty_bytes(error_message);
}

xcw_native_owned_bytes xcw_native_render_chrome_button_png(
    const char *udid, const char *button_name, bool pressed,
    char **error_message) {
  (void)udid;
  (void)button_name;
  (void)pressed;
  return xcw_empty_bytes(error_message);
}

xcw_native_owned_bytes xcw_native_render_screen_mask_png(
    const char *udid, char **error_message) {
  (void)udid;
  return xcw_empty_bytes(error_message);
}

xcw_native_owned_bytes xcw_native_screenshot_png(const char *udid,
                                                 bool include_bezel,
                                                 char **error_message) {
  (void)udid;
  (void)include_bezel;
  return xcw_empty_bytes(error_message);
}

xcw_native_owned_bytes xcw_native_screen_recording_mp4(
    const char *udid, double duration_seconds, char **error_message) {
  (void)udid;
  (void)duration_seconds;
  return xcw_empty_bytes(error_message);
}

char *xcw_native_recent_logs(const char *udid, double seconds, uintptr_t limit,
                             char **error_message) {
  (void)udid;
  (void)seconds;
  (void)limit;
  xcw_unsupported(error_message);
  return NULL;
}

char *xcw_native_accessibility_snapshot(const char *udid, bool has_point,
                                        double x, double y,
                                        uintptr_t max_depth,
                                        char **error_message) {
  (void)udid;
  (void)has_point;
  (void)x;
  (void)y;
  (void)max_depth;
  xcw_unsupported(error_message);
  return NULL;
}

bool xcw_native_send_touch(const char *udid, double x, double y,
                           const char *phase, char **error_message) {
  (void)udid;
  (void)x;
  (void)y;
  (void)phase;
  return xcw_unsupported(error_message);
}

bool xcw_native_send_key(const char *udid, uint16_t key_code,
                         uint32_t modifiers, char **error_message) {
  (void)udid;
  (void)key_code;
  (void)modifiers;
  return xcw_unsupported(error_message);
}

bool xcw_native_press_home(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_open_app_switcher(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_press_button(const char *udid, const char *button_name,
                             uint32_t duration_ms, char **error_message) {
  (void)udid;
  (void)button_name;
  (void)duration_ms;
  return xcw_unsupported(error_message);
}

bool xcw_native_send_button(const char *udid, const char *button_name,
                            bool pressed, bool has_usage, uint32_t usage_page,
                            uint32_t usage, char **error_message) {
  (void)udid;
  (void)button_name;
  (void)pressed;
  (void)has_usage;
  (void)usage_page;
  (void)usage;
  return xcw_unsupported(error_message);
}

bool xcw_native_rotate_crown(const char *udid, double delta,
                             char **error_message) {
  (void)udid;
  (void)delta;
  return xcw_unsupported(error_message);
}

bool xcw_native_rotate_right(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_rotate_left(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_erase_simulator(const char *udid, char **error_message) {
  (void)udid;
  return xcw_unsupported(error_message);
}

bool xcw_native_install_app(const char *udid, const char *app_path,
                            char **error_message) {
  (void)udid;
  (void)app_path;
  return xcw_unsupported(error_message);
}

bool xcw_native_uninstall_app(const char *udid, const char *bundle_id,
                              char **error_message) {
  (void)udid;
  (void)bundle_id;
  return xcw_unsupported(error_message);
}

bool xcw_native_set_pasteboard_text(const char *udid, const char *text,
                                    char **error_message) {
  (void)udid;
  (void)text;
  return xcw_unsupported(error_message);
}

char *xcw_native_get_pasteboard_text(const char *udid, char **error_message) {
  (void)udid;
  xcw_unsupported(error_message);
  return NULL;
}

void *xcw_native_input_create(const char *udid, char **error_message) {
  (void)udid;
  xcw_unsupported(error_message);
  return NULL;
}

void xcw_native_input_destroy(void *handle) { (void)handle; }

bool xcw_native_input_display_size(void *handle, double *width,
                                   double *height) {
  (void)handle;
  if (width != NULL) {
    *width = 0.0;
  }
  if (height != NULL) {
    *height = 0.0;
  }
  return false;
}

bool xcw_native_input_send_touch(void *handle, double x, double y,
                                 const char *phase, char **error_message) {
  (void)handle;
  (void)x;
  (void)y;
  (void)phase;
  return xcw_unsupported(error_message);
}

bool xcw_native_input_send_multitouch(void *handle, double x1, double y1,
                                      double x2, double y2,
                                      const char *phase,
                                      char **error_message) {
  (void)handle;
  (void)x1;
  (void)y1;
  (void)x2;
  (void)y2;
  (void)phase;
  return xcw_unsupported(error_message);
}

bool xcw_native_input_send_key(void *handle, uint16_t key_code,
                               uint32_t modifiers, char **error_message) {
  (void)handle;
  (void)key_code;
  (void)modifiers;
  return xcw_unsupported(error_message);
}

bool xcw_native_input_send_key_event(void *handle, uint16_t key_code,
                                     bool down, char **error_message) {
  (void)handle;
  (void)key_code;
  (void)down;
  return xcw_unsupported(error_message);
}

void *xcw_native_session_create(const char *udid, char **error_message) {
  (void)udid;
  xcw_unsupported(error_message);
  return NULL;
}

void xcw_native_session_destroy(void *handle) { (void)handle; }

bool xcw_native_session_start(void *handle, char **error_message) {
  (void)handle;
  return xcw_unsupported(error_message);
}

void xcw_native_session_request_refresh(void *handle) { (void)handle; }

void xcw_native_session_request_keyframe(void *handle) { (void)handle; }

void xcw_native_session_reconfigure_video_encoder(void *handle) {
  (void)handle;
}

void xcw_native_session_set_client_foreground(void *handle, bool foreground) {
  (void)handle;
  (void)foreground;
}

char *xcw_native_session_video_encoder_stats(void *handle,
                                             char **error_message) {
  (void)handle;
  (void)error_message;
  return xcw_strdup("{}");
}

int32_t xcw_native_session_rotation_quarter_turns(void *handle) {
  (void)handle;
  return 0;
}

void xcw_native_session_set_frame_callback(
    void *handle, xcw_native_frame_callback callback, void *user_data) {
  (void)handle;
  (void)callback;
  (void)user_data;
}

bool xcw_native_session_send_touch(void *handle, double x, double y,
                                   const char *phase,
                                   char **error_message) {
  (void)handle;
  (void)x;
  (void)y;
  (void)phase;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_send_edge_touch(void *handle, double x, double y,
                                        const char *phase, uint32_t edge,
                                        char **error_message) {
  (void)handle;
  (void)x;
  (void)y;
  (void)phase;
  (void)edge;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_send_multitouch(void *handle, double x1, double y1,
                                        double x2, double y2,
                                        const char *phase,
                                        char **error_message) {
  (void)handle;
  (void)x1;
  (void)y1;
  (void)x2;
  (void)y2;
  (void)phase;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_send_key(void *handle, uint16_t key_code,
                                 uint32_t modifiers, char **error_message) {
  (void)handle;
  (void)key_code;
  (void)modifiers;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_press_home(void *handle, char **error_message) {
  (void)handle;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_press_button(void *handle, const char *button_name,
                                     uint32_t duration_ms,
                                     char **error_message) {
  (void)handle;
  (void)button_name;
  (void)duration_ms;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_send_button(void *handle, const char *button_name,
                                    bool pressed, bool has_usage,
                                    uint32_t usage_page, uint32_t usage,
                                    char **error_message) {
  (void)handle;
  (void)button_name;
  (void)pressed;
  (void)has_usage;
  (void)usage_page;
  (void)usage;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_rotate_crown(void *handle, double delta,
                                     char **error_message) {
  (void)handle;
  (void)delta;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_open_app_switcher(void *handle, char **error_message) {
  (void)handle;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_rotate_right(void *handle, char **error_message) {
  (void)handle;
  return xcw_unsupported(error_message);
}

bool xcw_native_session_rotate_left(void *handle, char **error_message) {
  (void)handle;
  return xcw_unsupported(error_message);
}

void *xcw_native_h264_encoder_create(xcw_native_frame_callback callback,
                                     void *user_data, char **error_message) {
  (void)callback;
  (void)user_data;
  xcw_set_error(error_message,
                "H.264 encoding is only available in the macOS native bridge.");
  return NULL;
}

void xcw_native_h264_encoder_destroy(void *handle) { (void)handle; }

bool xcw_native_h264_encoder_encode_rgba(void *handle, const uint8_t *rgba,
                                         uintptr_t length, uint32_t width,
                                         uint32_t height,
                                         uint64_t timestamp_us,
                                         char **error_message) {
  (void)handle;
  (void)rgba;
  (void)length;
  (void)width;
  (void)height;
  (void)timestamp_us;
  xcw_set_error(error_message,
                "H.264 encoding is only available in the macOS native bridge.");
  return false;
}

void xcw_native_h264_encoder_request_keyframe(void *handle) { (void)handle; }

void xcw_native_free_string(char *value) { free(value); }

void xcw_native_free_bytes(xcw_native_owned_bytes bytes) { free(bytes.data); }

void xcw_native_release_shared_bytes(xcw_native_shared_bytes bytes) {
  (void)bytes;
}
