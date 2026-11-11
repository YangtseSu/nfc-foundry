//! Reader lifecycle and card-presence monitoring.
//!
//! The monitor is stepped by the host instead of owning a thread: `nfc-app`
//! enforces the poll interval and the exclusive operation lease, so an
//! operation always runs before the next poll. A single missed card answer is
//! tolerated; `removal_misses` consecutive misses report the card as removed.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use nfc_core::{CardInfo, NfcError};
use nfc_transport::ReaderCapabilities;

/// Poll spacing while the reader answers normally.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(400);
/// Consecutive no-card answers before a card is reported removed.
pub const DEFAULT_REMOVAL_MISSES: u8 = 2;
/// First reconnect delay after a transport failure.
pub const DEFAULT_RECONNECT_INITIAL: Duration = Duration::from_millis(200);
/// Upper bound for the exponential reconnect delay.
pub const DEFAULT_RECONNECT_MAX: Duration = Duration::from_secs(5);

/// Lifecycle state shown by the reader panel and reported to hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderStatus {
    /// The reader is being opened for the first time or after a release.
    Connecting,
    /// The reader answers card polls; `ReaderSnapshot::card` says whether a
    /// card is present.
    Polling,
    /// An exclusive application operation holds the reader.
    Busy,
    /// A transport failure closed the reader; a reconnect is scheduled.
    Recovering,
    /// A non-retryable reader error stopped polling.
    Error,
}

impl ReaderStatus {
    /// Stable lower-case label used by `scan --watch` and the self-check JSON.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Polling => "polling",
            Self::Busy => "busy",
            Self::Recovering => "recovering",
            Self::Error => "error",
        }
    }
}

/// Poll cadence, removal tolerance, and reconnect backoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReaderMonitorConfig {
    pub poll_interval: Duration,
    pub removal_misses: u8,
    pub reconnect_initial: Duration,
    pub reconnect_max: Duration,
}

impl Default for ReaderMonitorConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            removal_misses: DEFAULT_REMOVAL_MISSES,
            reconnect_initial: DEFAULT_RECONNECT_INITIAL,
            reconnect_max: DEFAULT_RECONNECT_MAX,
        }
    }
}

impl ReaderMonitorConfig {
    /// Normalize values that would stall or divide the monitor.
    pub fn normalized(mut self) -> Self {
        self.removal_misses = self.removal_misses.max(1);
        self.reconnect_initial = self.reconnect_initial.max(Duration::from_millis(1));
        self.reconnect_max = self
            .reconnect_max
            .max(self.reconnect_initial)
            .max(Duration::from_millis(1));
        self
    }
}

/// One card or connection transition observed by the monitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderEvent {
    CardAppeared(CardInfo),
    CardRemoved(CardInfo),
    StatusChanged(ReaderStatus),
}

/// Current reader facts. `capabilities` stays empty until the reader opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderSnapshot {
    pub status: ReaderStatus,
    pub selected: Option<String>,
    pub card: Option<CardInfo>,
    pub capabilities: ReaderCapabilities,
    /// Diagnostic firmware text for the last opened reader.
    pub firmware: Option<String>,
    pub error: Option<String>,
}

/// One monitor step: the current facts plus the transitions since the last
/// step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderUpdate {
    pub snapshot: ReaderSnapshot,
    pub events: Vec<ReaderEvent>,
}

/// Internal monitor bookkeeping. Reader I/O stays in `DeviceManager`; this
/// type owns the state machine and the event queue.
pub(crate) struct MonitorState {
    config: ReaderMonitorConfig,
    lifecycle: ReaderStatus,
    card: Option<CardInfo>,
    misses: u8,
    error: Option<String>,
    capabilities: ReaderCapabilities,
    firmware: Option<String>,
    events: VecDeque<ReaderEvent>,
    reported: ReaderStatus,
    next_poll: Option<Instant>,
    reconnect_at: Option<Instant>,
    backoff: Duration,
}

impl MonitorState {
    pub(crate) fn new() -> Self {
        Self {
            config: ReaderMonitorConfig::default(),
            lifecycle: ReaderStatus::Connecting,
            card: None,
            misses: 0,
            error: None,
            capabilities: ReaderCapabilities::default(),
            firmware: None,
            events: VecDeque::new(),
            reported: ReaderStatus::Connecting,
            next_poll: None,
            reconnect_at: None,
            backoff: ReaderMonitorConfig::default().reconnect_initial,
        }
    }

