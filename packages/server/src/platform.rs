//! Host platform capability reporting.
//!
//! SimDeck ships one CLI for macOS, Windows, and Linux. The macOS build links
//! the private simulator bridge, which owns iOS simulator control and the
//! VideoToolbox/x264 encoders. Windows and Linux builds compile
//! `native_stubs.c` in its place, so they cannot drive iOS simulators, but they
//! still stream Android emulators through the Rust OpenH264 encoder in
//! `transport::software_h264`. This module is the single place that describes
//! those differences for the CLI banner, the HTTP API, and the browser client.

use serde_json::{json, Value};
use std::path::PathBuf;

/// Rust target OS name for the running binary (`macos`, `windows`, `linux`).
pub fn host_os() -> &'static str {
    std::env::consts::OS
}

/// The current user's home directory, or `None` when no environment variable
/// describes one.
///
/// Unix shells always export `HOME`. On Windows only Git Bash and similar
/// environments do; PowerShell and cmd expose `USERPROFILE` (and the older
/// `HOMEDRIVE`/`HOMEPATH` pair) instead. Every SimDeck path under the home
/// directory must resolve the same way in all of them, otherwise a service
/// started from one shell is invisible to the CLI in another.
pub fn user_home_dir() -> Option<PathBuf> {
    let from_env = |key: &str| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    from_env("HOME")
        .or_else(|| from_env("USERPROFILE"))
        .or_else(|| {
            let drive = from_env("HOMEDRIVE")?;
            let path = from_env("HOMEPATH")?;
            Some(drive.join(path))
        })
}

/// Whether this build includes the native simulator bridge needed to boot,
/// control, and stream iOS simulators.
pub fn ios_simulator_supported() -> bool {
    cfg!(target_os = "macos")
}

/// Name of the H.264 encoder that produces the live browser stream.
#[cfg(target_os = "macos")]
pub fn live_video_encoder() -> &'static str {
    "native"
}

/// Name of the H.264 encoder that produces the live browser stream.
#[cfg(not(target_os = "macos"))]
pub fn live_video_encoder() -> &'static str {
    crate::transport::software_h264::SOFTWARE_H264_ENCODER_NAME
}

/// Full user-facing explanation returned by the API when an iOS simulator
/// stream is requested on a build without the native bridge.
pub fn ios_simulator_unsupported_message() -> String {
    ios_simulator_unsupported_message_for(host_os())
}

/// Short note printed by the CLI after the service URLs on non-macOS hosts.
pub fn live_video_cli_note() -> String {
    format!(
        "Android emulators stream with software H.264 ({}). iOS simulators require macOS.",
        live_video_encoder()
    )
}

/// JSON capability block shared by `/api/health` and `/api/stream-quality`.
pub fn live_video_capability_value() -> Value {
    live_video_capability_value_for(host_os(), live_video_encoder(), ios_simulator_supported())
}

pub(crate) fn ios_simulator_unsupported_message_for(os: &str) -> String {
    format!(
        "iOS simulators require macOS. This SimDeck build for {} can boot, control, and stream Android emulators, but the iOS simulator bridge is only available on macOS.",
        os_display_name(os)
    )
}

pub(crate) fn live_video_capability_value_for(
    os: &str,
    encoder: &str,
    ios_simulator: bool,
) -> Value {
    let mut value = json!({
        "supported": true,
        "encoder": encoder,
        "iosSimulator": ios_simulator,
        "androidEmulator": true,
    });
    if !ios_simulator {
        value["iosSimulatorReason"] = Value::String(ios_simulator_unsupported_message_for(os));
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
    fn ios_message_names_the_host_and_the_requirement() {
        let message = ios_simulator_unsupported_message_for("windows");
        assert!(message.starts_with("iOS simulators require macOS."));
        assert!(message.contains("build for Windows"));
        assert!(message.contains("Android emulators"));
    }

    #[test]
    fn ios_message_falls_back_to_raw_os_name() {
        let message = ios_simulator_unsupported_message_for("freebsd");
        assert!(message.contains("build for freebsd"));
    }

    #[test]
    fn capability_value_reports_android_everywhere_and_ios_only_on_macos() {
        let windows = live_video_capability_value_for("windows", "openh264", false);
        assert_eq!(windows["supported"], Value::Bool(true));
        assert_eq!(windows["encoder"], Value::String("openh264".to_owned()));
        assert_eq!(windows["androidEmulator"], Value::Bool(true));
        assert_eq!(windows["iosSimulator"], Value::Bool(false));
        assert!(windows["iosSimulatorReason"]
            .as_str()
            .is_some_and(|reason| reason.contains("Windows")));

        let macos = live_video_capability_value_for("macos", "native", true);
        assert_eq!(macos["supported"], Value::Bool(true));
        assert_eq!(macos["encoder"], Value::String("native".to_owned()));
        assert_eq!(macos["iosSimulator"], Value::Bool(true));
        assert!(macos.get("iosSimulatorReason").is_none());
    }

    #[test]
    fn ios_support_matches_the_compiled_target() {
        assert_eq!(ios_simulator_supported(), cfg!(target_os = "macos"));
        assert_eq!(host_os() == "macos", ios_simulator_supported());
        assert_eq!(live_video_encoder() == "native", ios_simulator_supported());
    }

    #[test]
    fn cli_note_names_the_encoder() {
        let note = live_video_cli_note();
        assert!(note.contains(live_video_encoder()));
        assert!(note.contains("iOS simulators require macOS"));
    }
}
