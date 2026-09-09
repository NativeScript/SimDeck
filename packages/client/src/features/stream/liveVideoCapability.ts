import type { LiveVideoCapability } from "../../api/types";

export const LIVE_VIDEO_UNAVAILABLE_MESSAGE =
  "Live H.264 video streaming requires macOS. This SimDeck build can manage devices but cannot stream video to the browser.";

/**
 * Returns the user-facing reason live video is unavailable on the connected
 * server, or an empty string when the server can stream (or did not say).
 *
 * Servers older than the capability field omit `liveVideo`; treat that as
 * supported so the client keeps attempting WebRTC against macOS hosts.
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
