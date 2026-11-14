# ADR 0007: ACR122U PC/SC is a second transport

- Status: accepted
- Date: 2026-11-14
- Supersedes: the ACR122U PC/SC exclusion in
  [ADR 0002](0002-reader-scope.md)

## Context

The ACR122U presents one CCID interface. Direct USB/CCID (`acr122_usb`) claims
it through `nusb` and is the validated default. When `pcscd` owns the reader,
the interface cannot be claimed, and users who already run a smart-card
service had no supported path.

The ACR122U firmware exposes its internal PN532 through the same pseudo-APDU
on two PC/SC channels:

- `SCARD_SHARE_DIRECT` with the CCID escape control (`SCARD_CTL_CODE(3500)`):
  raw PN532 frames with no card activation. This is the working path on the
  ACR122U214 firmware.
- `SCARD_SHARE_SHARED` with T=1: pseudo-APDUs in `SCardTransmit`, a fallback
  for firmware that rejects direct mode.

Both channels return the same `D5 <command+1> ... 90 00` responses that the
direct USB link returns. The reference behavior matches libnfc-ng's
`acr122_pcsc` driver.

## Decision

- Add `acr122_pcsc` as a second transport for the same reader. It is not
  generic PC/SC support.
- Use the `pcsc` crate over the system `libpcsclite`; the dependency owns the
  FFI. No `unsafe` code is added to `nfc-drivers`.
- Reuse `Acr122Protocol`; no second ACR122 command layer.
- Open in `SCARD_SHARE_DIRECT` and send PN532 frames through the CCID escape
  control. Fall back to `SCARD_SHARE_SHARED` with T=1 only when direct mode
  fails; report both failures when the fallback also fails.
- Enumerate only reader names containing `ACR122`, case-insensitively. The
  connstring is `acr122_pcsc:<exact reader name>`.
- Direct USB remains the default ACR122U transport. The two transports are
  never used at the same time and never fall back to each other.
- `pcscd` must be running for this path. Enumeration reports no PC/SC devices
  when the service is unavailable instead of failing the whole reader list.
- The `acr122-pcsc` cargo feature is on by default; `--no-default-features`
  builds the direct-USB-only workspace.
- Capabilities follow the hardware rows for this transport, not the direct USB
  profile.

## Consequences

- Release packages depend on `pcsclite`; CI installs `libpcsclite-dev`.
- `nfctl diag` and the GUI diagnostics page report PC/SC availability,
  `pcscd` reachability, and matched ACR122U reader names.
- `nfctl list` and the GUI reader rail show `ACR122U USB` and
  `ACR122U PC/SC` separately when both are visible.
- Raw bits and recovery parity are validated over PC/SC direct mode; a
  firmware that only accepts card mode would report those rows as absent until
  validated.
