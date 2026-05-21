---
title: Support
description: Get help with SimDeck Studio, pairing, simulator streaming, and local setup.
---

# Support

Need help with SimDeck Studio, the SimDeck CLI, browser streaming, pairing, or simulator control?

## Contact

Email [support@nativescript.org](mailto:support@nativescript.org) for app issues, setup questions, general feedback, and feature requests.

## What To Include

When you contact support, include the details that match your issue:

- SimDeck Studio app version, iOS or iPadOS version, and device model.
- macOS, Xcode, and SimDeck CLI versions if you are connecting to a Mac.
- Whether you are using local Wi-Fi, Tailscale, localhost, or a hosted Studio session.
- A short description of what you expected and what happened instead.
- Any visible error message, screenshot, or screen recording.
- For pairing issues, the command you ran, such as `simdeck pair`, and whether the QR code or manual server entry failed.

Do not send access tokens, private pairing links, or credentials in public issue trackers. If you accidentally share a token, run `simdeck service reset` before reconnecting.

## Common Help Topics

- [Install SimDeck](/guide/installation)
- [Start a local session](/guide/quick-start)
- [Pair over LAN](/guide/lan-access)
- [Use the CLI](/cli/commands)
- [Troubleshoot startup, streaming, and inspectors](/guide/troubleshooting)

## Pairing QR Codes

SimDeck Studio pairs with a SimDeck server by scanning the QR code printed by:

```sh
simdeck pair
```

The QR code contains a `simdeck://pair` link, the current pairing code, and detected local network addresses for your SimDeck server. The camera is used only to scan that pairing code.

## Public Issues

For reproducible bugs that do not include private project details, you can also file an issue in the [SimDeck GitHub repository](https://github.com/NativeScript/SimDeck/issues).