    pub(crate) fn config(&self) -> ReaderMonitorConfig {
        self.config
    }

    pub(crate) fn set_config(&mut self, config: ReaderMonitorConfig) {
        self.config = config.normalized();
        self.backoff = self.backoff.min(self.config.reconnect_max);
    }

    pub(crate) fn capabilities(&self) -> ReaderCapabilities {
        self.capabilities
    }

    pub(crate) fn lifecycle(&self) -> ReaderStatus {
        self.lifecycle
    }

    /// Start a new selection: drop the previous card and poll immediately.
    pub(crate) fn reset(&mut self, now: Instant) {
        self.forget_card();
        self.lifecycle = ReaderStatus::Connecting;
        self.misses = 0;
        self.error = None;
        self.capabilities = ReaderCapabilities::default();
        self.firmware = None;
        self.next_poll = Some(now);
        self.reconnect_at = None;
        self.backoff = self.config.reconnect_initial;
        self.sync(false);
    }

    /// Drop card state after an explicit reader release or device removal.
    pub(crate) fn forget_card(&mut self) {
        self.misses = 0;
        self.next_poll = None;
        self.reconnect_at = None;
        self.lifecycle = ReaderStatus::Connecting;
        if let Some(card) = self.card.take() {
            self.events.push_back(ReaderEvent::CardRemoved(card));
        }
    }

    pub(crate) fn opened(&mut self, capabilities: ReaderCapabilities, firmware: Option<String>) {
        self.capabilities = capabilities;
        self.firmware = firmware;
    }

    pub(crate) fn begin_operation(&mut self) {
        self.sync(true);
    }

    pub(crate) fn end_operation(&mut self) {
        self.sync(false);
    }

    /// Emit a status transition when the externally visible status changed.
    pub(crate) fn sync(&mut self, busy: bool) {
        let status = if busy {
            ReaderStatus::Busy
        } else {
            self.lifecycle
        };
        if status != self.reported {
            self.reported = status;
            self.events.push_back(ReaderEvent::StatusChanged(status));
        }
    }

    /// Whether a poll may run now. An `Error` state waits for an explicit
    /// selection or refresh instead of retrying a known-bad reader.
    pub(crate) fn poll_due(&self, now: Instant) -> bool {
        if self.lifecycle == ReaderStatus::Error {
            return false;
        }
        if let Some(at) = self.reconnect_at
            && now < at
        {
            return false;
        }
        if let Some(at) = self.next_poll
            && now < at
        {
            return false;
        }
        true
    }

    pub(crate) fn card_seen(&mut self, card: CardInfo, now: Instant) {
        self.lifecycle = ReaderStatus::Polling;
        self.error = None;
        self.misses = 0;
        self.reconnect_at = None;
        self.next_poll = Some(now + self.config.poll_interval);
        self.backoff = self.config.reconnect_initial;
        match &self.card {
            Some(previous) if !previous.same_identity(&card) => {
                self.events
                    .push_back(ReaderEvent::CardRemoved(previous.clone()));
                self.events
                    .push_back(ReaderEvent::CardAppeared(card.clone()));
            }
            None => self
                .events
                .push_back(ReaderEvent::CardAppeared(card.clone())),
            Some(_) => {}
        }
        self.card = Some(card);
    }

    pub(crate) fn card_missing(&mut self, now: Instant) {
        self.lifecycle = ReaderStatus::Polling;
        self.error = None;
        self.next_poll = Some(now + self.config.poll_interval);
        self.misses = self.misses.saturating_add(1);
        if self.misses >= self.config.removal_misses
            && let Some(card) = self.card.take()
        {
            self.events.push_back(ReaderEvent::CardRemoved(card));
        }
    }

    /// A transport failure: close the reader, keep the last card for now, and
    /// schedule an exponential reconnect.
    pub(crate) fn reader_failure(&mut self, error: &NfcError, now: Instant) {
        self.lifecycle = ReaderStatus::Recovering;
        self.error = Some(error.to_string());
        self.next_poll = None;
        self.reconnect_at = Some(now + self.backoff);
        self.backoff = self
            .backoff
            .saturating_mul(2)
            .min(self.config.reconnect_max);
    }

    /// A non-retryable error stops polling until the reader is selected again.
    pub(crate) fn fatal(&mut self, error: &NfcError) {
        self.lifecycle = ReaderStatus::Error;
        self.error = Some(error.to_string());
        self.next_poll = None;
        self.reconnect_at = None;
    }

