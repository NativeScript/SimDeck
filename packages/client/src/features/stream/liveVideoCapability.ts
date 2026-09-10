import type { LiveVideoCapability } from "../../api/types";

export const LIVE_VIDEO_UNAVAILABLE_MESSAGE =
  "This SimDeck server cannot stream live video to the browser.";

/**
 * Returns the user-facing reason live video is unavailable on the connected
 * server, or an empty string when the server can stream (or did not say).
 *
 * Every shipped build streams (macOS through the native bridge, Windows and
 * Linux through OpenH264), so this only trips for a server that explicitly
 * reports `supported: false`. Servers older than the capability field omit
 * `liveVideo`; treat that as supported so the client keeps attempting WebRTC.
 */
export function liveVideoUnavailableReason(
  capability: LiveVideoCapability | null | undefined,
): string {
  if (!capability || capability.supported !== false) {
    return "";
  }
  const reason = capability.reason?.trim();
  return reason || LIVE_VIDEO_UNAVAILABLE_MESSAGE;
}
