# Note: 34.2 NFC hardware candidates

Status: research only, 2026-11-15. No code, hardware, or capability claim.
Plan: [34-nfc-hardware-candidates.md](../plan/34-nfc-hardware-candidates.md).
Related: [34a-st25r3916-i2c-feasibility.md](34a-st25r3916-i2c-feasibility.md).

## Verdict

For the current scope (MIFARE Classic, Type 2/NDEF, Type 4 / DESFire), the
PN532 UART and ACR122U already cover the release. Another reader is worth
adding only if it adds a card family or mode, replaces aging hardware, or
preserves a capability the new reader would otherwise lose.

Priority:

1. **ACR1552U**: best new-capability hardware (ISO15693/NFC-V, Type 1/Topaz,
   broader card coverage, card emulation).
2. **ACR1252U**: modern ACR122U replacement with P2P and card emulation; no
   ISO15693.
3. **PN532 I2C/SPI**: low-cost link variant of the existing PN532 command
   layer; same capabilities, no new card family.
4. **PN7160**: long-term NCI controller; I2C/SPI plus IRQ, no USB; large Rust
   NCI work.
5. **Raw frontends** (ST25R3916, PN5180, RC663, TRF7970A): custom embedded or
   RF research only.
6. **MFRC522**: partial reader; not worth it.
7. **Generic PC/SC or Sony RC-S380**: out of scope or niche; not first.

## Candidate comparison

| Candidate | New capability | Host link | Raw recovery | Verdict |
|---|---|---|---|---|
| ACR1552U | ISO15693/Type 5, Type 1/Topaz, Picopass, FeliCa, 848 kbps, 64 KB APDU, card emulation | USB CCID/PC/SC | Likely no | Best new-capability hardware |
| ACR1252U | Modern ACR122U, NFC Forum certified, P2P, card emulation, 424 kbps | USB CCID/PC/SC | Likely no | Best ACR122U replacement / emulation candidate |
| PN532 I2C/SPI | Same as PN532 UART | I2C or SPI, plus IRQ | Preserved | Cheap link variant; low product value |
| PN7160 | NCI, Type 1–5, card emulation, P2P | I2C or SPI, plus IRQ | No | Long-term standard controller; bridge/MCU + Rust NCI |
| ST25R3916 / PN5180 / RC663 / TRF7970A | Raw NFC/NFC-V | SPI/I2C | Host-owned | Custom embedded/RF research only |
| MFRC522 | Classic/ISO14443A only | SPI | Partial | Not worth it |
| Generic PC/SC / RC-S380 | FeliCa/PCSC niche | USB PC/SC | No | Out of scope or not first |

## ACR1552U

The strongest candidate when the product expands beyond the current card
families:

- USB CCID/PC/SC; Linux PC/SC support; no USB-I2C bridge.
- ISO14443A/B, MIFARE Classic, FeliCa, ISO18092, ISO15693, Type 1/Topaz,
  Picopass.
- Reader/writer and card emulation; 848 kbps; extended APDU up to 64 KB.
- The host sends card native commands or APDUs; the reader firmware owns RF,
  Crypto1, and ISO-DEP. No host Crypto1/ISO-DEP port is required.
- The existing `pcsc` crate and reader enumeration plumbing can be reused, but
  the ACR122 pseudo-APDU layer is not guaranteed compatible. A new specific
  PC/SC command layer is required.

Costs and limits:

- Specific PC/SC-only reader; this is an exception to the current “generic
  PC/SC out of scope” rule, like the ACR122U PC/SC path. An ADR and hardware
  record are required.
- ISO15693/Type 5, Type 1/Topaz, Type 3, and Type 4B need their own card-family
  adapters, GUI/CLI surfaces, and hardware records. The driver alone does not
  deliver those features.
- Card emulation still needs the Phase 31 target-mode transport boundary.
- The ACR1552U likely does not expose raw Crypto1 parity/nonce commands. Keep
  PN532/ACR122U for recovery; `ReaderCapabilities.recovery_parity` should stay
  false until a raw path is validated.
- ACR1552U PC/SC and ACR122U PC/SC can coexist under `pcscd`. The ACR122U
  direct USB path requires `pcscd` stopped, so it cannot run at the same time
  as a PC/SC ACR1552U; use the ACR122U PC/SC path or PN532 UART for recovery.
- `ApduCapabilities.max_command` can be larger than the current 247-byte cap;
  Type 4/DESFire workflows need a review before advertising the larger limit.

For ISO15693/NFC-V, the ACR1552U is a better nfc-foundry path than the
ST25R3916: the reader firmware handles the RF/protocol work, while the
ST25R3916 is a raw frontend that requires host Crypto1/ISO-DEP.

## ACR1252U

