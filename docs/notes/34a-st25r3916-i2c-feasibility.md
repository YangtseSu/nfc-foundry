# Note: 34.1 ST25R3916 I2C reader feasibility

Status: research only, 2026-11-15. No code, hardware, or capability claim.
Plan: [34-nfc-hardware-candidates.md](../plan/34-nfc-hardware-candidates.md).
Other hardware candidates:
[34b-nfc-hardware-candidates.md](34b-nfc-hardware-candidates.md).

## Verdict

**Defer.** For the current product scope (MIFARE Classic, Type 2/NDEF, Type 4 /
DESFire), a `st25r3916_i2c` backend adds a third path to the same cards and no
new product capability. It is technically feasible, but it is a raw NFC
frontend project, not a PN532-class driver: a new I2C transport, host Crypto1,
host ISO-DEP, and a full hardware-validation matrix.

Revisit only if ISO15693/NFC-V or card emulation becomes an accepted product
goal, or if a Type 2-only low-cost reader is a deliberate experiment.

## Device

- M5Stack Unit NFC, SKU U216, ST25R3916-AQWT.
- I2C slave `0x50`, 100/400 kHz; 3.3 V logic; 5 V input through an
  AP7343D-33 LDO; 2.2 k pull-ups to `VCC_3V3`; Grove 4-pin GND/5V/SDA/SCL.
- The ST25R3916 IRQ is not on the Grove header. A desktop driver must poll the
  interrupt/FIFO status.
- Desktop use requires a USB-to-I2C bridge; the unit has no USB.
- Identity: register `0x3F`, read opcode `0x7F`, expected `0x2A` (type `0x05`,
  revision `2`; revision bits may vary).
- The M5 Unit NFC also supports ISO15693, FeliCa, and target mode at the
  hardware level; those have no card family or product workflow in this
  repository.

## Chip vs module vs board

The limits are split across three layers:

| Layer | Limit |
|---|---|
| ST25R3916 chip | Host interface is SPI (up to 10 Mbit/s) or I2C (up to 3.4 Mbit/s); **no UART**. IRQ and MCU_CLK are chip pins. It is a raw frontend: no Crypto1 and no ISO-DEP block layer. |
| M5 Unit NFC | Only I2C is brought out (`0x50`, 100/400 kHz); SPI and IRQ are not on the Grove header. This is a module choice, not a chip limit. |
| UART boards | No ST25R3916 has a native UART. A UART/USB port requires an MCU bridge running firmware; the host then sees the MCU protocol, not the raw chip. |

Boards that use the chip:

| Board | Host interface | IRQ | MCU | Notes |
|---|---|---|---|---|
| M5 Unit NFC | I2C only | No | No | Grove; compact; cheapest path to the chip |
| Elechouse ST25R3916 module | SPI default, I2C via a pad bridge | Yes | No | Bare module; exposes IRQ; SPI is the default |
| Mikroe NFC 4 Click | SPI and I2C | Yes | No | mikroBUS Click; needs a Click host |
| X-NUCLEO-NFC06A1 | SPI | Yes | Requires a Nucleo | Arduino expansion; obsolete/out of production |
| ST25R3916-DISCO | Chip to STM32 via SPI or I2C; USB to PC | Yes | STM32L476 | Stock USB mode is the ST GUI protocol, not a generic UART |
| Custom STM32/ESP32 bridge | UART or USB CDC | Depends on wiring | Yes | Firmware defines the host protocol |

A SPI+IRQ board removes the M5 module's I2C-only/no-IRQ link risk, but it does
not remove the chip-level host Crypto1 and host ISO-DEP work. A UART board adds
an MCU protocol, not a chip UART; `nfc-foundry` would need a serial protocol
driver, not a raw ST25R3916 driver.

## Current reader contract

`crates/nfc-transport/src/lib.rs` defines the `Reader` trait:
`authenticate`, `read_block`, `write_block`, `read_page_quad`, `write_page`,
`password_auth`, `transceive_apdu`, raw bytes/bits, and recovery parity.

The existing drivers are command-level:

- PN532 UART uses `InDataExchange` for Classic authentication/read/write and
  for ISO-DEP after the PN532's auto-RATS
  (`crates/nfc-drivers/src/pn532_uart/protocol.rs`).
- ACR122U reuses the same PN532 command layer
  (`crates/nfc-drivers/src/acr122/protocol.rs`).
- Crypto1 lives in `crates/nfc-recover/src/crypto1.rs`, above the transport,
  and recovery uses `transceive_parity`.
- `crates/nfc-drivers` has no I2C dependency; `DriverRegistry` in
  `crates/nfc-drivers/src/lib.rs` only contains `pn532_uart`, `acr122_usb`, and
  `acr122_pcsc`.
- `docs/hardware.md` lists SPI/I2C and embedded Linux buses as unsupported by
  design; adding one requires an ADR, a driver boundary, and a hardware
  validation record.
- `docs/hardware-matrix.md` has no ST25R3916 row.

## Value

| Goal | Current readers | ST25R3916 increment | Verdict |
|---|---|---|---|
| Classic 1K/4K | PN532 UART, ACR122U | None; host Crypto1 required | Not worth it |
| Type 2 / NDEF | PN532 UART, ACR122U | None | Not worth it |
| Type 4 / DESFire | PN532 UART, ACR122U | None; host ISO-DEP required | Not worth it |
| Recovery parity | PN532 UART, ACR122U | New timing/parity validation | High risk, no proven gain |
| ISO15693 / NFC-V | None | Raw frontend supports it | Only with a Type 5/NDEF project |
| FeliCa / ISO14443B | PN532 hardware, no product workflow | No unique gain | Not worth it |
| Card emulation | None | Target-mode hardware | Only after an accepted Phase 31 direction |

