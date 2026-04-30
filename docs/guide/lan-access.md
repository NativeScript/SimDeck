# LAN Access

SimDeck binds to `127.0.0.1` by default. You can move it to a LAN-reachable interface so other devices on your network — another Mac, an iPad, a phone — can stream the simulator.

## Bind to all interfaces

Use `--bind` to listen on a non-loopback address:

```sh
simdeck ui --port 4310 --bind 0.0.0.0 --open
```

Both the HTTP server and the WebTransport server bind to the requested address. The HTTP server is plain HTTP, so any browser on the LAN can reach it through `http://<your-mac-ip>:4310`. LAN browsers must enter the pairing code printed by the foreground command or returned by `daemon start` before the API cookie is issued.

## Advertise the right host

WebTransport needs a hostname or IP that matches the certificate the server generates. By default SimDeck advertises `localhost`, which only works for browsers running on the same Mac.

Tell the server what host to advertise to remote clients:

```sh
simdeck ui \
  --port 4310 \
  --bind 0.0.0.0 \
  --advertise-host 192.168.1.50 \
  --open
```

The advertised host shows up in three places:

- The `webTransport.urlTemplate` field on `GET /api/health`.
- The Subject Alternative Name list on the self-signed WebTransport certificate.
- The certificate hash that the client pins by SHA-256.

If you skip `--advertise-host` while binding to `0.0.0.0`, the server prints a warning at startup because it will still tell remote clients to dial `localhost`.

## Pick a hostname or IP

You can advertise either a DNS name (preferred when you have a stable mDNS or DHCP entry) or an IP literal. Examples:

```sh
simdeck ui --bind 0.0.0.0 --advertise-host my-mac.lan --open
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
simdeck ui --bind ::      --advertise-host my-mac.local --open
```

Whatever you advertise must be resolvable from the remote client.

## Certificate handling

The server generates a fresh self-signed certificate every time it starts. The certificate's SHA-256 hash is exposed on `GET /api/health`:

```json
{
  "ok": true,
  "httpPort": 4310,
  "wtPort": 4311,
  "videoCodec": "h264-software",
  "webTransport": {
    "urlTemplate": "https://192.168.1.50:4311/wt/simulators/{udid}?simdeckToken=...",
    "certificateHash": {
      "algorithm": "sha-256",
      "value": "..."
    },
    "packetVersion": 1
  }
}
```

The browser client passes that hash to the WebTransport API as the `serverCertificateHashes` option, so no certificate trust prompts are involved. As long as the client fetched the hash through HTTP and dialled the same advertised host, the WebTransport handshake completes without any user interaction.

Restarting the server invalidates the previous certificate. Open clients reconnect automatically as soon as `/api/health` reports the new hash.

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

On LAN HTTP, browsers that cannot use WebTransport fall back to WebRTC. The WebTransport URL template returned by authenticated `GET /api/health` includes a `simdeckToken` query parameter for the browser stream worker.

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
3. Allow the chosen ports through any firewalls.
4. Visit `http://<advertise-host>:<port>` from the remote device.