    pub(crate) fn update(&mut self, busy: bool, selected: Option<String>) -> ReaderUpdate {
        ReaderUpdate {
            snapshot: ReaderSnapshot {
                status: if busy {
                    ReaderStatus::Busy
                } else {
                    self.lifecycle
                },
                selected,
                card: self.card.clone(),
                capabilities: self.capabilities,
                firmware: self.firmware.clone(),
                error: self.error.clone(),
            },
            events: self.events.drain(..).collect(),
        }
    }

    pub(crate) fn snapshot(&self, busy: bool, selected: Option<String>) -> ReaderSnapshot {
        ReaderSnapshot {
            status: if busy {
                ReaderStatus::Busy
            } else {
                self.lifecycle
            },
            selected,
            card: self.card.clone(),
            capabilities: self.capabilities,
            firmware: self.firmware.clone(),
            error: self.error.clone(),
        }
    }
}

/// Transport failures a reconnect can recover from. Card absence is handled
/// separately so it never closes the reader.
pub(crate) fn is_transient(error: &NfcError) -> bool {
    matches!(error, NfcError::Timeout(_) | NfcError::Io(_))
}

#[cfg(test)]
mod tests {
    use nfc_core::CardType;

    use super::*;

    fn card(uid: u8) -> CardInfo {
        CardInfo::new(vec![uid; 4], [0x00, 0x04], 0x08, CardType::MifareClassic1K)
    }

    fn monitor() -> MonitorState {
        let mut monitor = MonitorState::new();
        monitor.set_config(
            ReaderMonitorConfig {
                poll_interval: Duration::ZERO,
                ..ReaderMonitorConfig::default()
            }
            .normalized(),
        );
        monitor
    }

    #[test]
    fn removal_needs_the_configured_number_of_misses() {
        let now = Instant::now();
        let mut monitor = monitor();
        monitor.reset(now);
        monitor.sync(false);
        monitor.card_seen(card(1), now);
        monitor.sync(false);
        let update = monitor.update(false, None);
        assert!(matches!(
            update.events.as_slice(),
            [
                ReaderEvent::CardAppeared(_),
                ReaderEvent::StatusChanged(ReaderStatus::Polling),
            ]
        ));

        monitor.card_missing(now);
        let update = monitor.update(false, None);
        assert_eq!(update.snapshot.card, Some(card(1)));
        assert!(
            !update
                .events
                .iter()
                .any(|event| matches!(event, ReaderEvent::CardRemoved(_)))
        );

        monitor.card_missing(now);
        let update = monitor.update(false, None);
        assert_eq!(update.snapshot.card, None);
        assert!(matches!(
            update.events.as_slice(),
            [ReaderEvent::CardRemoved(removed)] if removed.uid == vec![1; 4]
        ));
    }

    #[test]
    fn busy_is_visible_between_operations() {
        let now = Instant::now();
        let mut monitor = monitor();
        monitor.reset(now);
        monitor.card_seen(card(1), now);
        monitor.sync(false);
        monitor.update(false, None);

        monitor.begin_operation();
        let update = monitor.update(true, None);
        assert_eq!(update.snapshot.status, ReaderStatus::Busy);
        assert_eq!(
            update.events,
            vec![ReaderEvent::StatusChanged(ReaderStatus::Busy)]
        );

        monitor.end_operation();
        let update = monitor.update(false, None);
        assert_eq!(update.snapshot.status, ReaderStatus::Polling);
        assert_eq!(
            update.events,
            vec![ReaderEvent::StatusChanged(ReaderStatus::Polling)]
        );
    }

    #[test]
    fn reconnect_backoff_doubles_per_transient_failure() {
        let now = Instant::now();
        let mut monitor = monitor();
        monitor.reset(now);
        monitor.card_seen(card(1), now);
        monitor.reader_failure(&NfcError::Timeout("no answer".to_owned()), now);
        assert!(!monitor.poll_due(now + Duration::from_millis(199)));
        assert!(monitor.poll_due(now + Duration::from_millis(200)));
        monitor.reader_failure(
            &NfcError::Timeout("still no answer".to_owned()),
            now + Duration::from_millis(200),
        );
        assert!(!monitor.poll_due(now + Duration::from_millis(599)));
        assert!(monitor.poll_due(now + Duration::from_millis(600)));
    }
}
