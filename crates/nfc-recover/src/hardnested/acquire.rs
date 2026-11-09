// SPDX-FileCopyrightText: Proxmark3 contributors
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardnested nonce acquisition over the transport parity exchange.
//!
//! PM3 acquires one nested nonce per select/authenticate cycle and uses a fast
//! ISO/IEC 14443A SELECT between cycles so the card's PRNG keeps advancing.
//! This module performs the same read-only sequence: fast-select the card,
//! authenticate the known sector with the known key, then send one nested
//! authentication frame to the target sector and keep the encrypted nonce and
//! parity.

use nfc_core::{CancelToken, NfcError, Result};
use nfc_transport::Reader;

use super::filter::Filter;
use super::tables::Tables;
use crate::acquire::Collector;

#[derive(Clone, Copy, Debug, Default)]
pub struct AcquisitionProgress {
    pub received: usize,
    pub distinct_nonces: usize,
    pub first_bytes: usize,
    pub expected_states: f64,
}

pub struct Acquisition {
    pub cuid: u32,
    pub received: usize,
}

fn leave_collector(collector: &mut Option<Collector>, reader: &mut dyn Reader) {
    if let Some(mut active) = collector.take() {
        let _ = active.leave(reader);
    }
}

/// Collects nonces until the PM3 stopping criterion or `max_nonces`.
pub fn collect<'a>(
    reader: &mut dyn Reader,
    tables: &'a Tables,
    request: &super::HardnestedRequest,
    cancel: &CancelToken,
    max_nonces: usize,
    progress: &mut dyn FnMut(AcquisitionProgress),
) -> Result<(Acquisition, Filter<'a>)> {
    let mut filter = Filter::new(tables);
    let mut received = 0usize;
    let mut cuid = None;
    let mut last_encrypted = None;
    let mut repeated = 0u8;
    let mut consecutive_failures = 0u8;
    let mut collector: Option<Collector> = None;
    let mut last_sample = std::time::Instant::now();
    let mut sample_period_ms = 100.0;

    while received < max_nonces {
        if let Err(error) = cancel.check() {
            leave_collector(&mut collector, reader);
            return Err(error);
        }
        if let Some(cuid_value) = cuid
            && filter.nonces.all_first_bytes_seen()
            && received.is_multiple_of(8)
        {
            filter.sample_period_ms = sample_period_ms;
            let (expected, stop) = filter.shrink_key_space();
            progress(AcquisitionProgress {
                received,
                distinct_nonces: filter.nonces.num_acquired,
                first_bytes: filter.nonces.first_byte_num,
                expected_states: expected,
            });
            if stop {
                if let Some(mut active) = collector.take() {
                    let _ = active.leave(reader);
                }
                return Ok((
                    Acquisition {
                        cuid: cuid_value,
                        received,
                    },
                    filter,
                ));
            }
        }

        if collector.is_none() {
            match Collector::begin(
                reader,
                request.known_block,
                request.known_key_type,
                request.known_key,
                request.target_block,
                request.target_key_type,
            ) {
                Ok(active) => collector = Some(active),
                Err(error) if retryable(&error) => {
                    consecutive_failures += 1;
                    if consecutive_failures.is_multiple_of(4) {
                        reopen(reader)?;
                    }
                    if consecutive_failures >= 16 {
                        leave_collector(&mut collector, reader);
                        return Err(error);
                    }
                    continue;
                }
                Err(error) => {
                    leave_collector(&mut collector, reader);
                    return Err(error);
                }
            }
        }

        let active = collector.as_mut().expect("collector was just created");
        match active.next_nonce(reader) {
            Ok(nonce) => {
                consecutive_failures = 0;
                let uid = active.uid();
                if let Some(existing) = cuid {
                    if existing != uid {
                        leave_collector(&mut collector, reader);
                        return Err(NfcError::CardChanged);
                    }
                } else {
                    cuid = Some(uid);
                }

                if last_encrypted == Some(nonce.tag_nonce_encrypted) {
                    repeated += 1;
                } else {
                    repeated = 0;
                    last_encrypted = Some(nonce.tag_nonce_encrypted);
                }
                if repeated >= 5 {
                    leave_collector(&mut collector, reader);
                    return Err(NfcError::Unsupported(
                        "the card returns a static encrypted nonce; this is the FM11RF08S static-encrypted nonce profile, whose recovery needs the reader-scheduled FM11RF08S acquisition",
                    ));
                }

                filter.add_nonce(nonce.tag_nonce_encrypted, nonce.parity >> 4);
                received += 1;
                let now = std::time::Instant::now();
                sample_period_ms = now.duration_since(last_sample).as_millis() as f64;
                last_sample = now;
                if (filter.stage & super::filter::CHECK_2ND_BYTES == 0
                    || received.is_multiple_of(4))
                    && let Err(error) = filter.update()
                {
                    leave_collector(&mut collector, reader);
                    return Err(error);
                }
            }
            Err(error) if retryable(&error) || matches!(error, NfcError::AuthenticationFailed) => {
                consecutive_failures += 1;
                leave_collector(&mut collector, reader);
                if retryable(&error) && consecutive_failures.is_multiple_of(4) {
                    reopen(reader)?;
                }
                if consecutive_failures >= 16 {
                    leave_collector(&mut collector, reader);
                    return Err(error);
                }
            }
            Err(error) => {
                leave_collector(&mut collector, reader);
                return Err(error);
            }
        }
    }

    leave_collector(&mut collector, reader);
    filter.update()?;
    let cuid = cuid.ok_or(NfcError::NoCard)?;
    filter.sample_period_ms = sample_period_ms;
    let (expected, _) = filter.shrink_key_space();
    progress(AcquisitionProgress {
        received,
        distinct_nonces: filter.nonces.num_acquired,
        first_bytes: filter.nonces.first_byte_num,
        expected_states: expected,
    });
    Ok((Acquisition { cuid, received }, filter))
}

fn retryable(error: &NfcError) -> bool {
    matches!(error, NfcError::Timeout(_) | NfcError::Io(_))
}

fn reopen(reader: &mut dyn Reader) -> Result<()> {
    let conn_string = reader.device().conn_string.clone();
    let _ = reader.close();
    reader.open(&conn_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_errors_are_transport_failures() {
        assert!(retryable(&NfcError::Timeout("x".to_owned())));
        assert!(retryable(&NfcError::Io("x".to_owned())));
        assert!(!retryable(&NfcError::AuthenticationFailed));
    }
}