The release already certifies Classic, Type 2, Type 4/DESFire, and recovery on
two readers. A third reader that cannot use the PN532 command layer does not
reduce that matrix; it adds rows.

## Feasibility

### Link

- Add a feature-gated `st25r3916_i2c` family, for example
  `st25r3916_i2c:/dev/i2c-N`.
- Use `rustix` `ioctl` on `/dev/i2c-N` (`I2C_RDWR`, `I2C_SLAVE`); no C shim.
- Prefer MCP2221A or FT260 (400 kHz, repeated start). CP2112 works for SMBus
  paths, but its raw multi-message write-read is limited by hardware revision.
- Enumerate adapters from `/sys/class/i2c-dev`; do not probe unrelated buses.
  Open validates `0x50` and `0x2A`.
- `/dev/i2c-N` is not a stable physical identity. Resolve the bridge's USB
  serial/sysfs path for `DeviceInfo::physical_id`; add an `i2c` group or udev
  permission path for packaging.
- Register reads need a repeated start: write `0x40 | reg`, repeated start,
  read the data. `I2C_RDWR` with a two-message transfer is the required link
  primitive.

### Protocol

- Register/FIFO access, oscillator/regulator/antenna initialization, ISO14443A
  REQA/WUPA, anticollision, SELECT, CRC-A, parity, timers, and FIFO handling.
- Type 2 READ/WRITE/PWD_AUTH/GET_VERSION can be layered on raw frames; the
  existing `nfc-type2` workflow is reusable.
- No IRQ: poll interrupt/FIFO status. Type 2 is the low-risk slice.
- Raw byte/bit and parity frames are feasible, but the recovery timing
  assumptions must be revalidated on this frontend.

### Classic

- The ST25R3916 has no Crypto1 engine; `authenticate`, `read_block`, and
  `write_block` must be host Crypto1.
- Crypto1 currently lives in `nfc-recover`, above the driver. A driver that
  uses it would invert the dependency; extract a shared lower-level crate or
  duplicate the code. Duplication is not acceptable under the layer rules.
- Gen1A/UFUID writes, recovery parity, fast select, expected response sizes,
  and every recovery attack need new hardware validation.

### Type 4

- `transceive_apdu` currently relies on PN532 auto-RATS and `InDataExchange`.
- The ST25R3916 needs host ISO-DEP: RATS, I-block, WTX, chaining, CRC, and
  timeouts.
- This is a separate large module; do not fold it into the link/protocol
  spike.

### Validation

- Every advertised operation needs a hardware record; the matrix is
  reader/card-specific.
- The Unit's no-IRQ polling and USB-I2C latency are the main timing risks.
- There is no in-tree Linux NFC driver for the ST25R3916. STSW-ST25R013 is a
  user-space RFAL over SPI; `pguyot/st25r391x` is an out-of-tree kernel driver.
  Neither is a dependency under `AGENTS.md`.
- `M5Unit-NFC` is C++/Arduino and is a behavioral reference for register setup
  and NFC-A, not a runtime dependency.

## Bounded spike, if accepted

- Feature-gated `st25r3916_i2c`; exact `/dev/i2c-N` connstring.
- Identity, ISO14443A select, and Type 2 READ/WRITE/read-back only.
- `ReaderCapabilities` advertises `type2_read`, `type2_write`, `raw_bytes`, and
  `raw_bits` only; Classic, APDU, recovery, and emulation stay absent and fail
  closed.
- Not in the release matrix until a hardware record exists.
- Exit criteria: identity `0x2A`, stable selection, Type 2 write/read-back on
  an owned tag, repeated open/close, measured latency, and no card corruption.
- Hardware: M5 Unit NFC plus MCP2221A/FT260 (preferred) or CP2112; 5 V to the
  unit, 3.3 V I2C, common ground.

## Decision

- Defer. Keep PN532 UART and ACR122U for the current scope.
- If ISO15693/NFC-V or target mode becomes a product goal, write a new ADR for
  the I2C transport and supported bridges, then a phase spec before code.
- If the only goal is a third reader, buy another PN532/ACR122U. If the only
  goal is to use the M5 Unit NFC, use it on the ESP32 side, not as a new
  `nfc-foundry` dependency.

## References

- M5 Unit NFC: https://docs.m5stack.com/en/unit/Unit_NFC
- M5 Unit NFC schematic:
  https://m5stack-doc.oss-cn-shenzhen.aliyuncs.com/1229/SCH_Unit_NFC_V0.1_2025_11_28_12_15_35.pdf
- ST25R3916 datasheet: https://www.st.com/resource/en/datasheet/st25r3916.pdf
- STSW-ST25R013: https://www.st.com/en/embedded-software/stsw-st25r013.html
- UM2548 (ST Linux RFAL): https://www.st.com/resource/en/user_manual/um2548-linux-driver-for-st25r3916st25r3916b-stmicroelectronics.pdf
- pguyot/st25r391x: https://github.com/pguyot/st25r391x
- AntorFr/st25r391x: https://github.com/AntorFr/st25r391x
- M5Unit-NFC: https://github.com/m5stack/M5Unit-NFC
