//! Reader selection policy for `nfctl`.
//!
//! The order is explicit flag, environment, the only enumerated reader, then a
//! failure. Selection stays a pure function of those inputs so the rules can
//! be tested without a transport or a process environment. The GUI keeps its
//! own auto-select-first policy; both surfaces share the device list and the
//! application service.

use nfc_transport::DeviceInfo;

/// Environment variable that names an exact reader connstring.
pub const DEVICE_ENV: &str = "NFC_FOUNDRY_DEVICE";

/// Where an explicit selection came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Flag,
    Environment,
}

/// Why a reader could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionError {
    /// No supported reader was enumerated and nothing was selected.
    NoReaders,
    /// Several readers were enumerated and nothing was selected.
    Ambiguous { candidates: Vec<String> },
    /// The selected connstring is not in the enumerated device list.
    Unknown {
        origin: Origin,
        target: String,
        candidates: Vec<String>,
    },
}

impl SelectionError {
    /// One actionable line for the CLI error envelope.
    pub fn message(&self) -> String {
        match self {
            Self::NoReaders => "no supported reader found; connect a PN532 UART or ACR122U reader, or pass --device <connstring>".to_owned(),
            Self::Ambiguous { candidates } => format!(
                "multiple readers found; pass --device <connstring> or set {DEVICE_ENV}: {}",
                candidate_list(candidates)
            ),
            Self::Unknown {
                origin: Origin::Flag,
                target,
                candidates,
            } => format!(
                "reader {target} is not in the device list: {}",
                candidate_list(candidates)
            ),
            Self::Unknown {
                origin: Origin::Environment,
                target,
                candidates,
            } => format!(
                "{DEVICE_ENV} names {target}, which is not in the device list: {}",
                candidate_list(candidates)
            ),
        }
    }
}

fn candidate_list(candidates: &[String]) -> String {
    if candidates.is_empty() {
        "no supported reader is connected; run `nfctl list`".to_owned()
    } else {
        candidates.join(", ")
    }
}

/// Resolve one reader connstring.
///
/// Only the first present source participates: an explicit flag suppresses the
/// environment, and a non-empty environment suppresses auto-selection. An
/// empty environment value counts as unset so a shell export with no value
/// does not break single-reader selection.
pub fn resolve_device(
    explicit: Option<&str>,
    environment: Option<&str>,
    devices: &[DeviceInfo],
) -> std::result::Result<String, SelectionError> {
    let candidates = devices
        .iter()
        .map(|device| device.conn_string.clone())
        .collect::<Vec<_>>();
    if let Some(target) = explicit.filter(|value| !value.is_empty()) {
        return select_from(Origin::Flag, target, &candidates);
    }
    if let Some(target) = environment.filter(|value| !value.is_empty()) {
        return select_from(Origin::Environment, target, &candidates);
    }
    if let [only] = candidates.as_slice() {
        return Ok(only.clone());
    }
    if candidates.is_empty() {
        return Err(SelectionError::NoReaders);
    }
    Err(SelectionError::Ambiguous { candidates })
}

fn select_from(
    origin: Origin,
    target: &str,
    candidates: &[String],
) -> std::result::Result<String, SelectionError> {
    if candidates.iter().any(|candidate| candidate == target) {
        Ok(target.to_owned())
    } else {
        Err(SelectionError::Unknown {
            origin,
            target: target.to_owned(),
            candidates: candidates.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices(conn_strings: &[&str]) -> Vec<DeviceInfo> {
        conn_strings
            .iter()
            .map(|conn_string| DeviceInfo::new("Reader", *conn_string))
            .collect()
    }

    #[test]
    fn the_flag_wins_over_the_environment_and_auto_selection() {
        let devices = devices(&["mock:one", "mock:two"]);
        assert_eq!(
            resolve_device(Some("mock:one"), Some("mock:missing"), &devices).expect("flag"),
            "mock:one"
        );
    }

    #[test]
    fn the_environment_selects_when_the_flag_is_absent() {
        let devices = devices(&["mock:one", "mock:two"]);
        assert_eq!(
            resolve_device(None, Some("mock:two"), &devices).expect("environment"),
            "mock:two"
        );
    }

    #[test]
    fn one_reader_is_selected_without_configuration() {
        let devices = devices(&["mock:only"]);
        assert_eq!(
            resolve_device(None, None, &devices).expect("single reader"),
            "mock:only"
        );
        assert_eq!(
            resolve_device(None, Some(""), &devices).expect("empty environment"),
            "mock:only"
        );
    }

    #[test]
    fn no_readers_is_a_hardware_failure() {
        assert_eq!(
            resolve_device(None, None, &[]),
            Err(SelectionError::NoReaders)
        );
        assert!(
            SelectionError::NoReaders
                .message()
                .contains("no supported reader")
        );
    }

    #[test]
    fn several_readers_require_a_selection_and_name_the_choices() {
        let devices = devices(&["mock:one", "mock:two"]);
        let error = resolve_device(None, None, &devices).expect_err("ambiguous");
        assert_eq!(
            error,
            SelectionError::Ambiguous {
                candidates: vec!["mock:one".to_owned(), "mock:two".to_owned()]
            }
        );
        let message = error.message();
        assert!(message.contains("--device"));
        assert!(message.contains(DEVICE_ENV));
        assert!(message.contains("mock:one, mock:two"));
    }

    #[test]
    fn an_unknown_source_names_the_variable_or_flag() {
        let devices = devices(&["mock:one"]);
        let flag = resolve_device(Some("mock:missing"), None, &devices).expect_err("unknown flag");
        assert_eq!(
            flag,
            SelectionError::Unknown {
                origin: Origin::Flag,
                target: "mock:missing".to_owned(),
                candidates: vec!["mock:one".to_owned()]
            }
        );
        let environment =
            resolve_device(None, Some("mock:missing"), &devices).expect_err("unknown environment");
        assert_eq!(
            environment,
            SelectionError::Unknown {
                origin: Origin::Environment,
                target: "mock:missing".to_owned(),
                candidates: vec!["mock:one".to_owned()]
            }
        );
        assert!(environment.message().contains(DEVICE_ENV));
    }

    #[test]
    fn an_unknown_selection_with_no_readers_still_names_the_missing_target() {
        let error = resolve_device(Some("mock:missing"), None, &[]).expect_err("unknown");
        assert_eq!(
            error,
            SelectionError::Unknown {
                origin: Origin::Flag,
                target: "mock:missing".to_owned(),
                candidates: Vec::new()
            }
        );
        assert!(error.message().contains("run `nfctl list`"));
    }
}
