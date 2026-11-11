# Spec: phase 24e Type 2 authenticated access

Status: planned. Plan: [24-parity-recovery.md](../plan/24-parity-recovery.md).
Slice: S6.

## Problem

The Type 2 workflow can program PWD, PACK, AUTH0, and ACCESS, but it has no
authentication path for the protected state it creates. After `AUTH0` is set,
pages at or above `AUTH0` reject reads and writes, so the application cannot
read its own NDEF message, write it, or change the configuration back. The
existing password hardware test proves only that `PWD_AUTH` returns the PACK;
it leaves the tag protected and unusable by the rest of the product. That is
not a shippable security feature.

## Scope

Supported:

- `PWD_AUTH` for the selected NTAG/Type 2 tag, with the returned PACK checked
  before the session is accepted.
- Authenticated reads, page writes, NDEF read/write/format, and configuration
  writes for pages protected by `AUTH0`.
- Changing PWD/PACK/AUTH0/ACCESS on a tag that is already protected, using the
  current password first.
- Clearing protection by writing `AUTH0=0xFF` and `ACCESS=0x00` with a known
  password.
- CLI and GUI password entry through the same service path.

Not in this slice:

- Recovering an unknown password or bypassing AUTHLIM/CFGLCK.
- Reading PWD/PACK back. They remain write-only; a dump cannot restore them.
- `GET_VERSION`, `FAST_READ`, `READ_CNT`, and `READ_SIG`.

## Authentication session

`Type2Auth` carries the four-byte password and the expected two-byte PACK. The
service opens one write or read operation, reads the configuration, and
authenticates once before the first protected page. A wrong PACK fails before
any mutation. The session ends with the operation, a card change, a reader
change, or a field reset; no secret is retained in `Service`, the workbench,
the key store, a dump, or an activity row.

The read path may authenticate after the configuration read, because the
configuration pages themselves are readable until `AUTH0` protects them. If
`AUTH0` already protects the configuration pages and the caller did not supply
a password, the service reports a protected tag instead of a partial image.

## Configuration changes

`Type2Password` gains an optional current password. The sequence is:

1. Read the current configuration and reject `CFGLCK`.
2. If the current `AUTH0` protects the configuration, authenticate with the
   current password and require the configured PACK.
3. Write PWD and PACK.
4. Write CFG1 and CFG0, with `AUTH0`/`ACCESS` last.
5. Authenticate with the new password and require the new PACK.
6. Read back the configuration and verify the intended values.

Clearing protection uses the same path with `AUTH0=0xFF` and `ACCESS=0x00`,
authenticated by the current password. A failure after PWD/PACK is written
reports the exact committed pages and the password state; it never claims the
tag is unprotected or usable.

## CLI

`nfctl type2 password --auth0 N --access HEX --pwd HEX --pack HEX
[--current-pwd HEX]` and the protected read/write paths accept
`--type2-password HEX`. Secrets are read from the environment or an explicit
flag, never printed in the plan. Non-interactive use requires the existing
confirmation flag plus the password flag.

## GUI

The Protection form gains a password entry for protected access and a
"current password" field when changing an existing protected configuration.
The entry is masked, cleared after the operation, and never written to
`ui.json`. The same service calls and confirmation gates apply.

## Tests

- Mock tests: correct and wrong password, wrong PACK, protected read/write,
  configuration change, protection clear, authentication after `AUTH0` change,
  and cancellation before the first write.
- Service tests prove a wrong password leaves the tag unchanged and that no
  backup, dump, activity row, or error contains the secret.
- Hardware test on an owned NTAG215: read the original vCard, set
  `AUTH0=0x20` with a known password, read and rewrite the NDEF message
  through authentication, restore `AUTH0=0xFF`/`ACCESS=0x00`, and verify the
  original vCard. Use a dedicated tag if the test fails partway.

## Safety

- Never program a protected state without a known password and a tested
  authentication path.
- PWD/PACK are written before AUTH0/ACCESS.
- A failed authentication stops before the first mutation.
- A protected configuration cannot be changed without the current password.
- No password is stored in a file or log.
