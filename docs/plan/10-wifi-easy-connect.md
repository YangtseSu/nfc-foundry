# Phase 10: Full Wi-Fi Easy Connect / DPP

## Goal

Turn the current DPP-URI writer into a complete onboarding flow where NFC
supplies the DPP bootstrapping URI and the actual DPP authentication and
configuration run over Wi-Fi.

## Boundary

A DPP URI is bootstrapping information, not a static Wi-Fi credential. A static
tag alone cannot complete onboarding. This phase supplies the missing active
peer: a DPP Configurator/Enrollee running on the host or coordinated with an
external AP/service.

## Work

- Integrate with Linux `wpa_supplicant`/`hostapd` DPP support through their
  control interfaces or D-Bus; do not reimplement the DPP cryptography first.
- Support both directions:
  - write a DPP URI for a host/AP responder to an NFC tag;
  - read a DPP URI from a tag and start the local DPP role.
- Model DPP roles and capabilities explicitly (Configurator, Enrollee,
  Responder, Initiator).
- Validate WPA3-only provisioning and WPA2/WPA3 transition behavior.
- Provide a clear error when no DPP-capable Wi-Fi interface or external
  configurator is available.
- Keep the NFC write side usable without the Wi-Fi stack.
- Add hardware validation with an Android phone and/or a Linux AP, plus a
  disposable tag.

## Acceptance

- A tag written by NFC Foundry can bootstrap a real DPP exchange.
- A DPP URI read from a tag can start the local configured role.
- WPA3-only provisioning succeeds on the validated test setup.
- The app never claims that a static tag alone provides Wi-Fi credentials.
- Failures distinguish NFC, DPP, Wi-Fi, and role/configuration errors.
- `make check` passes; DPP hardware tests are explicit.

## Non-goals

- No custom DPP cryptographic implementation unless the existing Linux stack
  cannot meet the required role.
- No claim of universal phone/AP compatibility.
- No credential exposure in logs or NDEF output beyond the standard DPP URI.