- USB CCID/PC/SC; NFC Forum certified.
- ISO14443A/B, MIFARE, FeliCa, ISO18092; reader/writer, P2P, card emulation.
- 424 kbps; no ISO15693.
- New specific PC/SC command layer; the ACR122 pseudo-APDU is not guaranteed
  compatible.
- Likely no raw recovery parity; keep PN532/ACR122U for recovery.
- Best if the goal is a modern ACR122U replacement, P2P, or card emulation;
  the ACR1552U is better for new card families.

## PN532 I2C/SPI

- The PN532 supports HSU/UART, I2C, and SPI; the command payloads are shared
  with the existing `pn53x` layer.
- A new link/frame implementation is required, plus an IRQ/ready line. A
  desktop needs a USB-I2C/SPI bridge.
- Preserves the existing Classic/Type 2/raw/recovery capabilities; no new card
  family.
- Only worth it for a module that lacks UART or for a faster link. Otherwise
  the validated PN532 UART path is simpler.

## PN7160 and the USB question

The PN7160 is an NCI 2.0 controller, not a USB device:

- PN7160A1: I2C. PN7160B1: SPI. No UART, no native USB.
- IRQ is part of the host interface; Ven/GPIO control is also required.
- Supports Type 1–5, card emulation (Type 3/4), and P2P; the controller owns
  RF, Crypto1/MIFARE handling, and ISO-DEP. No host Crypto1/ISO-DEP port is
  required.
- The PN7160 does not expose raw Crypto1 parity; recovery remains on
  PN532/ACR122U.

A desktop therefore still needs one of:

- USB-I2C or USB-SPI bridge plus IRQ/Ven GPIO;
- an MCU/USB board that forwards NCI;
- a host with native I2C/SPI, such as a Raspberry Pi or i.MX board.

NXP EVKs and third-party USB modules often expose USB through a host MCU or
bridge; the PN7160 chip link remains I2C/SPI. Confirm the specific kit before
assuming a USB reader.

Software: NXP's Linux stack is the C libnfc-nci, which is not a dependency
under `AGENTS.md`. A pure Rust NCI implementation plus IRQ/Ven handling is a
large project. Using the Linux kernel `nxp-nci` driver and NFC netlink is an
alternative, but it is a different transport architecture and does not provide
the raw recovery path.

The PN7160 is a long-term NCI candidate, not the next desktop reader. For a
desktop ISO15693/card-emulation path, the ACR1552U is simpler.

## Raw frontends

ST25R3916, PN5180, RC663, and TRF7970A are raw NFC frontends. They can support
NFC-V or custom RF work, but the host owns Crypto1, ISO-DEP, framing, parity,
timing, and recovery validation. They make sense for custom embedded hardware
or RF research, not for a desktop workbench with complete readers. The
ST25R3916 analysis is in [34a](34a-st25r3916-i2c-feasibility.md).

## Not candidates

- **MFRC522**: cheap, but limited to Classic/ISO14443A and partial Type 2;
  adds matrix burden without new product value.
- **Generic PC/SC-only readers**: out of scope; only specific, named readers
  with an accepted ADR.
- **Sony RC-S380 / PaSoRi**: PC/SC FeliCa niche; the ACR1552U/ACR1252U cover
  FeliCa with a broader feature set.
- **Proxmark3**: not a reader backend; the project exchanges dumps and key
  files only.

## Recovery split

The new complete readers are reader/writer/NDEF/DESFire candidates, not
recovery replacements. Their PC/SC or NCI command sets likely do not expose
raw Crypto1 parity/nonces. Keep PN532 UART and ACR122U for Classic recovery,
and use `ReaderCapabilities` to fail closed on every reader that has not
validated `recovery_parity`.

## Recommendation

- Current scope: no new hardware; keep PN532 UART and ACR122U.
- Expand to ISO15693/Type 5 or Type 1/Topaz: ACR1552U first; add a specific
  PC/SC driver, new card-family adapters, and hardware records.
- Replace ACR122U or pursue P2P/card emulation: ACR1252U, after the Phase 31
  target-mode boundary is accepted.
- Hardware flexibility only: PN532 I2C/SPI link variant; low priority.
- Long-term NCI: PN7160 later, with a bridge/MCU and a Rust NCI layer.
- Custom embedded/RF research: raw frontends only.

## References

- ACR1552U: https://www.acs.com.hk/en/products/575/acr1552u-usb-nfc-reader-iv
- ACR1252U: https://www.acs.com.hk/en/products/342/acr1252u-usb-nfc-reader-iii-nfc-forum-certified-reader
- PN532 datasheet: https://www.nxp.com/docs/en/nxp/data-sheets/PN532_C1.pdf
- PN7160: https://www.nxp.com/products/PN7160
- PN7160 NCI / Linux: https://github.com/NXPNFCLinux/nxpnfc
- ST25R3916 note: [34a-st25r3916-i2c-feasibility.md](34a-st25r3916-i2c-feasibility.md)
