# LAN Access

SimDeck binds to `127.0.0.1` by default. Bind to a LAN address when another device needs to open the browser UI.

## Start A LAN Session

```sh
simdeck ui \
  --bind 0.0.0.0 \
  --advertise-host 192.168.1.50 \
  --open
```

Open the printed network URL from the remote browser:

```text
http://192.168.1.50:4310
```

Enter the pairing code printed by the CLI. After pairing, the browser receives the API cookie.

## Pick The Right Host

Use an IP address or hostname that the remote device can resolve:

```sh
simdeck ui --bind 0.0.0.0 --advertise-host my-mac.local --open
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
```

If you bind to `0.0.0.0` but advertise `localhost`, remote browsers will try to connect to themselves.

## Direct API Access

Loopback browser sessions are authenticated automatically. Direct API callers should send the token from:

```sh
simdeck daemon status
```

Use either header:

```text
X-SimDeck-Token: <token>
Authorization: Bearer <token>
```

## Security Checklist

- Use LAN mode only on networks you trust.
- Treat the daemon token as a secret.
- Restrict the port with macOS Firewall when needed.
- Stop the daemon when the shared session is done:

  ```sh
  simdeck daemon stop
  ```

## Troubleshooting

| Symptom                       | Fix                                                               |
| ----------------------------- | ----------------------------------------------------------------- |
| Remote browser cannot connect | Confirm `--bind 0.0.0.0`, firewall rules, and the advertised host |
| Pairing code is rejected      | Restart the daemon and use the newly printed code                 |
| Stream connects but stutters  | Use `--stream-quality low` or `--video-codec software`            |
| API script gets `401`         | Send `X-SimDeck-Token` or `Authorization: Bearer <token>`         |
