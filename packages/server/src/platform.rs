//! Host platform capability reporting.
//!
//! SimDeck ships one CLI for macOS, Windows, and Linux, but live H.264 video
//! streaming depends on the macOS native simulator bridge and its
//! VideoToolbox/x264 encoders. Non-macOS builds compile `native_stubs.c`
//! instead of that bridge, so this module is the single place that describes
//! the gap for the CLI banner, the HTTP API, and the browser client.

use serde_json::{json, Value};

/// Operating system that provides the native H.264 encoder.
pub const LIVE_VIDEO_REQUIRED_OS: &str = "macos";

/// Rust target OS name for the running binary (`macos`, `windows`, `linux`).
pub fn host_os() -> &'static str {
    std::env::consts::OS
}

/// Whether this build includes the native H.264 encoder used by the browser
/// WebRTC stream for both iOS simulators and Android emulators.
pub fn live_video_supported() -> bool {
    cfg!(target_os = "macos")
}

/// Full user-facing explanation returned by the API when live video is
/// requested on a build that cannot encode it.
pub fn live_video_unsupported_message() -> String {
    live_video_unsupported_message_for(host_os())
}

/// Short note printed by the CLI after the service URLs.
pub fn live_video_cli_note() -> String {
    format!(
        "Unavailable on {}. Live H.264 streaming requires macOS.",
        os_display_name(host_os())
    )
}

/// JSON capability block shared by `/api/health` and `/api/stream-quality`.
pub fn live_video_capability_value() -> Value {
    live_video_capability_value_for(host_os(), live_video_supported())
}

pub(crate) fn live_video_unsupported_message_for(os: &str) -> String {
    format!(
        "Live H.264 video streaming requires macOS. This SimDeck build for {} can manage devices but does not include the native H.264 encoder, so the browser stream and the `--video-codec` setting are unavailable here.",
        os_display_name(os)
    )
}

pub(crate) fn live_video_capability_value_for(os: &str, supported: bool) -> Value {
    let mut value = json!({
        "supported": supported,
        "requires": LIVE_VIDEO_REQUIRED_OS,
    });
    if !supported {
        value["reason"] = Value::String(live_video_unsupported_message_for(os));
    }
    value
}

pub(crate) fn os_display_name(os: &str) -> String {
    match os {
        "macos" => "macOS".to_owned(),
        "windows" => "Windows".to_owned(),
        "linux" => "Linux".to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_message_names_the_host_and_the_requirement() {
        let message = live_video_unsupported_message_for("windows");
        assert!(message.contains("requires macOS"));
        assert!(message.contains("build for Windows"));
        assert!(message.contains("--video-codec"));
    }

    #[test]
    fn unsupported_message_falls_back_to_raw_os_name() {
        let message = live_video_unsupported_message_for("freebsd");
        assert!(message.contains("build for freebsd"));
    }

    #[test]
    fn capability_value_only_carries_a_reason_when_unsupported() {
        let unsupported = live_video_capability_value_for("linux", false);
        assert_eq!(unsupported["supported"], Value::Bool(false));
        assert_eq!(unsupported["requires"], Value::String("macos".to_owned()));
        assert!(unsupported["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("Linux")));

        let supported = live_video_capability_value_for("macos", true);
        assert_eq!(supported["supported"], Value::Bool(true));
        assert_eq!(supported["requires"], Value::String("macos".to_owned()));
        assert!(supported.get("reason").is_none());
    }

    #[test]
    fn live_video_support_matches_the_compiled_target() {
        assert_eq!(live_video_supported(), cfg!(target_os = "macos"));
        assert_eq!(host_os() == LIVE_VIDEO_REQUIRED_OS, live_video_supported());
    }
}
