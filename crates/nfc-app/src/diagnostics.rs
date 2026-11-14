//! Deterministic self-check shared by `nfctl diag --json` and the GUI
//! diagnostics page.
//!
//! The report contains build and installation facts only: no card identity,
//! key material, or dump payload is read or serialized. Reader enumeration and
//! the firmware of an already opened reader are the only device facts.

use std::path::Path;

use nfc_recover::hardnested::tables;
use nfc_transport::ReaderCapabilities;
use serde::Serialize;

use crate::reader::ReaderSnapshot;

/// Product name used by both surfaces.
pub const APP_NAME: &str = "NFC Foundry";

/// One supported driver family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DriverInfo {
    pub name: String,
    pub conn_string_prefix: String,
}

impl DriverInfo {
    pub fn new(name: impl Into<String>, conn_string_prefix: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            conn_string_prefix: conn_string_prefix.into(),
        }
    }
}

/// Which operations the selected reader reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CapabilityReport {
    pub classic_read: bool,
    pub classic_write: bool,
    pub type2_read: bool,
    pub type2_write: bool,
    pub raw_bytes: bool,
    pub raw_bits: bool,
    pub manufacturer_write: bool,
    pub recovery_parity: bool,
}

impl From<ReaderCapabilities> for CapabilityReport {
    fn from(capabilities: ReaderCapabilities) -> Self {
        Self {
            classic_read: capabilities.classic_read,
            classic_write: capabilities.classic_write,
            type2_read: capabilities.type2_read,
            type2_write: capabilities.type2_write,
            raw_bytes: capabilities.raw_bytes,
            raw_bits: capabilities.raw_bits,
            manufacturer_write: capabilities.manufacturer_write,
            recovery_parity: capabilities.recovery_parity,
        }
    }
}

/// Build identity of this binary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub commit: String,
    pub build_date: String,
}

/// Host platform of this binary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlatformInfo {
    pub os: String,
    pub arch: String,
}

/// One enumerated reader.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReaderDevice {
    pub name: String,
    pub conn_string: String,
}

/// Reader enumeration plus the current selected-reader session facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReaderListing {
    pub available: bool,
    pub error: Option<String>,
    pub devices: Vec<ReaderDevice>,
    pub selected: Option<String>,
    pub status: String,
    pub firmware: Option<String>,
    pub capabilities: CapabilityReport,
}

/// Hardnested table lookup result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TablesStatus {
    pub available: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}

/// PC/SC availability reported by the surface that owns the driver list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PcscInfo {
    pub enabled: bool,
    pub available: bool,
    pub error: Option<String>,
    pub readers: Vec<String>,
}

/// Complete self-check document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SelfCheck {
    pub app: AppInfo,
    pub platform: PlatformInfo,
    pub drivers: Vec<DriverInfo>,
    pub readers: ReaderListing,
    pub pcsc: PcscInfo,
    pub hardnested_tables: TablesStatus,
}

impl SelfCheck {
    /// Builds the report. Reader enumeration is performed by the caller's
    /// service state; `snapshot` supplies the selected-reader facts.
    pub fn new(
        drivers: Vec<DriverInfo>,
        devices: Result<Vec<ReaderDevice>, String>,
        snapshot: &ReaderSnapshot,
        pcsc: PcscInfo,
        tables_dir: Option<&Path>,
    ) -> Self {
        let readers = match devices {
            Ok(devices) => ReaderListing {
                available: true,
                error: None,
                devices,
                selected: snapshot.selected.clone(),
                status: snapshot.status.label().to_owned(),
                firmware: snapshot.firmware.clone(),
                capabilities: snapshot.capabilities.into(),
            },
            Err(error) => ReaderListing {
                available: false,
                error: Some(error),
                devices: Vec::new(),
                selected: snapshot.selected.clone(),
                status: snapshot.status.label().to_owned(),
                firmware: snapshot.firmware.clone(),
                capabilities: snapshot.capabilities.into(),
            },
        };
        let hardnested_tables = match tables::resolve_dir(tables_dir) {
            Ok(path) => TablesStatus {
                available: true,
                path: Some(path.display().to_string()),
                error: None,
            },
            Err(error) => TablesStatus {
                available: false,
                path: None,
                error: Some(error.to_string()),
            },
        };
        Self {
            app: AppInfo {
                name: APP_NAME.to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                commit: option_env!("NFC_FOUNDRY_GIT_COMMIT")
                    .unwrap_or("unknown")
                    .to_owned(),
                build_date: option_env!("NFC_FOUNDRY_BUILD_DATE")
                    .unwrap_or("unknown")
                    .to_owned(),
            },
            platform: PlatformInfo {
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
            },
            drivers,
            readers,
            pcsc,
            hardnested_tables,
        }
    }

