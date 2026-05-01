# LAN Access

SimDeck binds to `127.0.0.1` by default. You can move it to a LAN-reachable interface so other devices on your network — another Mac, an iPad, a phone — can stream the simulator.

## Bind to all interfaces

Use `--bind` to listen on a non-loopback address:

```sh
simdeck ui --port 4310 --bind 0.0.0.0 --open
```

The HTTP server binds to the requested address. It serves the REST API, browser UI, inspector WebSocket, and WebRTC offer endpoint, so any browser on the LAN can reach SimDeck through `http://<your-mac-ip>:4310`. LAN browsers must enter the pairing code printed by the foreground command or returned by `daemon start` before the API cookie is issued.

## Advertise the right host

By default SimDeck advertises `localhost`, which only works for browsers running on the same Mac. Tell the server what host to print for remote clients:

```sh
simdeck ui \
  --port 4310 \
  --bind 0.0.0.0 \
  --advertise-host 192.168.1.50 \
  --open
```

The advertised host is used in the printed Network URL and in `daemon start` JSON output.

If you skip `--advertise-host` while binding to `0.0.0.0`, the server prints a warning at startup because it will still tell remote clients to dial `localhost`.

## Pick a hostname or IP

You can advertise either a DNS name (preferred when you have a stable mDNS or DHCP entry) or an IP literal. Examples:

```sh
simdeck ui --bind 0.0.0.0 --advertise-host my-mac.lan --open
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
simdeck ui --bind ::      --advertise-host my-mac.local --open
```

Whatever you advertise must be resolvable from the remote client.

## Health response

`GET /api/health` reports the HTTP port and active video encoder mode:

```json
{
  "ok": true,
  "httpPort": 4310,
  "videoCodec": "auto",
  "lowLatency": false,
  "webRtc": {
    "iceServers": [{ "urls": ["stun:stun.l.google.com:19302"] }],
    "iceTransportPolicy": "all"
  }
}
```

Restarting the server rotates the generated API access token. Open clients reconnect automatically after pairing or receiving the loopback cookie again.

## Authentication and security

SimDeck generates an API access token when it starts the project daemon. Loopback browser UI loads receive the token automatically through a strict same-site cookie, so opening `http://127.0.0.1:<port>` remains seamless. Non-loopback LAN browsers do not receive that cookie from the static page; they must submit the six-digit pairing code shown by the CLI before the server sets the cookie.

Direct API callers must send one of:

```text
X-SimDeck-Token: <token>
Authorization: Bearer <token>
```

The foreground `simdeck` command prints an HTTP network URL and a pairing code:

```text
🚀 SimDeck is ready

      Local:   http://127.0.0.1:4310
    Network:   http://192.168.1.50:4310
       Pair:   123 456

q or ^C to stop server
```

Get the token for scripts with:

```sh
simdeck daemon status
```

Recommended practice for shared networks:

- Run SimDeck only on networks you control.
- Treat the token from `daemon status` as a secret for scripted LAN access.
- Combine with macOS Application Firewall to restrict inbound access to known peers.
- For shared NativeScript inspectors, set an `authToken` when starting the [Swift in-app agent](/inspector/swift#auth-token) so app-side requests must include the token.

## Quick checklist

To make a SimDeck server reachable from another device:

1. `--bind 0.0.0.0` (or `--bind ::`).
2. `--advertise-host <reachable-host-or-ip>`.
3. Allow the chosen port through any firewalls.
4. Visit `http://<advertise-host>:<port>` from the remote device.
