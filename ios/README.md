# SimDeck Studio iOS

Native SwiftUI client for SimDeck live sessions.

- Opens LAN, Tailscale, and SimDeck Studio URLs.
- Uses the daemon's `/api/simulators/{udid}/webrtc/offer` endpoint and renders the H.264 WebRTC track with Metal.
- Sends touch and hardware controls over the `simdeck-control` WebRTC data channel.
- Supports `https://simdeck.djdev.me/simulator/{id}` links through Associated Domains and the `simdeck://` custom URL scheme.

Open `SimDeckStudio.xcodeproj`, select the `SimDeckStudio` scheme, and run on an iPhone or iPad target. The app display name is `SimDeck`.
