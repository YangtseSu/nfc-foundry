# Spec: 29.3 device selection

Status: implemented 2026-11-14.

## Goal

A script can select a reader without guessing, and an ambiguous setup fails
instead of silently picking an arbitrary device. The GUI keeps its
auto-select-first behavior but makes the resolved reader visible.

## Resolution order

`nfctl` reader commands resolve one connstring in this order:

1. explicit `--device <connstring>`;
2. the `NFC_FOUNDRY_DEVICE` environment variable;
3. the only enumerated supported reader;
4. zero enumerated readers: exit 4, `code: "no_reader"`;
5. several enumerated readers: exit 2, `code: "usage"`, naming the candidate
   connstrings.

Only the first present source participates. An explicit flag suppresses the
environment entirely, including an invalid environment value. An empty
environment value counts as unset.

The selected connstring must be in the freshly enumerated device list.
`--device` and `NFC_FOUNDRY_DEVICE` values outside that list exit 2. This keeps
selection tied to supported, currently present hardware instead of a
plausible-looking path.

## Contract

- Selection is a pure function in `crates/nfctl/src/select.rs`:
  explicit flag, environment value, device list in; connstring or a typed
  selection error out.
- `CliError::NoReader` carries the actionable no-reader message and maps to
  `no_reader`/4. Unknown and ambiguous selections map to `usage`/2.
- The `--json`/`--jsonl` envelope `device` field is the resolved connstring,
  including when it came from the environment or auto-selection.
- `--device` rejects an empty value at parse time (exit 2).
- A non-Unicode `NFC_FOUNDRY_DEVICE` value is a usage error.
- `LIBNFC_DEVICE` is not read. `docs/cli.md` records the rename.

## Messages

- Zero readers: `no supported reader found; connect a PN532 UART or ACR122U
  reader, or pass --device <connstring>`.
- Several readers: `multiple readers found; pass --device <connstring> or set
  NFC_FOUNDRY_DEVICE: <connstring>, ...`.
- Unknown flag: `reader <target> is not in the device list: <choices>`.
- Unknown environment: `NFC_FOUNDRY_DEVICE names <target>, which is not in the
  device list: <choices>`.
- With no devices, the choices text asks the user to run `nfctl list`.

## GUI

- Refresh enumerates devices and selects index 0.
- Selection is applied by the refresh path itself, not only by the combo
  signal: a programmatic combo change emits no signal when the index is
  already 0, which would leave the service on a reader that is no longer first
  after a refresh.
- The reader panel shows the device name (transport: `PN532 UART`,
  `ACR122U USB`, `ACR122U PC/SC`) and the exact connstring on the line below.
  The caption follows the monitor snapshot, so a failed open still shows the
  resolved reader.
- Empty enumeration shows `No supported reader found` and disables the combo.
- No `NFC_FOUNDRY_DEVICE` handling in the GUI; it keeps its own selection
  state and preferences.

## Tests

- `select.rs` unit tests: flag precedence, environment selection, single
  reader, empty environment, zero readers, several readers with choices, and
  unknown values from each source.
- `error.rs`: `CliError::NoReader` classifies as `no_reader`/4.
- `nfctl` integration tests: environment selects a reader; `--device`
  overrides an invalid environment; an invalid environment target exits 2 with
  `usage`; several mock readers without a selection exit 2 and name a choice.
- GTK scenario: refresh keeps the first reader selected and shows its name and
  exact connstring. A unit test covers the transport label mapping.

## Non-goals

- `LIBNFC_DEVICE` compatibility.
- Selecting a connstring that is not currently enumerated.
- A GUI environment override or a new selection preference.
- Auto-selecting among several readers.