    /// Stable JSON document with a trailing newline.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::reader::ReaderStatus;

    use super::*;

    fn report() -> SelfCheck {
        SelfCheck::new(
            vec![
                DriverInfo::new("PN532 UART", "pn532_uart:"),
                DriverInfo::new("ACR122U USB", "acr122_usb:"),
            ],
            Ok(vec![ReaderDevice {
                name: "PN532 UART".to_owned(),
                conn_string: "pn532_uart:/dev/ttyUSB0".to_owned(),
            }]),
            &ReaderSnapshot {
                status: ReaderStatus::Polling,
                selected: Some("pn532_uart:/dev/ttyUSB0".to_owned()),
                card: None,
                capabilities: ReaderCapabilities {
                    classic_read: true,
                    classic_write: true,
                    type2_read: true,
                    type2_write: true,
                    raw_bytes: true,
                    raw_bits: true,
                    manufacturer_write: true,
                    recovery_parity: true,
                },
                firmware: Some("PN532 IC 32 · firmware 1.6 · support 07".to_owned()),
                error: None,
            },
            PcscInfo::default(),
            None,
        )
    }

    #[test]
    fn report_has_stable_fields_and_no_card_payload() {
        let text = report().to_json().expect("json");
        let value: Value = serde_json::from_str(&text).expect("parse");
        let object = value.as_object().expect("object");
        assert_eq!(
            object.keys().collect::<Vec<_>>(),
            [
                "app",
                "drivers",
                "hardnested_tables",
                "pcsc",
                "platform",
                "readers"
            ]
        );
        assert_eq!(value["app"]["name"], APP_NAME);
        assert_eq!(value["app"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(value["app"]["commit"].is_string());
        assert!(value["app"]["build_date"].is_string());
        assert_eq!(
            value["app"]
                .as_object()
                .expect("app")
                .keys()
                .collect::<Vec<_>>(),
            ["build_date", "commit", "name", "version"]
        );
        assert_eq!(value["platform"]["os"], std::env::consts::OS);
        assert_eq!(
            value["platform"]
                .as_object()
                .expect("platform")
                .keys()
                .collect::<Vec<_>>(),
            ["arch", "os"]
        );
        assert_eq!(value["drivers"][0]["conn_string_prefix"], "pn532_uart:");
        assert_eq!(
            value["drivers"][0]
                .as_object()
                .expect("driver")
                .keys()
                .collect::<Vec<_>>(),
            ["conn_string_prefix", "name"]
        );
        assert_eq!(value["readers"]["devices"][0]["name"], "PN532 UART");
        assert_eq!(value["readers"]["status"], "polling");
        assert_eq!(
            value["readers"]
                .as_object()
                .expect("readers")
                .keys()
                .collect::<Vec<_>>(),
            [
                "available",
                "capabilities",
                "devices",
                "error",
                "firmware",
                "selected",
                "status"
            ]
        );
        assert_eq!(
            value["readers"]["capabilities"]
                .as_object()
                .expect("capabilities")
                .keys()
                .collect::<Vec<_>>(),
            [
                "classic_read",
                "classic_write",
                "manufacturer_write",
                "raw_bits",
                "raw_bytes",
                "recovery_parity",
                "type2_read",
                "type2_write"
            ]
        );
        assert_eq!(value["readers"]["capabilities"]["classic_read"], true);
        assert!(value["hardnested_tables"]["available"].is_boolean());
        assert_eq!(
            value["hardnested_tables"]
                .as_object()
                .expect("tables")
                .keys()
                .collect::<Vec<_>>(),
            ["available", "error", "path"]
        );
        for forbidden in ["uid", "key", "dump", "block", "sector"] {
            assert!(
                !text.to_ascii_lowercase().contains(forbidden),
                "self-check leaked {forbidden:?}: {text}"
            );
        }
    }

    #[test]
    fn unreadable_tables_and_reader_failure_are_reported_not_fatal() {
        let report = SelfCheck::new(
            Vec::new(),
            Err("usb enumeration failed".to_owned()),
            &ReaderSnapshot {
                status: ReaderStatus::Error,
                selected: None,
                card: None,
                capabilities: ReaderCapabilities::default(),
                firmware: None,
                error: None,
            },
            PcscInfo::default(),
            Some(Path::new("/nonexistent/nfc-foundry-tables")),
        );
        let value = serde_json::to_value(&report).expect("json");
        assert_eq!(value["readers"]["available"], false);
        assert_eq!(value["readers"]["error"], "usb enumeration failed");
        assert_eq!(value["readers"]["status"], "error");
        let tables = &value["hardnested_tables"];
        assert!(tables["available"].is_boolean());
        if tables["available"] == false {
            assert!(tables["error"].is_string());
        } else {
            assert!(tables["path"].is_string());
        }
    }
}
