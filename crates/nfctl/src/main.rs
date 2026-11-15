use std::fs;
use std::io::{self, IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use clap::{Parser, Subcommand};
use nfc_app::amiibo::{
    AmiiboFileRequest, AmiiboReadRequest, AmiiboSourceFormat, AmiiboWriteExecution,
    AmiiboWriteRequest,
};
use nfc_app::classic::{
    AuthenticatedKey, BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicNdefFormat,
    ClassicNdefRead, ClassicNdefWrite, ClassicRead, ClassicSealExecution, ClassicSealPlan,
    ClassicSealPreflight, ClassicUidExecution, ClassicUidPlan, ClassicUidPreflight,
    ClassicWriteExecution, ClassicWritePlan, ClassicWritePreflight, KeyEntry, KeyFile, KeyKind,
    KeySource, KeyStore, KeyTypes, RestoreSelection, RestoreUidState, adopt_sidecar, common_keys,
    format_uid, parse_block_list, parse_uid, write_sidecar,
};
use nfc_app::desfire::{
    AccessRight, AccessRights, ApplicationKeyType, AuthKeyType, CommunicationMode,
    DesfireAppCreateRequest, DesfireAppDeleteRequest, DesfireFileCreateRequest,
    DesfireFileDeleteRequest, DesfireFileInspectionRequest, DesfireFileSettingsRequest,
    DesfireFileSpec, DesfireFormatPiccRequest, DesfireFreeMemoryRequest, DesfireKeyChangeRequest,
    DesfireKeySettingsRequest, DesfireNdefDeleteRequest, DesfireNdefFormatRequest,
    DesfireNdefReplaceRequest, DesfireReadResult, DesfireRecordOperation, DesfireRecordRequest,
    DesfireUidRequest, DesfireValueOperation, DesfireValueRequest, DesfireVerifyKeyRequest,
    FileSettings, NDEF_APPLICATION_AID, NDEF_CAPABILITY_CONTAINER, NDEF_CC_FILE, NDEF_CC_SIZE,
    NDEF_DATA_FILE, NDEF_DATA_SIZE, NDEF_DF_NAME, app_create_text, app_delete_text, card_uid_text,
    file_create_text, file_delete_text, file_settings_plan_text, file_settings_text,
    free_memory_text, key_change_text, key_settings_text, key_verification_text,
    read_protection_text, record_text, value_text,
};
use nfc_app::type2::{
    Type2Auth, Type2Format, Type2Lock, Type2NdefWrite, Type2PageWrite, Type2Password,
    Type2ReadRequest, Type2Restore, Type2UidExecution, parse_uid_hex,
};
use nfc_app::{
    AmiiboIdentity, AmiiboKeys, ReaderEvent, ReaderStatus, Service, classic_health, load_key_parts,
    load_keys,
};
use nfc_classic::{DumpFormat, Layout, bcc_for_uid, diff, exchange};
use nfc_classic_ndef::{
    ClassicNdefInfo, Mad, MadVersion, NdefState as ClassicNdefState, format_ndef_area,
};
use nfc_core::{
    BLOCK_SIZE, CancelToken, CardInfo, CardType, Key, KeyType, NfcError, PAGE_SIZE, Result,
};
use nfc_desfire::mock::{MockDesfire, MockDesfireTransport};
use nfc_drivers::DriverRegistry;
use nfc_ndef::{NdefMessage, NdefRecord, TextRecord, UriRecord, WifiRecord};
use nfc_pboc::mock::{MockEmvTransport, MockTUnionTransport};
use nfc_recover::auto::RecoveryStrategy;
use nfc_recover::{RecoveryProgress, RecoveryRequest};
use nfc_transport::mock::{MockTransport, MockType4Tag};
use nfc_transport::{DeviceInfo, Reader, Transport};
use nfc_type2::{
    Configuration, LockBits, NdefState, RestoreOptions, Type2Image, exchange as type2,
};
use nfc_type4::CcTlv;
use serde_json::{Value, json};

mod error;
mod output;
mod runtime;
mod select;

use error::CliError;
use output::{Mode, Output, compact_hex};
use select::{SelectionError, resolve_device};

type CliResult<T> = std::result::Result<T, CliError>;

/// Human result line; muted outside human mode.
macro_rules! say {
    ($out:expr, $($arg:tt)*) => {
        $out.line(format_args!($($arg)*))
    };
}

/// Human result fragment without a newline; muted outside human mode.
macro_rules! say_inline {
    ($out:expr, $($arg:tt)*) => {
        $out.inline(format_args!($($arg)*))
    };
}

/// Warning: stderr in human mode, envelope `warnings` in JSON modes.
macro_rules! warn {
    ($out:expr, $($arg:tt)*) => {
        $out.warn(format_args!($($arg)*))
    };
}

/// Diagnostic note: always stderr, never the envelope.
macro_rules! note {
    ($out:expr, $($arg:tt)*) => {
        $out.note(format_args!($($arg)*))
    };
}

#[derive(Debug, Parser)]
#[command(
    name = "nfctl",
    version,
    about = "NFC Foundry command-line workbench",
    long_about = None
)]
struct Cli {
    /// Use the scripted in-memory readers instead of hardware.
    #[arg(long, global = true)]
    mock: bool,
    /// Emit exactly one JSON document on stdout.
    #[arg(long, global = true, conflicts_with = "jsonl")]
    json: bool,
    /// Emit compact JSON Lines events and a final result on stdout.
    #[arg(long, global = true)]
    jsonl: bool,
    /// Total operation deadline, e.g. 500ms, 30s, 5m, or 1h.
    #[arg(long, global = true, value_name = "DURATION", value_parser = parse_timeout)]
    timeout: Option<Duration>,
    /// Reader, driver, and retry detail on stderr.
    #[arg(long, global = true)]
    verbose: bool,
    /// Print the plan and stop without writing. Mutation commands only.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Exact reader connstring. Overrides NFC_FOUNDRY_DEVICE and auto-selection.
    #[arg(
        long,
        global = true,
        value_name = "CONNSTRING",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    device: Option<String>,
    /// Candidate Classic key as 12 hexadecimal digits. May be repeated.
    #[arg(long, global = true, value_name = "HEX")]
    key: Vec<String>,
    /// Text key file with one 12-digit key per line. May be repeated.
    #[arg(long, global = true, value_name = "PATH")]
    key_file: Vec<PathBuf>,
    /// Classic key store JSON.
    #[arg(long, global = true, value_name = "PATH")]
    key_store: Option<PathBuf>,
    /// Directory for pre-write backups.
    #[arg(long, global = true, value_name = "DIR")]
    backup_dir: Option<PathBuf>,
    /// Do not try the built-in default keys.
    #[arg(long, global = true)]
    no_common_keys: bool,
    /// Hardnested table directory. Defaults to the installed resource path.
    #[arg(long, global = true, value_name = "DIR")]
    hardnested_tables: Option<PathBuf>,
    /// Hardnested acquisition cap; zero uses the engine default.
    #[arg(long, global = true, value_name = "N", default_value_t = 0)]
    max_nonces: usize,
    /// Confirm destructive actions without prompting.
    #[arg(long, global = true)]
    yes: bool,
    /// PWD_AUTH password for a protected Type 2 tag, as 8 hex digits.
    #[arg(
        long = "type2-password",
        visible_alias = "current-pwd",
        global = true,
        value_name = "HEX",
        env = "NFC_FOUNDRY_TYPE2_PASSWORD",
        hide_env_values = true
    )]
    type2_password: Option<String>,
    /// Expected PACK returned by PWD_AUTH, as 4 hex digits.
    #[arg(
        long = "type2-pack",
        visible_alias = "current-pack",
        global = true,
        value_name = "HEX",
        env = "NFC_FOUNDRY_TYPE2_PACK",
        hide_env_values = true
    )]
    type2_pack: Option<String>,
    /// Amiibo combined 160-byte key file. Never logged or saved.
    #[arg(long = "amiibo-keys", global = true, value_name = "PATH")]
    amiibo_keys: Option<PathBuf>,
    /// Amiibo 80-byte data key file (unfixed-info.bin).
    #[arg(long = "amiibo-data-key", global = true, value_name = "PATH")]
    amiibo_data_key: Option<PathBuf>,
    /// Amiibo 80-byte tag key file (locked-secret.bin).
    #[arg(long = "amiibo-tag-key", global = true, value_name = "PATH")]
    amiibo_tag_key: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Subcommand)]
enum Command {
    /// Enumerate supported readers.
    List,
    /// Select a reader and report the current card.
    Scan {
        /// Keep watching for card changes.
        #[arg(long)]
        watch: bool,
    },
    /// Card information.
    Card {
        #[command(subcommand)]
        command: CardCommand,
    },
    /// MIFARE Classic operations.
    Classic {
        #[command(subcommand)]
        command: ClassicCommand,
    },
    /// Type 2 / NTAG operations.
    Type2 {
        #[command(subcommand)]
        command: Type2Command,
    },
    /// China T-Union transit card read-only operations.
    Tunion {
        #[command(subcommand)]
        command: TunionCommand,
    },
    /// EMV payment-card read-only operations.
    Emv {
        #[command(subcommand)]
        command: EmvCommand,
    },
    /// Amiibo / NTAG215 decrypt, encrypt, and controlled write operations.
    Amiibo {
        #[command(subcommand)]
        command: AmiiboCommand,
    },
    /// DESFire inventory, file read/write, management, keys, and NDEF.
    Desfire {
        #[command(subcommand)]
        command: Box<DesfireCommand>,
    },
    /// NDEF operations.
    Ndef {
        #[command(subcommand)]
        command: NdefCommand,
    },
    /// Native MIFARE Classic key recovery.
    Recover {
        #[command(subcommand)]
        command: RecoveryCommand,
    },
    /// Raw PN532 command console (developer diagnostic).
    #[cfg(feature = "developer-tools")]
    Pn532 {
        #[command(subcommand)]
        command: Pn532Command,
    },
    /// Send one raw ISO 7816-4 APDU (developer diagnostic).
    #[cfg(feature = "developer-tools")]
    Apdu {
        /// Complete command APDU as hexadecimal bytes.
        #[arg(long, value_name = "HEX")]
        raw: String,
    },
    /// Run the self-check.
    Diag,
}

#[derive(Clone, Debug, Subcommand)]
enum CardCommand {
    Info,
    /// Details and health for a saved Classic image, without a reader.
    Health {
        #[arg(long, short, value_name = "PATH")]
        input: PathBuf,
        /// auto, raw, proxmark3, mct, or project.
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum ClassicCommand {
    /// Read one block, authenticating its sector.
    Read {
        #[arg(long, value_name = "BLOCK")]
        block: usize,
    },
    /// Read the card into a dump file.
    Dump {
        #[arg(long, short, value_name = "PATH")]
        output: PathBuf,
        /// auto, raw, proxmark3, mct, or project.
        #[arg(long, default_value = "auto", value_name = "FORMAT")]
        format: String,
        /// Do not write the `.nfcx.json` metadata sidecar.
        #[arg(long)]
        no_sidecar: bool,
    },
    /// Write one block after a backup and read-back verification.
    Write {
        #[arg(long, value_name = "BLOCK")]
        block: usize,
        /// 32 hexadecimal digits or `@path`.
        #[arg(long, value_name = "HEX")]
        data: String,
        /// Required to replace a sector trailer.
        #[arg(long)]
        confirm_trailer: bool,
    },
    /// Change the UID of a magic MIFARE Classic 1K card by writing block 0.
    Uid {
        /// Target four-byte UID, e.g. 04A2248A.
        #[arg(long, value_name = "UID")]
        uid: String,
        /// Required: UID writes are an Advanced action.
        #[arg(long)]
        advanced: bool,
    },
    /// Permanently disable the Gen1A backdoor on a UFUID card (irreversible).
    Seal {
        /// Required: sealing is an Advanced action.
        #[arg(long)]
        advanced: bool,
    },
    /// Restore a dump after a backup.
    Restore {
        #[arg(long, short, value_name = "PATH")]
        input: PathBuf,
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Continue after a failed block instead of stopping.
        #[arg(long)]
        continue_on_error: bool,
        /// Explicit block numbers, e.g. `1,4,5-7`.
        #[arg(long, value_name = "BLOCKS", conflicts_with = "changes_only")]
        blocks: Option<String>,
        /// Write only blocks that differ from the loaded image baseline.
        #[arg(long)]
        changes_only: bool,
        /// After a full restore, write this four-byte UID to block 0.
        #[arg(long, value_name = "UID")]
        uid: Option<String>,
        /// Required with `--uid`: UID writes are an Advanced action.
        #[arg(long)]
        advanced: bool,
    },
    /// Try candidate keys against every sector.
    Keyscan {
        /// Save discovered keys to this store (`.dic` for a per-card key file).
        #[arg(long, value_name = "PATH")]
        save: Option<PathBuf>,
        /// Name stored in a `.dic` key file header.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// A, B, or AB.
        #[arg(long, default_value = "AB", value_name = "TYPES")]
        types: String,
    },
    /// Inspect or update the key store.
    Keys {
        #[command(subcommand)]
        command: KeysCommand,
    },
    /// Compare two dump files without a reader.
    Diff {
        #[arg(value_name = "BEFORE")]
        before: PathBuf,
        #[arg(value_name = "AFTER")]
        after: PathBuf,
    },
    /// Classic NDEF mapping, read, write, and format.
    Ndef {
        #[command(subcommand)]
        command: ClassicNdefCommand,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum ClassicNdefCommand {
    /// Show the MAD, NFC sectors, and NDEF state.
    Info,
    /// Print the NDEF records; optionally write the raw message to a file.
    Read {
        #[arg(long, short, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Write a raw NDEF file, a Text record, or a URI record.
    Write {
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        #[arg(long, value_name = "TEXT")]
        text: Option<String>,
        #[arg(long, value_name = "URI")]
        uri: Option<String>,
        #[arg(long, default_value = "en", value_name = "LANG")]
        language: String,
    },
    /// Format the MAD and every NFC sector after confirmation.
    Format {
        /// Write the read-only GPB and access bits.
        #[arg(long)]
        read_only: bool,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum KeysCommand {
    /// List stored keys.
    List {
        /// Also print every built-in common key.
        #[arg(long)]
        builtin: bool,
    },
    /// Import a text key file as wildcard keys.
    Import {
        #[arg(value_name = "PATH")]
        file: PathBuf,
    },
    /// Add one key.
    Add {
        #[arg(value_name = "HEX")]
        hex: String,
        #[arg(long, value_name = "SECTOR")]
        sector: Option<usize>,
        #[arg(long, default_value = "any", value_name = "A|B|any")]
        key_type: String,
    },
    /// Delete one stored key entry.
    Delete {
        #[arg(value_name = "HEX")]
        hex: String,
        #[arg(long, value_name = "SECTOR")]
        sector: Option<usize>,
        #[arg(long, default_value = "any", value_name = "A|B|any")]
        key_type: String,
    },
    /// Export the store as a Proxmark3 `.dic` or JSON file.
    Export {
        #[arg(long, default_value = "dic", value_name = "dic|json")]
        format: String,
        /// Include the built-in dictionary; the store itself is not changed.
        #[arg(long)]
        all_candidates: bool,
        /// Write to this path instead of standard output.
        #[arg(long, short, value_name = "PATH")]
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum Type2Command {
    /// Show the page layout, CC, locks, configuration, and NDEF state.
    Info,
    /// Print every page, or one page with `--page`.
    Read {
        #[arg(long, value_name = "PAGE")]
        page: Option<u8>,
    },
    /// Write one page after a backup and read-back verification.
    Write {
        #[arg(long, value_name = "PAGE")]
        page: u8,
        /// Eight hexadecimal digits.
        #[arg(long, value_name = "HEX")]
        data: String,
        /// Allow a lock, configuration, password, or capability-container page.
        #[arg(long)]
        advanced: bool,
    },
    /// Save a project JSON image.
    Dump {
        #[arg(long, short, value_name = "PATH")]
        output: PathBuf,
    },
    /// Restore a project JSON image after confirmation.
    Restore {
        #[arg(long, short, value_name = "PATH")]
        input: PathBuf,
        #[arg(long)]
        include_configuration: bool,
        #[arg(long)]
        include_capability_container: bool,
        #[arg(long)]
        include_locks: bool,
    },
    /// Set one-way lock bits for pages such as `4,8,16-20`.
    Lock {
        #[arg(long, value_name = "PAGES")]
        pages: String,
    },
    /// Program PWD/PACK, AUTH0, and ACCESS.
    Password {
        #[arg(long, value_name = "PAGE")]
        auth0: u8,
        #[arg(long, value_name = "HEX")]
        access: String,
        #[arg(long, value_name = "HEX")]
        pwd: String,
        #[arg(long, value_name = "HEX")]
        pack: String,
    },
    /// Change the UID of a UID-writable Type 2 clone.
    Uid {
        /// Target seven-byte UID, e.g. 0400E3C2000003.
        #[arg(long, value_name = "UID")]
        uid: String,
        /// Required: UID writes are an Advanced action.
        #[arg(long)]
        advanced: bool,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum TunionCommand {
    /// Read the T-Union application, balance, and record files.
    Read,
}

#[derive(Clone, Debug, Subcommand)]
enum EmvCommand {
    /// Read the payment application, masked PAN, expiry, and records.
    Read,
}

#[derive(Clone, Debug, Subcommand)]
enum AmiiboCommand {
    /// Read the selected NTAG215 and optionally decrypt it.
    Read {
        /// Save the raw 540-byte dump here.
        #[arg(long, short, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Verify and decrypt a raw dump into its plaintext form.
    Decrypt {
        #[arg(long, short, value_name = "PATH")]
        input: PathBuf,
        #[arg(long, short, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Encrypt and sign a plaintext dump.
    Encrypt {
        #[arg(long, short, value_name = "PATH")]
        input: PathBuf,
        #[arg(long, short, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Re-sign a source dump for the selected tag and write it.
    Write {
        #[arg(long, short, value_name = "PATH")]
        input: PathBuf,
        /// Treat the input as a plaintext dump instead of a raw dump.
        #[arg(long)]
        plain: bool,
        /// Required: Amiibo writes are an Advanced action.
        #[arg(long)]
        advanced: bool,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum DesfireCommand {
    /// Read the card version and application/file inventory.
    Info,
    /// Create the AN11004 NDEF Tag Application on an unformatted card.
    FormatNdef {
        /// PICC master key: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: String,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// PICC key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Required: formatting changes the card structure.
        #[arg(long)]
        advanced: bool,
    },
    /// Delete the NDEF Tag Application created by `format-ndef`.
    DeleteNdef {
        /// PICC master key: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: String,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// PICC key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Required: deleting an application changes the card structure.
        #[arg(long)]
        advanced: bool,
    },
    /// Replace an existing AN11004 NDEF Tag Application.
    ReplaceNdef {
        /// PICC master key: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: String,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// PICC key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Required: replacing changes the card structure.
        #[arg(long)]
        advanced: bool,
    },
    /// Format the PICC and erase every application.
    FormatPicc {
        /// Current PICC master key: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: String,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// PICC key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Required: formatting erases every application and resets the PICC key.
        #[arg(long)]
        advanced: bool,
    },
    /// Read the Type 4 NDEF Tag Application and print its records.
    NdefRead {
        /// Write the raw NDEF message to this file.
        #[arg(long, short, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Authenticate one application key without reading file data.
    VerifyKey {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: String,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
    },
    /// Read the card UID after authentication.
    Uid {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: String,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
    },
    /// Read one data, value, or record file, optionally authenticating first.
    Read {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// File ID, e.g. 01.
        #[arg(long, value_name = "HEX")]
        file: String,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Data-file byte offset.
        #[arg(long, default_value_t = 0, value_name = "N")]
        offset: u32,
        /// Data-file read length; defaults to the rest of the file.
        #[arg(long, value_name = "N")]
        length: Option<u32>,
        /// Record offset for a record file; 0 is the most recent record.
        #[arg(long, default_value_t = 0, value_name = "N")]
        start_record: u32,
        /// Record count for a record file; defaults to the rest.
        #[arg(long, value_name = "N")]
        record_count: Option<u32>,
    },
    /// Write a Standard or Backup data file and verify it.
    Write {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// File ID, e.g. 01.
        #[arg(long, value_name = "HEX")]
        file: String,
        /// Data as hexadecimal digits.
        #[arg(long, value_name = "HEX")]
        data: Option<String>,
        /// File whose bytes are written.
        #[arg(long, value_name = "PATH")]
        data_file: Option<PathBuf>,
        /// Data-file byte offset.
        #[arg(long, default_value_t = 0, value_name = "N")]
        offset: u32,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
    },
    /// Credit, debit, or limited-credit a value file and verify the new value.
    Value {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// File ID, e.g. 01.
        #[arg(long, value_name = "HEX")]
        file: String,
        /// Credit this amount.
        #[arg(long, value_name = "N")]
        credit: Option<u32>,
        /// Debit this amount.
        #[arg(long, value_name = "N")]
        debit: Option<u32>,
        /// Add limited credit of this amount.
        #[arg(long, value_name = "N")]
        limited_credit: Option<u32>,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// AppTransactionMACKey used to verify the committed transaction MAC.
        #[arg(long, value_name = "HEX")]
        tmac_key: Option<String>,
    },
    /// Append, update, or clear records and verify every record.
    Record {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// File ID, e.g. 01.
        #[arg(long, value_name = "HEX")]
        file: String,
        /// Append one complete record from --data or --data-file.
        #[arg(long)]
        append: bool,
        /// Update a range of an existing record.
        #[arg(long)]
        update: bool,
        /// Clear every record.
        #[arg(long)]
        clear: bool,
        /// Record number for --update; 0 is the newest record.
        #[arg(long, default_value_t = 0, value_name = "N")]
        record: u32,
        /// Byte offset inside the record for --update.
        #[arg(long, default_value_t = 0, value_name = "N")]
        offset: u32,
        /// Data as hexadecimal digits.
        #[arg(long, value_name = "HEX")]
        data: Option<String>,
        /// File whose bytes are written.
        #[arg(long, value_name = "PATH")]
        data_file: Option<PathBuf>,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// AppTransactionMACKey used to verify the committed transaction MAC.
        #[arg(long, value_name = "HEX")]
        tmac_key: Option<String>,
    },
    /// Read the free EEPROM bytes of the PICC or one application.
    Memory {
        /// Application ID, e.g. A1B2C3. Defaults to the PICC.
        #[arg(long, value_name = "HEX")]
        aid: Option<String>,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
    },
    /// Create an application at the PICC level.
    AppCreate {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// PICC master key: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// PICC key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Number of application keys, 1..=14.
        #[arg(long, default_value_t = 1, value_name = "N")]
        keys: u8,
        /// Application key family: des, 3k3des, or aes.
        #[arg(long, default_value = "des", value_name = "TYPE")]
        app_key_type: String,
    },
    /// Delete an application after exporting its manifest.
    AppDelete {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// PICC master key: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// PICC key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Required: deleting an application changes the card structure.
        #[arg(long)]
        advanced: bool,
        /// Manifest destination; defaults to the backups directory.
        #[arg(long, value_name = "PATH")]
        manifest: Option<PathBuf>,
    },
    /// Create one file in an application.
    FileCreate {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// File ID, e.g. 01.
        #[arg(long, value_name = "HEX")]
        file: String,
        /// standard, backup, value, linear-record, cyclic-record, or transaction-mac.
        #[arg(long = "type", value_name = "TYPE")]
        kind: String,
        /// plain, maced, or enciphered.
        #[arg(long, default_value = "plain", value_name = "MODE")]
        communication: String,
        /// Raw 2-byte little-endian access word, e.g. 0000.
        #[arg(long, value_name = "HEX")]
        access_rights: Option<String>,
        /// Read right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        read_access: Option<String>,
        /// Write right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        write_access: Option<String>,
        /// Read/write right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        read_write_access: Option<String>,
        /// Change right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        change_access: Option<String>,
        /// Optional ISO 7816-4 file ID.
        #[arg(long, value_name = "HEX")]
        iso_file: Option<String>,
        /// Standard or Backup content size in bytes.
        #[arg(long, value_name = "N")]
        size: Option<u32>,
        /// Value file lower limit.
        #[arg(long, value_name = "N")]
        lower_limit: Option<i32>,
        /// Value file upper limit.
        #[arg(long, value_name = "N")]
        upper_limit: Option<i32>,
        /// Value file initial value.
        #[arg(long, value_name = "N")]
        value: Option<i32>,
        /// Value flags: 01 limited credit, 02 free GetValue.
        #[arg(long, value_name = "HEX")]
        flags: Option<String>,
        /// Record size in bytes.
        #[arg(long, value_name = "N")]
        record_size: Option<u32>,
        /// Maximum record count.
        #[arg(long, value_name = "N")]
        max_records: Option<u32>,
        /// Transaction MAC AES-128 key, 32 hex digits.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_TMAC_KEY",
            hide_env_values = true
        )]
        tmackey: Option<String>,
        /// Transaction MAC key version, 0..=255.
        #[arg(long, default_value_t = 0, value_name = "N")]
        tmackey_version: u8,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
    },
    /// Delete one file after exporting its manifest.
    FileDelete {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// File ID, e.g. 01.
        #[arg(long, value_name = "HEX")]
        file: String,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Required: deleting a file changes the card structure.
        #[arg(long)]
        advanced: bool,
        /// Manifest destination; defaults to the backups directory.
        #[arg(long, value_name = "PATH")]
        manifest: Option<PathBuf>,
    },
    /// Change a file's communication mode and access rights.
    FileSettings {
        /// Application ID, e.g. A1B2C3.
        #[arg(long, value_name = "HEX")]
        aid: String,
        /// File ID, e.g. 01.
        #[arg(long, value_name = "HEX")]
        file: String,
        /// New mode: plain, maced, or enciphered. Standard/Backup only.
        #[arg(long, value_name = "MODE")]
        communication: Option<String>,
        /// New raw 2-byte little-endian access word.
        #[arg(long, value_name = "HEX")]
        access_rights: Option<String>,
        /// New read right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        read_access: Option<String>,
        /// New write right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        write_access: Option<String>,
        /// New read/write right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        read_write_access: Option<String>,
        /// New change right: key0..key13, free, or denied.
        #[arg(long, value_name = "RIGHT")]
        change_access: Option<String>,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// Required: changing access or communication settings is a
        /// configuration action.
        #[arg(long)]
        advanced: bool,
    },
    /// Read the key settings and key versions, or change the KeySettings byte.
    KeySettings {
        /// Application ID, e.g. A1B2C3. Defaults to the PICC.
        #[arg(long, value_name = "HEX")]
        aid: Option<String>,
        /// Key for the selected family: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: Option<String>,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Application key number, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        key_number: u8,
        /// New KeySettings byte, e.g. 0F. Ommitted reads only.
        #[arg(long, value_name = "HEX")]
        new_settings: Option<String>,
        /// Required with --new-settings.
        #[arg(long)]
        advanced: bool,
    },
    /// Change one application key or the PICC master key.
    ChangeKey {
        /// Application ID, e.g. A1B2C3. Defaults to the PICC master key.
        #[arg(long, value_name = "HEX")]
        aid: Option<String>,
        /// Key used for the current session: DES 8, AES/2K3DES 16, 3K3DES 24.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_KEY",
            hide_env_values = true
        )]
        key: String,
        /// Explicit key family: des, 2k3des, 3k3des, aes, or ev2.
        #[arg(long, value_name = "TYPE")]
        key_type: Option<String>,
        /// Key number used for the session, 0..=14.
        #[arg(long, default_value_t = 0, value_name = "N")]
        auth_key_number: u8,
        /// Key number to change, 0..=14.
        #[arg(long, value_name = "N")]
        key_number: u8,
        /// New key value: DES 8, AES/2K3DES 16, 3K3DES 24 bytes.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_NEW_KEY",
            hide_env_values = true
        )]
        new_key: String,
        /// New key version, 0..=255; embedded for DES/3K3DES.
        #[arg(long, default_value_t = 0, value_name = "N")]
        new_key_version: u8,
        /// Current value of the target key when it differs from --key.
        #[arg(
            long,
            value_name = "HEX",
            env = "NFC_FOUNDRY_DESFIRE_CURRENT_KEY",
            hide_env_values = true
        )]
        current_key: Option<String>,
        /// Write the new key to this file (mode 0600) before the change.
        #[arg(long, value_name = "PATH")]
        export: Option<PathBuf>,
        /// Confirm the new key is recorded outside this session when it is
        /// not exported.
        #[arg(long)]
        acknowledge_unrecorded: bool,
        /// Required: changing a key changes how the card authenticates.
        #[arg(long)]
        advanced: bool,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum NdefCommand {
    /// Print the NDEF records; optionally write the raw message to a file.
    Read {
        #[arg(long, short, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Write a raw NDEF file, a Text record, or a URI record.
    Write {
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        #[arg(long, value_name = "TEXT")]
        text: Option<String>,
        #[arg(long, value_name = "URI")]
        uri: Option<String>,
        /// IANA language code for `--text`.
        #[arg(long, default_value = "en", value_name = "LANG")]
        language: String,
    },
    /// Format the NDEF data area after confirmation.
    Format {
        /// Allow the one-way capability-container write on an unformatted
        /// Ultralight EV1. This permanently writes OTP page 3.
        #[arg(long)]
        otp: bool,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum RecoveryCommand {
    /// Scan every sector, seed with darkside when needed, then run nested and
    /// hardnested and verify the result with a full read.
    Auto,
    /// Card-only darkside recovery.
    Darkside {
        #[arg(long, default_value = "0", value_name = "BLOCK")]
        target_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        target_type: String,
    },
    /// Nested recovery using a known key from --key, --key-file, or --key-store.
    Nested {
        #[arg(long, default_value = "4", value_name = "BLOCK")]
        known_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        known_type: String,
        #[arg(long, default_value = "0", value_name = "BLOCK")]
        target_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        target_type: String,
    },
    /// Static-nested recovery using a known key.
    #[command(alias = "staticnested")]
    StaticNested {
        #[arg(long, default_value = "4", value_name = "BLOCK")]
        known_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        known_type: String,
        #[arg(long, default_value = "0", value_name = "BLOCK")]
        target_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        target_type: String,
    },
    /// Hardnested recovery using a known key.
    #[command(alias = "hardnested")]
    HardNested {
        #[arg(long, default_value = "4", value_name = "BLOCK")]
        known_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        known_type: String,
        #[arg(long, default_value = "0", value_name = "BLOCK")]
        target_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        target_type: String,
    },
    /// Detect the FM11RF08S static encrypted nonce profile.
    Fm11rf08s {
        #[arg(long, default_value = "4", value_name = "BLOCK")]
        known_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        known_type: String,
        #[arg(long, default_value = "A", value_name = "A|B")]
        target_type: String,
    },
}

#[cfg(feature = "developer-tools")]
#[derive(Clone, Debug, Subcommand)]
enum Pn532Command {
    /// Interactive raw command console.
    Shell,
}

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let mode = runtime::prescan_mode(&args);
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => return usage_exit(error, mode),
    };
    ExitCode::from(run(cli))
}

/// Help and version print their own text; every other parse failure is a
/// usage error (exit 2) and honors the requested machine mode.
fn usage_exit(error: clap::Error, mode: Mode) -> ExitCode {
    use clap::error::ErrorKind;
    match error.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        | ErrorKind::DisplayVersion => {
            let _ = error.print();
            ExitCode::SUCCESS
        }
        _ => match mode {
            Mode::Human => {
                let _ = error.print();
                ExitCode::from(2)
            }
            Mode::Json | Mode::JsonLines => ExitCode::from(
                Output::new(mode, false).fail(&CliError::Usage(error.to_string()), false),
            ),
        },
    }
}

fn parse_timeout(value: &str) -> std::result::Result<Duration, String> {
    runtime::parse_duration(value)
}

/// One CLI invocation: global flags, cancellation, and the output mode.
struct Run<'a> {
    cli: &'a Cli,
    out: &'a mut Output,
    cancel: CancelToken,
}

fn run(cli: Cli) -> u8 {
    let mode = if cli.json {
        Mode::Json
    } else if cli.jsonl {
        Mode::JsonLines
    } else {
        Mode::Human
    };
    let mut out = Output::new(mode, cli.verbose);
    let cancel = CancelToken::new();
    runtime::install_signal_handler(&cancel);
    let deadline = cli
        .timeout
        .map(|timeout| runtime::start_deadline(&cancel, timeout));

    let command = cli.command.clone();
    out.set_command(command.name());
    let mut service = build_service(cli.mock);
    let outcome = {
        let mut run = Run {
            cli: &cli,
            out: &mut out,
            cancel: cancel.clone(),
        };
        if cli.dry_run && !command.supports_dry_run() {
            Err(CliError::Usage(format!(
                "--dry-run is not supported by `{}`",
                command.name()
            )))
        } else {
            dispatch(&mut run, &mut service, command)
        }
    };
    match outcome {
        Ok(result) => out.finish(result),
        Err(error) => {
            let deadline_expired = deadline
                .as_ref()
                .is_some_and(|expired| expired.load(Ordering::Acquire));
            out.fail(&error, deadline_expired)
        }
    }
}

fn dispatch(run: &mut Run<'_>, service: &mut Service, command: Command) -> CliResult<Value> {
    match command {
        Command::List => list(run, service),
        Command::Scan { watch } => {
            prepare(run, service)?;
            if watch {
                run.out.set_command("scan.watch");
                watch_card(run, service)?;
                Ok(json!({}))
            } else {
                run.out.set_command("scan");
                let card = service.scan_with(run.cancel.clone())?;
                run.out.set_card(&card);
                print_card(run.out, &card);
                Ok(json!({ "card": output::card_facts(&card) }))
            }
        }
        Command::Card { command } => match command {
            CardCommand::Info => {
                run.out.set_command("card.info");
                prepare(run, service)?;
                let card = service.card_info_with(run.cancel.clone())?;
                run.out.set_card(&card);
                print_card(run.out, &card);
                Ok(json!({ "card": output::card_facts(&card) }))
            }
            CardCommand::Health { input, format } => {
                run.out.set_command("card.health");
                card_health(run, &input, format.as_deref())
            }
        },
        Command::Classic { command } => classic(run, service, command),
        Command::Type2 { command } => type2(run, service, command),
        Command::Tunion { command } => tunion(run, service, command),
        Command::Emv { command } => emv(run, service, command),
        Command::Amiibo { command } => amiibo(run, service, command),
        Command::Desfire { command } => desfire(run, service, *command),
        Command::Ndef { command } => ndef(run, service, command),
        Command::Recover { command } => recovery(run, service, command),
        #[cfg(feature = "developer-tools")]
        Command::Pn532 { command } => match command {
            Pn532Command::Shell => pn532_shell(run, service),
        },
        #[cfg(feature = "developer-tools")]
        Command::Apdu { raw } => apdu_raw(run, service, &raw),
        Command::Diag => {
            run.out.set_command("diag");
            diag(run, service)
        }
    }
}

impl Command {
    /// Stable command name used by the envelope and usage errors.
    fn name(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Scan { watch: true } => "scan.watch",
            Self::Scan { watch: false } => "scan",
            Self::Card { command } => match command {
                CardCommand::Info => "card.info",
                CardCommand::Health { .. } => "card.health",
            },
            Self::Classic { command } => match command {
                ClassicCommand::Read { .. } => "classic.read",
                ClassicCommand::Dump { .. } => "classic.dump",
                ClassicCommand::Write { .. } => "classic.write",
                ClassicCommand::Uid { .. } => "classic.uid",
                ClassicCommand::Seal { .. } => "classic.seal",
                ClassicCommand::Restore { .. } => "classic.restore",
                ClassicCommand::Keyscan { .. } => "classic.keyscan",
                ClassicCommand::Keys { command } => match command {
                    KeysCommand::List { .. } => "classic.keys.list",
                    KeysCommand::Import { .. } => "classic.keys.import",
                    KeysCommand::Add { .. } => "classic.keys.add",
                    KeysCommand::Delete { .. } => "classic.keys.delete",
                    KeysCommand::Export { .. } => "classic.keys.export",
                },
                ClassicCommand::Diff { .. } => "classic.diff",
                ClassicCommand::Ndef { command } => match command {
                    ClassicNdefCommand::Info => "classic.ndef.info",
                    ClassicNdefCommand::Read { .. } => "classic.ndef.read",
                    ClassicNdefCommand::Write { .. } => "classic.ndef.write",
                    ClassicNdefCommand::Format { .. } => "classic.ndef.format",
                },
            },
            Self::Amiibo { command } => match command {
                AmiiboCommand::Read { .. } => "amiibo.read",
                AmiiboCommand::Decrypt { .. } => "amiibo.decrypt",
                AmiiboCommand::Encrypt { .. } => "amiibo.encrypt",
                AmiiboCommand::Write { .. } => "amiibo.write",
            },
            Self::Type2 { command } => match command {
                Type2Command::Info => "type2.info",
                Type2Command::Read { .. } => "type2.read",
                Type2Command::Write { .. } => "type2.write",
                Type2Command::Dump { .. } => "type2.dump",
                Type2Command::Restore { .. } => "type2.restore",
                Type2Command::Lock { .. } => "type2.lock",
                Type2Command::Password { .. } => "type2.password",
                Type2Command::Uid { .. } => "type2.uid",
            },
            Self::Tunion { command } => match command {
                TunionCommand::Read => "tunion.read",
            },
            Self::Emv { command } => match command {
                EmvCommand::Read => "emv.read",
            },
            Self::Desfire { command } => match command.as_ref() {
                DesfireCommand::Info => "desfire.info",
                DesfireCommand::FormatNdef { .. } => "desfire.format_ndef",
                DesfireCommand::DeleteNdef { .. } => "desfire.delete_ndef",
                DesfireCommand::ReplaceNdef { .. } => "desfire.replace_ndef",
                DesfireCommand::FormatPicc { .. } => "desfire.format_picc",
                DesfireCommand::NdefRead { .. } => "desfire.ndef_read",
                DesfireCommand::VerifyKey { .. } => "desfire.verify_key",
                DesfireCommand::Uid { .. } => "desfire.uid",
                DesfireCommand::Read { .. } => "desfire.read",
                DesfireCommand::Write { .. } => "desfire.write",
                DesfireCommand::Value { .. } => "desfire.value",
                DesfireCommand::Record { .. } => "desfire.record",
                DesfireCommand::Memory { .. } => "desfire.memory",
                DesfireCommand::AppCreate { .. } => "desfire.app_create",
                DesfireCommand::AppDelete { .. } => "desfire.app_delete",
                DesfireCommand::FileCreate { .. } => "desfire.file_create",
                DesfireCommand::FileDelete { .. } => "desfire.file_delete",
                DesfireCommand::FileSettings { .. } => "desfire.file_settings",
                DesfireCommand::KeySettings { .. } => "desfire.key_settings",
                DesfireCommand::ChangeKey { .. } => "desfire.change_key",
            },
            Self::Ndef { command } => match command {
                NdefCommand::Read { .. } => "ndef.read",
                NdefCommand::Write { .. } => "ndef.write",
                NdefCommand::Format { .. } => "ndef.format",
            },
            Self::Recover { command } => match command {
                RecoveryCommand::Auto => "recover.auto",
                RecoveryCommand::Darkside { .. } => "recover.darkside",
                RecoveryCommand::Nested { .. } => "recover.nested",
                RecoveryCommand::StaticNested { .. } => "recover.static_nested",
                RecoveryCommand::HardNested { .. } => "recover.hardnested",
                RecoveryCommand::Fm11rf08s { .. } => "recover.fm11rf08s",
            },
            #[cfg(feature = "developer-tools")]
            Self::Pn532 { command } => match command {
                Pn532Command::Shell => "pn532.shell",
            },
            #[cfg(feature = "developer-tools")]
            Self::Apdu { .. } => "apdu.raw",
            Self::Diag => "diag",
        }
    }

    /// Only commands with a real planning phase accept the global `--dry-run`.
    fn supports_dry_run(&self) -> bool {
        match self {
            Self::Classic { command } => matches!(
                command,
                ClassicCommand::Write { .. }
                    | ClassicCommand::Uid { .. }
                    | ClassicCommand::Seal { .. }
                    | ClassicCommand::Restore { .. }
            ),
            Self::Type2 { command } => matches!(command, Type2Command::Uid { .. }),
            Self::Amiibo { command } => matches!(command, AmiiboCommand::Write { .. }),
            Self::Desfire { command } => matches!(
                command.as_ref(),
                DesfireCommand::FormatNdef { .. }
                    | DesfireCommand::ReplaceNdef { .. }
                    | DesfireCommand::FormatPicc { .. }
                    | DesfireCommand::FileSettings { .. }
            ),
            _ => false,
        }
    }
}

fn recovery(
    run: &mut Run<'_>,
    service: &mut Service,
    command: RecoveryCommand,
) -> CliResult<Value> {
    let (strategy, known_block, known_type, target_block, target_type) = match command {
        RecoveryCommand::Auto => (RecoveryStrategy::Auto, 4, "A".to_owned(), 0, "A".to_owned()),
        RecoveryCommand::Darkside {
            target_block,
            target_type,
        } => (
            RecoveryStrategy::DarkSide,
            4,
            "A".to_owned(),
            target_block,
            target_type,
        ),
        RecoveryCommand::Nested {
            known_block,
            known_type,
            target_block,
            target_type,
        } => (
            RecoveryStrategy::Nested,
            known_block,
            known_type,
            target_block,
            target_type,
        ),
        RecoveryCommand::StaticNested {
            known_block,
            known_type,
            target_block,
            target_type,
        } => (
            RecoveryStrategy::StaticNested,
            known_block,
            known_type,
            target_block,
            target_type,
        ),
        RecoveryCommand::HardNested {
            known_block,
            known_type,
            target_block,
            target_type,
        } => (
            RecoveryStrategy::HardNested,
            known_block,
            known_type,
            target_block,
            target_type,
        ),
        RecoveryCommand::Fm11rf08s {
            known_block,
            known_type,
            target_type,
        } => (
            RecoveryStrategy::Fm11rf08s,
            known_block,
            known_type,
            0,
            target_type,
        ),
    };
    run.out.set_command(recovery_command_name(strategy));
    prepare(run, service)?;
    require_confirmation(
        run,
        run.cli.yes,
        &format!(
            "starting {} recovery against the selected MIFARE Classic card",
            recovery_strategy_name(strategy)
        ),
    )?;
    let candidates = recovery_candidates(run.cli)?;
    if candidates.is_empty() && strategy != RecoveryStrategy::Auto {
        warn!(
            run.out,
            "no candidate keys; nested and hardnested recovery need a known key"
        );
    }
    let printer = Arc::new(CliProgress::new(run.out.mode()));
    let sink = Arc::clone(&printer);
    let request = RecoveryRequest {
        known_block: u8::try_from(known_block).map_err(|_| {
            CliError::Usage(format!("known block {known_block} is outside 0..=255"))
        })?,
        target_block: u8::try_from(target_block).map_err(|_| {
            CliError::Usage(format!("target block {target_block} is outside 0..=255"))
        })?,
        target_key_type: parse_key_type(&target_type)?,
        known_key_type: parse_key_type(&known_type)?,
        candidates,
        cancel: run.cancel.clone(),
        progress: Some(Arc::new(move |event| sink.report(event))),
        tables_dir: run.cli.hardnested_tables.clone(),
        max_nonces: run.cli.max_nonces,
    };
    let result = service.recover(strategy, request)?;
    if let Some(card) = &result.card {
        run.out.set_card(card);
    }
    let mut verified = Vec::new();
    if let Some(profile) = &result.profile {
        say!(
            run.out,
            "profile: prng={:?} known_key={}",
            profile.prng,
            profile.has_known_key
        );
    }
    for entry in &result.sector_keys {
        say!(
            run.out,
            "verified sector {} key {}: {}",
            entry.sector,
            entry.key_type.label(),
            entry.key.to_hex()
        );
        verified.push(json!({
            "sector": entry.sector,
            "keyType": entry.key_type.label(),
        }));
    }
    if result.sector_keys.is_empty() {
        for (key_type, key) in &result.keys {
            say!(
                run.out,
                "verified key {}: {}",
                key_type.label(),
                key.to_hex()
            );
            verified.push(json!({ "keyType": key_type.label() }));
        }
    }
    say!(run.out, "strategy: {:?}", result.attempts);
    if !result.note.is_empty() {
        say!(run.out, "{}", result.note);
    }
    let mut saved_path = None;
    if let Some(card) = &result.card {
        let mut captured = Vec::new();
        if !result.sector_keys.is_empty() {
            for entry in &result.sector_keys {
                captured.push(AuthenticatedKey {
                    sector: entry.sector,
                    key_type: entry.key_type,
                    key: entry.key,
                    source: KeySource::Recovery,
                });
            }
        } else if let Some(layout) = classic_layout_for(card)
            && let Ok(sector) = layout.sector_for_block(target_block)
        {
            for (key_type, key) in &result.keys {
                captured.push(AuthenticatedKey {
                    sector,
                    key_type: *key_type,
                    key: *key,
                    source: KeySource::Recovery,
                });
            }
        }
        let mut store = load_key_store(run.cli)?;
        saved_path = persist_recovery_keys(run, &mut store, card, &captured);
    } else {
        warn!(
            run.out,
            "recovery returned no card identity; keys were not saved"
        );
    }
    Ok(json!({
        "strategy": recovery_code(strategy),
        "attempts": result
            .attempts
            .iter()
            .map(|attempt| attack_kind_code(*attempt))
            .collect::<Vec<_>>(),
        "profile": result.profile.as_ref().map(|profile| json!({
            "prng": format!("{:?}", profile.prng),
            "hasKnownKey": profile.has_known_key,
        })),
        "verifiedKeys": verified,
        "note": result.note,
        "keyStore": saved_path.map(|path| path.display().to_string()),
    }))
}

fn classic_layout_for(card: &CardInfo) -> Option<Layout> {
    match card.card_type {
        CardType::MifareClassic1K => Some(Layout::Classic1K),
        CardType::MifareClassic4K => Some(Layout::Classic4K),
        _ => None,
    }
}

fn persist_recovery_keys(
    run: &mut Run<'_>,
    store: &mut KeyStore,
    card: &CardInfo,
    keys: &[AuthenticatedKey],
) -> Option<PathBuf> {
    if keys.is_empty() {
        return None;
    }
    if let Err(error) = store.record_verified(card, keys) {
        warn!(run.out, "recovered keys were not recorded: {error}");
        return None;
    }
    match persist_key_store(run.cli, store) {
        Ok(Some(path)) => {
            say!(
                run.out,
                "captured {} recovered key(s) in {}",
                keys.len(),
                path.display()
            );
            Some(path)
        }
        Ok(None) => None,
        Err(error) => {
            warn!(run.out, "recovered keys were not saved: {error}");
            None
        }
    }
}

fn recovery_command_name(strategy: RecoveryStrategy) -> &'static str {
    match strategy {
        RecoveryStrategy::Auto => "recover.auto",
        RecoveryStrategy::Dictionary => "recover.dictionary",
        RecoveryStrategy::DarkSide => "recover.darkside",
        RecoveryStrategy::Nested => "recover.nested",
        RecoveryStrategy::StaticNested => "recover.static_nested",
        RecoveryStrategy::HardNested => "recover.hardnested",
        RecoveryStrategy::Fm11rf08s => "recover.fm11rf08s",
    }
}

fn recovery_code(strategy: RecoveryStrategy) -> &'static str {
    match strategy {
        RecoveryStrategy::Auto => "auto",
        RecoveryStrategy::Dictionary => "dictionary",
        RecoveryStrategy::DarkSide => "darkside",
        RecoveryStrategy::Nested => "nested",
        RecoveryStrategy::StaticNested => "static_nested",
        RecoveryStrategy::HardNested => "hardnested",
        RecoveryStrategy::Fm11rf08s => "fm11rf08s",
    }
}

fn attack_kind_code(kind: nfc_recover::AttackKind) -> &'static str {
    match kind {
        nfc_recover::AttackKind::Dictionary => "dictionary",
        nfc_recover::AttackKind::Ev1Signature => "ev1_signature",
        nfc_recover::AttackKind::DarkSide => "darkside",
        nfc_recover::AttackKind::Nested => "nested",
        nfc_recover::AttackKind::StaticNested => "static_nested",
        nfc_recover::AttackKind::HardNested => "hardnested",
        nfc_recover::AttackKind::Fm11rf08s => "fm11rf08s",
    }
}

struct CliProgress {
    mode: Mode,
    start: std::time::Instant,
    state: std::sync::Mutex<ProgressState>,
}

#[derive(Default)]
struct ProgressState {
    last: Option<std::time::Instant>,
    last_stage: u8,
    last_percent: i64,
    last_received: usize,
}

impl CliProgress {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            start: std::time::Instant::now(),
            state: std::sync::Mutex::new(ProgressState::default()),
        }
    }

    fn emit(&self, phase: &str, message: &str, mut fields: Value) {
        let elapsed_ms = self.start.elapsed().as_millis();
        let mut event = json!({
            "type": "progress",
            "phase": phase,
            "message": message,
            "elapsed_ms": u64::try_from(elapsed_ms).unwrap_or(u64::MAX),
        });
        if let (Some(target), Some(extra)) = (event.as_object_mut(), fields.as_object_mut()) {
            for (key, value) in extra.iter_mut() {
                target.insert(key.clone(), value.take());
            }
        }
        match self.mode {
            Mode::Human | Mode::Json => {
                let mut stderr = io::stderr().lock();
                let _ = writeln!(stderr, "{message}");
            }
            Mode::JsonLines => output::emit_event(self.mode, event),
        }
    }

    fn report(&self, event: RecoveryProgress) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let now = std::time::Instant::now();
        let stage = match event {
            RecoveryProgress::Dictionary { .. }
            | RecoveryProgress::Darkside { .. }
            | RecoveryProgress::Sector { .. }
            | RecoveryProgress::Verify { .. } => 0,
            RecoveryProgress::Acquiring { .. } => 1,
            RecoveryProgress::Filtering { .. } => 2,
            RecoveryProgress::BruteForce { .. } => 3,
        };
        let elapsed = state
            .last
            .is_none_or(|last| now.duration_since(last).as_secs() >= 2);
        let first = state.last_stage != stage;
        match event {
            RecoveryProgress::Dictionary {
                sector,
                sector_count,
                found,
            } => {
                if first || elapsed {
                    let progress = percent(sector + 1, sector_count);
                    self.emit(
                        "dictionary",
                        &format!(
                            "scan: sector {}/{sector_count}, {found} key(s) verified",
                            sector + 1
                        ),
                        json!({
                            "sector": sector + 1,
                            "sectorCount": sector_count,
                            "found": found,
                            "progress": progress,
                        }),
                    );
                    state.last = Some(now);
                }
            }
            RecoveryProgress::Darkside { attempts } => {
                if first || elapsed {
                    self.emit(
                        "darkside",
                        &format!("darkside: {attempts} probes"),
                        json!({ "attempts": attempts }),
                    );
                    state.last = Some(now);
                }
            }
            RecoveryProgress::Sector {
                sector,
                sector_count,
                hardnested,
            } => {
                if first || elapsed {
                    let progress = percent(sector + 1, sector_count);
                    self.emit(
                        "attack",
                        &format!(
                            "{}: sector {}/{sector_count}",
                            if hardnested { "hardnested" } else { "nested" },
                            sector + 1
                        ),
                        json!({
                            "sector": sector + 1,
                            "sectorCount": sector_count,
                            "hardnested": hardnested,
                            "progress": progress,
                        }),
                    );
                    state.last = Some(now);
                }
            }
            RecoveryProgress::Verify {
                sector,
                sector_count,
            } => {
                if first || elapsed {
                    let progress = percent(sector + 1, sector_count);
                    self.emit(
                        "verify",
                        &format!("verify: sector {}/{sector_count}", sector + 1),
                        json!({
                            "sector": sector + 1,
                            "sectorCount": sector_count,
                            "progress": progress,
                        }),
                    );
                    state.last = Some(now);
                }
            }
            RecoveryProgress::Acquiring {
                received,
                distinct_nonces,
                first_bytes,
                expected_states,
            } => {
                if first || elapsed || received.saturating_sub(state.last_received) >= 64 {
                    self.emit(
                        "acquire",
                        &format!(
                            "acquire: {received} frames, {distinct_nonces} nonces, {first_bytes}/256 first bytes, estimated {expected_states:.3e} states"
                        ),
                        json!({
                            "received": received,
                            "distinctNonces": distinct_nonces,
                            "firstBytes": first_bytes,
                            "expectedStates": expected_states,
                        }),
                    );
                    state.last = Some(now);
                    state.last_received = received;
                }
            }
            RecoveryProgress::Filtering { expected_states } => {
                if first {
                    self.emit(
                        "filter",
                        &format!("filter: estimated {expected_states:.3e} states"),
                        json!({ "expectedStates": expected_states }),
                    );
                }
            }
            RecoveryProgress::BruteForce {
                tested_states,
                total_states,
                expected_states,
            } => {
                let percent = tested_states
                    .saturating_mul(100)
                    .checked_div(total_states)
                    .unwrap_or(0) as i64;
                if first
                    || percent >= state.last_percent + 10
                    || (elapsed && percent != state.last_percent)
                {
                    self.emit(
                        "brute_force",
                        &format!(
                            "brute force: {percent}% ({tested_states}/{total_states}), expected {expected_states:.3e} states"
                        ),
                        json!({
                            "testedStates": tested_states,
                            "totalStates": total_states,
                            "expectedStates": expected_states,
                            "progress": percent,
                        }),
                    );
                    state.last_percent = percent;
                    state.last = Some(now);
                }
            }
        }
        state.last_stage = stage;
    }
}

fn percent(done: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (done as f64) * 100.0 / (total as f64)
}

fn parse_key_type(value: &str) -> CliResult<KeyType> {
    match value.trim().to_ascii_uppercase().as_str() {
        "A" => Ok(KeyType::A),
        "B" => Ok(KeyType::B),
        other => Err(CliError::Usage(format!(
            "unknown key type {other:?}; use A or B"
        ))),
    }
}

fn recovery_candidates(cli: &Cli) -> CliResult<Vec<(KeyType, Key)>> {
    let store = load_key_store(cli)?;
    let mut candidates = Vec::new();
    let mut push = |candidate: (KeyType, Key)| {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };
    for entry in store.entries() {
        for key_type in [KeyType::A, KeyType::B] {
            if entry.kind.includes(key_type) {
                push((key_type, entry.key));
            }
        }
    }
    if !cli.no_common_keys {
        for key in common_keys() {
            for key_type in [KeyType::A, KeyType::B] {
                push((key_type, key));
            }
        }
    }
    Ok(candidates)
}

fn classic(run: &mut Run<'_>, service: &mut Service, command: ClassicCommand) -> CliResult<Value> {
    match command {
        ClassicCommand::Read { block } => {
            run.out.set_command("classic.read");
            classic_read(run, service, block)
        }
        ClassicCommand::Dump {
            output,
            format,
            no_sidecar,
        } => {
            run.out.set_command("classic.dump");
            classic_dump(run, service, &output, &format, no_sidecar)
        }
        ClassicCommand::Write {
            block,
            data,
            confirm_trailer,
        } => {
            run.out.set_command("classic.write");
            classic_write(run, service, block, &data, confirm_trailer)
        }
        ClassicCommand::Uid { uid, advanced } => {
            run.out.set_command("classic.uid");
            classic_uid(run, service, &uid, advanced)
        }
        ClassicCommand::Seal { advanced } => {
            run.out.set_command("classic.seal");
            classic_seal(run, service, advanced)
        }
        ClassicCommand::Restore {
            input,
            format,
            continue_on_error,
            blocks,
            changes_only,
            uid,
            advanced,
        } => {
            run.out.set_command("classic.restore");
            classic_restore(
                run,
                service,
                &input,
                format.as_deref(),
                continue_on_error,
                RestorePlanOptions {
                    blocks,
                    changes_only,
                    uid,
                    advanced,
                },
            )
        }
        ClassicCommand::Keyscan { save, name, types } => {
            run.out.set_command("classic.keyscan");
            classic_keyscan(run, service, save.as_deref(), name.as_deref(), &types)
        }
        ClassicCommand::Keys { command } => classic_keys(run, command),
        ClassicCommand::Diff { before, after } => {
            run.out.set_command("classic.diff");
            classic_diff(run, &before, &after)
        }
        ClassicCommand::Ndef { command } => classic_ndef(run, service, command),
    }
}

fn classic_read(run: &mut Run<'_>, service: &mut Service, block: usize) -> CliResult<Value> {
    prepare(run, service)?;
    let mut keys = load_key_store(run.cli)?;
    let result =
        service.classic_read_block(block, &keys, !run.cli.no_common_keys, run.cancel.clone())?;
    run.out.set_card(&result.card);
    say!(run.out, "block {block}: {}", hex_bytes(&result.data));
    say!(run.out, "ascii: {}", ascii(&result.data));
    capture_keys(run, &mut keys, &result.card, &[result.key])?;
    Ok(json!({
        "block": block,
        "hex": hex_bytes(&result.data),
        "ascii": ascii(&result.data),
    }))
}

fn classic_dump(
    run: &mut Run<'_>,
    service: &mut Service,
    output: &Path,
    format: &str,
    no_sidecar: bool,
) -> CliResult<Value> {
    prepare(run, service)?;
    let mut keys = load_key_store(run.cli)?;
    if let Some(key_file) = KeyFile::load_for_dump(output)? {
        key_file.merge_into(&mut keys)?;
    }
    let requested = DumpFormat::parse(format).map_err(CliError::usage)?;
    let result = service.classic_read(
        ClassicRead::new(&keys)
            .include_common_keys(!run.cli.no_common_keys)
            .allow_identity_only(true)
            .cancel(run.cancel.clone()),
    )?;
    if let Some(card) = &result.dump.card {
        run.out.set_card(card);
    }
    for warning in &result.warnings {
        warn!(run.out, "{warning}");
    }
    if result.identity_only {
        note!(
            run.out,
            "note: the card exposed only its selection identity; no MIFARE Classic data was readable"
        );
    }
    let bytes = exchange::export(&result.dump, requested)?;
    let chosen = requested.resolve(&result.dump);
    write_output(output, &bytes)?;
    let mut sidecar_path = None;
    if no_sidecar {
        if chosen != DumpFormat::Project && !result.dump.metadata.keys.is_empty() {
            warn!(
                run.out,
                "image-only formats do not carry sector keys; the sidecar is disabled"
            );
        }
    } else {
        match write_sidecar(output, chosen, &result.dump) {
            Ok(Some(path)) => {
                say!(run.out, "sidecar: {}", path.display());
                sidecar_path = Some(path.display().to_string());
            }
            Ok(None) => {}
            Err(error) => warn!(run.out, "sidecar was not written: {error}"),
        }
    }
    let mut keys_path = None;
    if !result.dump.metadata.keys.is_empty() {
        let key_path = KeyFile::path_for_dump(output);
        let mut key_file = if key_path.exists() {
            KeyFile::load(&key_path)?
        } else {
            KeyFile::from_keys(
                key_path.clone(),
                String::new(),
                result.dump.card.clone(),
                [],
            )
        };
        key_file.card = result.dump.card.clone().or(key_file.card);
        key_file.merge_keys(result.dump.metadata.keys.iter().map(|entry| entry.key));
        key_file.save()?;
        say!(
            run.out,
            "keys: {} ({} keys)",
            key_file.path.display(),
            key_file.keys.len()
        );
        keys_path = Some(key_file.path.display().to_string());
    }
    let readability = result.dump.readability();
    say!(
        run.out,
        "{}: {}/{} blocks, {}/{} sectors readable",
        result.dump.layout.name(),
        readability.readable_blocks,
        readability.total_blocks,
        readability.readable_sectors,
        readability.total_sectors
    );
    if !readability.skipped_sectors.is_empty() {
        say!(
            run.out,
            "skipped sectors: {}",
            readability
                .skipped_sectors
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    say!(
        run.out,
        "wrote {} ({} bytes, {})",
        output.display(),
        bytes.len(),
        chosen.name()
    );
    Ok(json!({
        "path": output.display().to_string(),
        "format": chosen.name(),
        "bytes": bytes.len(),
        "identityOnly": result.identity_only,
        "layout": result.dump.layout.name(),
        "blocksReadable": readability.readable_blocks,
        "blocksTotal": readability.total_blocks,
        "sectorsReadable": readability.readable_sectors,
        "sectorsTotal": readability.total_sectors,
        "skippedSectors": readability.skipped_sectors,
        "sidecarPath": sidecar_path,
        "keysPath": keys_path,
    }))
}

fn classic_write(
    run: &mut Run<'_>,
    service: &mut Service,
    block: usize,
    data: &str,
    confirm_trailer: bool,
) -> CliResult<Value> {
    prepare(run, service)?;
    let keys = load_key_store(run.cli)?;
    let backup = backup_store(run.cli);
    let block_data = parse_block_data(data)?;
    if run.cli.dry_run {
        if block == 0 {
            return Err(CliError::Service(NfcError::Unsupported(
                "manufacturer block 0 writes are not enabled",
            )));
        }
        let mut source = service
            .classic_read(
                ClassicRead::new(&keys)
                    .include_common_keys(!run.cli.no_common_keys)
                    .cancel(run.cancel.clone()),
            )?
            .dump;
        let target = source.blocks.get_mut(block).ok_or_else(|| {
            CliError::Service(NfcError::InvalidArgument(format!(
                "block {block} is outside the {} layout",
                source.layout.name()
            )))
        })?;
        target.set_read(block_data);
        let plan = service.classic_write_preflight(
            ClassicWritePreflight::new(source, &keys)
                .include_common_keys(!run.cli.no_common_keys)
                .selection(RestoreSelection::Blocks(vec![block]))
                .cancel(run.cancel.clone()),
        )?;
        run.out.set_card(&plan.card);
        print_write_plan(run.out, &plan);
        say!(run.out, "dry run: no card writes were performed");
        return Ok(json!({
            "dryRun": true,
            "plan": write_plan_json(&plan),
        }));
    }
    let result = service.classic_write_block(
        ClassicBlockWrite::new(block, block_data, &keys, &backup)
            .include_common_keys(!run.cli.no_common_keys)
            .confirm_trailer(confirm_trailer)
            .cancel(run.cancel.clone()),
    )?;
    let backup_path = result
        .backup
        .as_ref()
        .map(|backup| backup.path.display().to_string());
    if result.changed {
        say!(run.out, "block {} written and verified", result.block);
        if let Some(backup) = &result.backup {
            say!(run.out, "backup: {}", backup.path.display());
        }
    } else {
        say!(
            run.out,
            "block {} already contains the requested data",
            result.block
        );
    }
    Ok(json!({
        "block": result.block,
        "changed": result.changed,
        "verified": true,
        "backup": backup_path,
    }))
}

fn classic_uid(
    run: &mut Run<'_>,
    service: &mut Service,
    uid_text: &str,
    advanced: bool,
) -> CliResult<Value> {
    if !advanced {
        return Err(CliError::Usage("UID writes require --advanced".to_owned()));
    }
    let target = parse_uid(uid_text)?;
    prepare(run, service)?;
    let keys = load_key_store(run.cli)?;
    let backup = backup_store(run.cli);
    let plan = service.classic_uid_preflight(
        ClassicUidPreflight::new(target, &keys)
            .include_common_keys(!run.cli.no_common_keys)
            .advanced(true)
            .cancel(run.cancel.clone()),
    )?;
    run.out.set_card(&plan.card);
    print_uid_plan(run.out, &plan);
    if run.cli.dry_run {
        say!(run.out, "dry run: no card writes were performed");
        return Ok(json!({
            "dryRun": true,
            "plan": uid_plan_json(&plan),
        }));
    }
    require_confirmation(run, run.cli.yes, "changing the manufacturer block 0 UID")?;
    let result = service.classic_uid_execute(
        ClassicUidExecution::new(plan.token, &keys, &backup)
            .include_common_keys(!run.cli.no_common_keys)
            .advanced(true)
            .confirmed(true)
            .cancel(run.cancel.clone()),
    )?;
    run.out.set_card(&result.card_after);
    for warning in &result.warnings {
        warn!(run.out, "{warning}");
    }
    if !result.changed {
        say!(
            run.out,
            "UID {} already matches block 0; no write was needed",
            format_uid(&target)
        );
        return Ok(json!({
            "changed": false,
            "uidBefore": compact_hex(&result.card_before.uid),
            "uidAfter": compact_hex(&result.card_after.uid),
            "block0": hex_bytes(&result.new_block0),
            "backend": result.backend.map(|backend| backend.label()),
            "backup": Value::Null,
        }));
    }
    let backend = result.backend.map(|backend| backend.label());
    say!(
        run.out,
        "UID changed: {} -> {}",
        result.card_before.uid_hex(),
        result.card_after.uid_hex()
    );
    say!(run.out, "backend: {}", backend.unwrap_or("unknown"));
    say!(
        run.out,
        "block 0 verified: {}",
        hex_bytes(&result.new_block0)
    );
    say!(
        run.out,
        "BCC valid · reselected UID {}",
        result.card_after.uid_hex()
    );
    let backup_path = result
        .backup
        .as_ref()
        .map(|record| record.backup.path.display().to_string());
    if let Some(record) = &result.backup {
        say!(run.out, "backup: {}", record.backup.path.display());
    }
    Ok(json!({
        "changed": true,
        "uidBefore": compact_hex(&result.card_before.uid),
        "uidAfter": compact_hex(&result.card_after.uid),
        "block0": hex_bytes(&result.new_block0),
        "backend": backend,
        "backup": backup_path,
    }))
}

fn classic_seal(run: &mut Run<'_>, service: &mut Service, advanced: bool) -> CliResult<Value> {
    if !advanced {
        return Err(CliError::Usage(
            "UFUID sealing requires --advanced".to_owned(),
        ));
    }
    prepare(run, service)?;
    let keys = load_key_store(run.cli)?;
    let backup = backup_store(run.cli);
    let plan = service.classic_seal_preflight(
        ClassicSealPreflight::new(&keys)
            .include_common_keys(!run.cli.no_common_keys)
            .advanced(true)
            .cancel(run.cancel.clone()),
    )?;
    run.out.set_card(&plan.card);
    print_seal_plan(run.out, &plan);
    if run.cli.dry_run {
        say!(
            run.out,
            "dry run: no card writes were performed; sealing is irreversible"
        );
        return Ok(json!({
            "dryRun": true,
            "plan": seal_plan_json(&plan),
        }));
    }
    require_confirmation(
        run,
        run.cli.yes,
        "permanently sealing this UFUID (the UID cannot be changed afterwards)",
    )?;
    let result = service.classic_seal_execute(
        ClassicSealExecution::new(plan.token, &keys, &backup)
            .include_common_keys(!run.cli.no_common_keys)
            .advanced(true)
            .confirmed(true)
            .cancel(run.cancel.clone()),
    )?;
    run.out.set_card(&result.card_after);
    for warning in &result.warnings {
        warn!(run.out, "{warning}");
    }
    say!(
        run.out,
        "UFUID sealed: UID {} · the Gen1A backdoor no longer answers",
        result.card_after.uid_hex()
    );
    say!(run.out, "block 0 verified: {}", hex_bytes(&result.block0));
    say!(run.out, "seal configuration: {}", hex_bytes(&result.config));
    say!(run.out, "backup: {}", result.backup.backup.path.display());
    Ok(json!({
        "sealed": true,
        "uid": compact_hex(&result.uid),
        "block0": hex_bytes(&result.block0),
        "trailer": hex_bytes(&result.trailer),
        "config": hex_bytes(&result.config),
        "backup": result.backup.backup.path.display().to_string(),
    }))
}

fn print_seal_plan(out: &mut Output, plan: &ClassicSealPlan) {
    say!(
        out,
        "UFUID seal plan: {} · UID {} · sector 0 key {}",
        plan.layout.name(),
        plan.card.uid_hex(),
        plan.sector0_key.0.label()
    );
    say!(
        out,
        "  Gen1A probe answered; the card is unsealed and sealable"
    );
    say!(out, "  block 0 {}", hex_bytes(&plan.block0));
    say!(out, "  trailer {}", hex_bytes(&plan.trailer));
    say!(out, "  seal configuration {}", hex_bytes(&plan.config));
    for warning in &plan.warnings {
        warn!(out, "{warning}");
    }
}

fn seal_plan_json(plan: &ClassicSealPlan) -> Value {
    json!({
        "layout": plan.layout.name(),
        "uid": compact_hex(&plan.uid),
        "block0": hex_bytes(&plan.block0),
        "trailer": hex_bytes(&plan.trailer),
        "config": hex_bytes(&plan.config),
        "sector0KeyType": plan.sector0_key.0.label(),
        "gen1aProbe": true,
        "expiresAt": plan.expires_at,
    })
}

fn print_uid_plan(out: &mut Output, plan: &ClassicUidPlan) {
    let backends = plan
        .backends
        .iter()
        .map(|backend| backend.label())
        .collect::<Vec<_>>()
        .join(", ");
    say!(
        out,
        "UID plan: {} · current UID {} -> target {}",
        plan.layout.name(),
        plan.card.uid_hex(),
        hex_bytes(&plan.target_uid)
    );
    let write_key = plan
        .write_key
        .map(|(key_type, _)| key_type.label().to_string())
        .unwrap_or_else(|| "none".to_owned());
    say!(
        out,
        "sector 0 · read key {} · write key {write_key} · write with {} · backends: {backends}",
        plan.sector0_key.0.label(),
        plan.write_keys.label()
    );
    for backend in &plan.backends {
        say!(out, "  {}: {}", backend.label(), backend.note());
    }
    say!(out, "  old {}", hex_bytes(&plan.old_block0));
    say!(out, "  new {}", hex_bytes(&plan.new_block0));
    for warning in &plan.warnings {
        warn!(out, "{warning}");
    }
}

fn uid_plan_json(plan: &ClassicUidPlan) -> Value {
    json!({
        "layout": plan.layout.name(),
        "currentUid": compact_hex(&plan.current_uid),
        "targetUid": compact_hex(&plan.target_uid),
        "oldBlock0": hex_bytes(&plan.old_block0),
        "newBlock0": hex_bytes(&plan.new_block0),
        "backends": plan
            .backends
            .iter()
            .map(|backend| backend.label())
            .collect::<Vec<_>>(),
        "sector0KeyType": plan.sector0_key.0.label(),
        "writeKey": plan.write_key.map(|(key_type, _)| key_type.label()),
        "expiresAt": plan.expires_at,
    })
}

struct RestorePlanOptions {
    blocks: Option<String>,
    changes_only: bool,
    uid: Option<String>,
    advanced: bool,
}

fn classic_restore(
    run: &mut Run<'_>,
    service: &mut Service,
    input: &Path,
    format: Option<&str>,
    continue_on_error: bool,
    options: RestorePlanOptions,
) -> CliResult<Value> {
    let RestorePlanOptions {
        blocks,
        changes_only,
        uid,
        advanced,
    } = options;
    let target = uid.as_deref().map(parse_uid).transpose()?;
    if target.is_some() && !advanced {
        return Err(CliError::Usage(
            "a restore UID write requires --advanced".to_owned(),
        ));
    }
    if target.is_none() && advanced {
        return Err(CliError::Usage("--advanced requires --uid".to_owned()));
    }
    if target.is_some() && (blocks.is_some() || changes_only) {
        return Err(CliError::Usage(
            "a UID target requires a full restore; remove --blocks and --changes-only".to_owned(),
        ));
    }
    let action = match target {
        Some(uid) => format!(
            "restoring a Classic dump and writing UID {}",
            format_uid(&uid)
        ),
        None => "restoring a Classic dump".to_owned(),
    };
    if !run.cli.dry_run {
        require_confirmation(run, run.cli.yes, &action)?;
    }
    let selection = match (blocks.as_deref(), changes_only) {
        (Some(blocks), _) => RestoreSelection::Blocks(parse_block_list(blocks)?),
        (None, true) => RestoreSelection::ChangesOnly,
        (None, false) => RestoreSelection::All,
    };
    prepare(run, service)?;
    let bytes = read_input(input)?;
    let mut load = match format {
        Some(format) => {
            exchange::load_as(&bytes, DumpFormat::parse(format).map_err(CliError::usage)?)?
        }
        None => exchange::load(&bytes)?,
    };
    for warning in &load.warnings {
        warn!(run.out, "{warning}");
    }
    let sidecar = adopt_sidecar(input, load.format, &mut load.dump);
    if let Some(path) = &sidecar.path {
        say!(
            run.out,
            "sidecar: {} ({} key(s))",
            path.display(),
            sidecar.keys
        );
    }
    for warning in &sidecar.warnings {
        warn!(run.out, "{warning}");
    }
    let mut keys = load_key_store(run.cli)?;
    if let Some(key_file) = KeyFile::load_for_dump(input)? {
        key_file.merge_into(&mut keys)?;
    }
    capture_dump_metadata(run, &mut keys, &load.dump)?;
    let backup = backup_store(run.cli);
    let mut preflight = ClassicWritePreflight::new(load.dump.clone(), &keys)
        .include_common_keys(!run.cli.no_common_keys)
        .selection(selection)
        .continue_on_failure(continue_on_error)
        .cancel(run.cancel.clone());
    if let Some(uid) = target {
        preflight = preflight.uid_target(uid).advanced(true);
    }
    let plan = service.classic_write_preflight(preflight)?;
    run.out.set_card(&plan.card);
    print_write_plan(run.out, &plan);
    if run.cli.dry_run {
        say!(run.out, "dry run: no card writes were performed");
        return Ok(json!({
            "dryRun": true,
            "plan": write_plan_json(&plan),
        }));
    }
    let result = service.classic_write_execute(
        ClassicWriteExecution::new(plan.token, load.dump, &keys, &backup)
            .include_common_keys(!run.cli.no_common_keys)
            .advanced(target.is_some())
            .confirmed(true)
            .cancel(run.cancel.clone()),
    )?;
    say!(
        run.out,
        "restore: {} written, {} unchanged, {} unknown skipped, {} trailers",
        result.written_blocks,
        result.unchanged_blocks,
        result.skipped_unknown_blocks,
        result.trailer_writes
    );
    if result.not_selected_blocks > 0 {
        say!(run.out, "not selected: {}", result.not_selected_blocks);
    }
    let backup_path = result
        .backup
        .as_ref()
        .map(|backup| backup.path.display().to_string());
    if let Some(backup) = &result.backup {
        say!(run.out, "backup: {}", backup.path.display());
    }
    for warning in &result.warnings {
        warn!(run.out, "{warning}");
    }
    for failure in &result.failures {
        note!(run.out, "failure: {failure}");
    }
    if !result.failures.is_empty() {
        let committed: Vec<usize> = result
            .blocks
            .iter()
            .filter(|block| block.state == nfc_app::classic::RestoreBlockState::Written)
            .map(|block| block.block)
            .collect();
        say!(run.out, "committed blocks: {}", block_ranges(&committed));
    }
    if let Some(uid) = &result.uid {
        print_restore_uid(run.out, uid);
    }
    let uid_json = result.uid.as_ref().map(restore_uid_json);
    if !result.failures.is_empty() {
        return Err(CliError::Service(NfcError::VerificationFailed(format!(
            "restore finished with {} failure(s)",
            result.failures.len()
        ))));
    }
    if let Some(uid) = &result.uid
        && matches!(
            uid.state,
            RestoreUidState::Failed | RestoreUidState::Uncertain | RestoreUidState::NotAttempted
        )
    {
        return Err(CliError::Service(NfcError::VerificationFailed(format!(
            "UID phase {}: {}",
            uid.state.label(),
            uid.detail.as_deref().unwrap_or("no detail")
        ))));
    }
    Ok(json!({
        "dryRun": false,
        "written": result.written_blocks,
        "unchanged": result.unchanged_blocks,
        "unknownSkipped": result.skipped_unknown_blocks,
        "notSelected": result.not_selected_blocks,
        "planned": result.planned_blocks,
        "trailers": result.trailer_writes,
        "backup": backup_path,
        "failures": result.failures,
        "blocks": result
            .blocks
            .iter()
            .map(|block| json!({
                "block": block.block,
                "state": restore_block_state(block.state),
            }))
            .collect::<Vec<_>>(),
        "uid": uid_json,
    }))
}

fn restore_block_state(state: nfc_app::classic::RestoreBlockState) -> &'static str {
    use nfc_app::classic::RestoreBlockState;
    match state {
        RestoreBlockState::Planned => "planned",
        RestoreBlockState::Written => "written",
        RestoreBlockState::Unchanged => "unchanged",
        RestoreBlockState::NotSelected => "not_selected",
        RestoreBlockState::SkippedUnknown => "unknown",
        RestoreBlockState::Failed => "failed",
    }
}

fn restore_uid_json(uid: &nfc_app::classic::RestoreUidResult) -> Value {
    json!({
        "state": uid.state.label(),
        "targetUid": compact_hex(&uid.target_uid),
        "block0": hex_bytes(&uid.new_block0),
        "backend": uid.backend.map(|backend| backend.label()),
        "detail": uid.detail,
        "backup": uid
            .backup
            .as_ref()
            .map(|record| record.backup.path.display().to_string()),
    })
}

fn block_ranges(blocks: &[usize]) -> String {
    if blocks.is_empty() {
        return "none".to_owned();
    }
    let mut ranges = Vec::new();
    let mut start = blocks[0];
    let mut end = start;
    for &block in &blocks[1..] {
        if block == end + 1 {
            end = block;
        } else {
            ranges.push((start, end));
            start = block;
            end = block;
        }
    }
    ranges.push((start, end));
    ranges
        .into_iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn print_restore_uid(out: &mut Output, uid: &nfc_app::classic::RestoreUidResult) {
    for warning in &uid.warnings {
        warn!(out, "{warning}");
    }
    match uid.state {
        RestoreUidState::Applied => {
            let backend = uid
                .backend
                .map(|backend| backend.label())
                .unwrap_or("unknown");
            say!(
                out,
                "UID changed: {} -> {}",
                format_uid(&uid.old_block0[0..4]),
                format_uid(&uid.target_uid)
            );
            say!(out, "backend: {backend}");
            say!(out, "block 0 verified: {}", hex_bytes(&uid.new_block0));
            if let Some(card) = &uid.card_after {
                say!(out, "BCC valid · reselected UID {}", card.uid_hex());
            }
        }
        RestoreUidState::Unchanged => {
            say!(
                out,
                "UID {} already matches block 0; no UID write was needed",
                format_uid(&uid.target_uid)
            );
        }
        RestoreUidState::Failed | RestoreUidState::Uncertain | RestoreUidState::NotAttempted => {
            note!(
                out,
                "uid: {} · {}",
                uid.state.label(),
                uid.detail.as_deref().unwrap_or("")
            );
        }
    }
    if let Some(record) = &uid.backup {
        say!(out, "UID backup: {}", record.backup.path.display());
    }
}

fn write_plan_json(plan: &ClassicWritePlan) -> Value {
    json!({
        "layout": plan.layout.name(),
        "uid": compact_hex(&plan.card.uid),
        "selection": plan.selection.label(),
        "blocks": plan
            .blocks
            .iter()
            .map(|block| json!({
                "block": block.block,
                "sector": block.sector,
                "role": block.role_label(),
                "trailer": block.trailer,
                "old": hex_bytes(&block.old),
                "new": hex_bytes(&block.new),
            }))
            .collect::<Vec<_>>(),
        "unchanged": plan.unchanged_blocks,
        "unknownSkipped": plan.skipped_unknown_blocks,
        "trailers": plan.trailer_writes(),
        "expiresAt": plan.expires_at,
        "uid": plan.uid.as_ref().map(uid_plan_json),
    })
}

fn print_write_plan(out: &mut Output, plan: &ClassicWritePlan) {
    say!(
        out,
        "plan: {} · UID {} · {} · {} block(s), {} trailer(s)",
        plan.layout.name(),
        plan.card.uid_hex(),
        plan.selection.label(),
        plan.blocks.len(),
        plan.trailer_writes()
    );
    say!(
        out,
        "unchanged: {}, unknown skipped: {}, expires at {}",
        plan.unchanged_blocks,
        plan.skipped_unknown_blocks,
        plan.expires_at
    );
    for block in &plan.blocks {
        say!(
            out,
            "block {:<3} sector {:<2} {} · {}",
            block.block,
            block.sector,
            block.role_label(),
            block.access_note
        );
        say!(out, "  old {}", hex_bytes(&block.old));
        say!(out, "  new {}", hex_bytes(&block.new));
    }
    if let Some(uid) = &plan.uid {
        let backends = uid
            .backends
            .iter()
            .map(|backend| backend.label())
            .collect::<Vec<_>>()
            .join(", ");
        say!(
            out,
            "UID target {} · current UID {} · post-restore sector-0 read key {} · write with {} · backends: {backends}",
            format_uid(&uid.target_uid),
            format_uid(&uid.current_uid),
            uid.sector0_key.0.label(),
            uid.write_keys.label()
        );
        for backend in &uid.backends {
            say!(out, "  {}: {}", backend.label(), backend.note());
        }
        say!(out, "  old block 0 {}", hex_bytes(&uid.old_block0));
        say!(out, "  new block 0 {}", hex_bytes(&uid.new_block0));
    }
    for warning in &plan.warnings {
        warn!(out, "{warning}");
    }
    if let Some(uid) = &plan.uid {
        for warning in &uid.warnings {
            warn!(out, "{warning}");
        }
    }
}

fn classic_keyscan(
    run: &mut Run<'_>,
    service: &mut Service,
    save: Option<&Path>,
    name: Option<&str>,
    types: &str,
) -> CliResult<Value> {
    prepare(run, service)?;
    let keys = load_key_store(run.cli)?;
    let key_types = parse_key_types(types)?;
    let progress = {
        let mode = run.out.mode();
        let start = std::time::Instant::now();
        Arc::new(move |event: nfc_app::KeyscanProgress| {
            if mode != Mode::JsonLines {
                return;
            }
            let progress = if event.sector_count == 0 {
                0.0
            } else {
                ((event.sector + 1) as f64) * 100.0 / (event.sector_count as f64)
            };
            output::emit_event(
                mode,
                json!({
                    "type": "progress",
                    "phase": "keyscan",
                    "message": format!(
                        "sector {}/{}, {} attempts, {} verified",
                        event.sector + 1,
                        event.sector_count,
                        event.attempts,
                        event.verified
                    ),
                    "sector": event.sector + 1,
                    "sectorCount": event.sector_count,
                    "attempts": event.attempts,
                    "verified": event.verified,
                    "progress": progress,
                    "elapsed_ms": u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                }),
            );
        })
    };
    let result = service.classic_keyscan(
        ClassicKeyscan::new(&keys, keyscan_candidates(run.cli)?)
            .include_common_keys(!run.cli.no_common_keys)
            .key_types(key_types)
            .cancel(run.cancel.clone())
            .progress(Some(progress)),
    )?;
    run.out.set_card(&result.card);
    for sector in &result.sectors {
        match (sector.key_a, sector.key_b) {
            (Some(a), Some(b)) => say!(
                run.out,
                "sector {:<2} A={} B={}",
                sector.sector,
                a.to_hex(),
                b.to_hex()
            ),
            (Some(a), None) => say!(run.out, "sector {:<2} A={}", sector.sector, a.to_hex()),
            (None, Some(b)) => say!(run.out, "sector {:<2} B={}", sector.sector, b.to_hex()),
            (None, None) => say!(run.out, "sector {:<2} no key found", sector.sector),
        }
    }
    say!(
        run.out,
        "attempts: {}, verified: {}",
        result.attempts,
        result.verified
    );
    if result.cancelled {
        say!(run.out, "cancelled; keeping verified keys and sectors");
    }

    let mut saved = None;
    let mut captured = None;
    if let Some(path) = save {
        let is_dic = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dic"));
        if is_dic {
            if result.verified_keys.is_empty() {
                say!(run.out, "no verified keys to save");
            } else {
                let key_file = KeyFile::from_authenticated(
                    path.to_owned(),
                    name.unwrap_or_default(),
                    Some(result.card.clone()),
                    &result.verified_keys,
                );
                key_file.save()?;
                say!(
                    run.out,
                    "saved {} verified key(s) to {}",
                    key_file.keys.len(),
                    path.display()
                );
                saved = Some(path.display().to_string());
            }
        } else {
            let mut store = KeyStore::load_or_default(path)?;
            store.merge(&result.store)?;
            store.record_verified(&result.card, &result.verified_keys)?;
            store.save(path)?;
            say!(
                run.out,
                "saved {} verified key(s) to {}",
                result.verified_keys.len(),
                path.display()
            );
            saved = Some(path.display().to_string());
        }
    } else {
        let mut store = keys;
        if let Err(error) = store.record_verified(&result.card, &result.verified_keys) {
            warn!(run.out, "verified keys were not recorded: {error}");
        } else {
            match persist_key_store(run.cli, &store) {
                Ok(Some(path)) => {
                    say!(
                        run.out,
                        "captured {} verified key(s) in {}",
                        result.verified_keys.len(),
                        path.display()
                    );
                    captured = Some(path.display().to_string());
                }
                Ok(None) => {
                    say!(
                        run.out,
                        "no key store path; pass --save <card>.dic to keep discovered keys"
                    );
                }
                Err(error) => warn!(run.out, "verified keys were not saved: {error}"),
            }
        }
    }
    Ok(json!({
        "attempts": result.attempts,
        "verified": result.verified,
        "cancelled": result.cancelled,
        "sectors": result
            .sectors
            .iter()
            .map(|sector| json!({
                "sector": sector.sector,
                "keyA": sector.key_a.is_some(),
                "keyB": sector.key_b.is_some(),
            }))
            .collect::<Vec<_>>(),
        "saved": saved,
        "captured": captured,
    }))
}

fn keyscan_candidates(cli: &Cli) -> CliResult<Vec<Key>> {
    let mut candidates = Vec::new();
    for hex in &cli.key {
        let key = Key::from_hex(hex).map_err(CliError::usage)?;
        if !candidates.contains(&key) {
            candidates.push(key);
        }
    }
    if !cli.key_file.is_empty() {
        let mut store = KeyStore::new();
        for path in &cli.key_file {
            let report = store.import_text(path)?;
            report_import_issues_to_stderr(path, &report);
        }
        for key in store.all_keys() {
            if !candidates.contains(&key) {
                candidates.push(key);
            }
        }
    }
    Ok(candidates)
}

fn classic_keys(run: &mut Run<'_>, command: KeysCommand) -> CliResult<Value> {
    match command {
        KeysCommand::List { builtin } => {
            run.out.set_command("classic.keys.list");
            let (store, _path) = management_store(run.cli, false)?;
            if store.is_empty() {
                say!(run.out, "key store is empty");
            } else {
                for entry in store.entries() {
                    say!(
                        run.out,
                        "{:<12} {:<4} {} ({})",
                        entry.sector_label(),
                        entry.kind.label(),
                        entry.key.to_hex(),
                        entry.source.label()
                    );
                }
            }
            let common = common_keys();
            if builtin {
                for key in &common {
                    say!(
                        run.out,
                        "{:<12} {:<4} {} (common)",
                        "all sectors",
                        "A+B",
                        key.to_hex()
                    );
                }
            } else {
                say!(
                    run.out,
                    "built-in common keys: {} (use --builtin to list; they are not stored and cannot be deleted)",
                    common.len()
                );
            }
            Ok(json!({
                "entries": store
                    .entries()
                    .iter()
                    .map(|entry| json!({
                        "sector": entry.sector,
                        "keyType": entry.kind.label(),
                        "source": entry.source.label(),
                    }))
                    .collect::<Vec<_>>(),
                "commonKeys": common.len(),
                "builtinListed": builtin,
            }))
        }
        KeysCommand::Import { file } => {
            run.out.set_command("classic.keys.import");
            let (mut store, path) = management_store(run.cli, true)?;
            let report = store.import_text(&file)?;
            store.save(&path)?;
            say!(
                run.out,
                "imported {} key(s) into {}; {} duplicate(s), {} ignored, {} issue(s)",
                report.added,
                path.display(),
                report.duplicates,
                report.ignored,
                report.issues.len()
            );
            report_import_issues(run.out, &file, &report);
            Ok(json!({
                "path": path.display().to_string(),
                "added": report.added,
                "duplicates": report.duplicates,
                "ignored": report.ignored,
                "issues": report.issues.len(),
            }))
        }
        KeysCommand::Add {
            hex,
            sector,
            key_type,
        } => {
            run.out.set_command("classic.keys.add");
            let (mut store, path) = management_store(run.cli, true)?;
            let key = Key::from_hex(&hex).map_err(CliError::usage)?;
            let kind = KeyKind::parse(&key_type).map_err(CliError::usage)?;
            let added = store.add(KeyEntry::new(sector, kind, key, KeySource::Manual))?;
            store.save(&path)?;
            if added {
                say!(run.out, "added key to {}", path.display());
            } else {
                say!(run.out, "key is already present in {}", path.display());
            }
            Ok(json!({
                "path": path.display().to_string(),
                "added": added,
            }))
        }
        KeysCommand::Delete {
            hex,
            sector,
            key_type,
        } => {
            run.out.set_command("classic.keys.delete");
            let key = Key::from_hex(&hex).map_err(CliError::usage)?;
            let kind = KeyKind::parse(&key_type).map_err(CliError::usage)?;
            let (mut store, path) = management_store(run.cli, true)?;
            let index = store
                .entries()
                .iter()
                .position(|entry| entry.key == key && entry.sector == sector && entry.kind == kind);
            let deleted = match index.and_then(|index| store.remove(index)) {
                Some(entry) => {
                    store.save(&path)?;
                    say!(
                        run.out,
                        "deleted {} key {} ({}) from {}",
                        entry.kind.label(),
                        entry.key.to_hex(),
                        entry.sector_label(),
                        path.display()
                    );
                    true
                }
                None => {
                    let sector_label = match sector {
                        Some(sector) => format!("sector {sector}"),
                        None => "all sectors".to_owned(),
                    };
                    say!(
                        run.out,
                        "no stored {} key {} for {sector_label} in {}",
                        kind.label(),
                        key.to_hex(),
                        path.display()
                    );
                    false
                }
            };
            Ok(json!({
                "path": path.display().to_string(),
                "deleted": deleted,
            }))
        }
        KeysCommand::Export {
            format,
            all_candidates,
            output,
        } => {
            run.out.set_command("classic.keys.export");
            let (store, _path) = management_store(run.cli, false)?;
            let format = format.trim().to_ascii_lowercase();
            let scope = if all_candidates {
                "all_candidates"
            } else {
                "stored"
            };
            let keys = if all_candidates {
                store.candidate_keys().len()
            } else {
                store.all_keys().len()
            };
            let bytes = match (all_candidates, format.as_str()) {
                (true, "dic") => store.export_candidate_dic().into_bytes(),
                (true, "json") => candidate_json(&store)?,
                (false, "dic") => store.export_dic().into_bytes(),
                (false, "json") => store.to_json()?,
                (_, other) => {
                    return Err(CliError::Usage(format!(
                        "unknown key export format {other:?}; use dic or json"
                    )));
                }
            };
            match output {
                Some(path) => {
                    write_output(&path, &bytes)?;
                    say!(
                        run.out,
                        "wrote {} ({} bytes, {} keys, {scope})",
                        path.display(),
                        bytes.len(),
                        keys
                    );
                    Ok(json!({
                        "path": path.display().to_string(),
                        "format": format,
                        "bytes": bytes.len(),
                        "keys": keys,
                        "scope": scope,
                    }))
                }
                None => {
                    if !run.out.mode().is_human() {
                        return Err(CliError::Usage(
                            "classic keys export writes key material to stdout; pass --output with --json or --jsonl"
                                .to_owned(),
                        ));
                    }
                    io::stdout().write_all(&bytes)?;
                    Ok(json!({}))
                }
            }
        }
    }
}

fn candidate_json(store: &KeyStore) -> Result<Vec<u8>> {
    let keys: Vec<_> = store
        .candidate_keys()
        .into_iter()
        .map(|candidate| {
            json!({
                "key": candidate.key.to_hex(),
                "source": candidate.source.label(),
            })
        })
        .collect();
    let document = json!({
        "version": 1,
        "scope": "all_candidates",
        "keys": keys,
    });
    let mut bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| NfcError::Io(format!("could not serialize key candidates: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn classic_diff(run: &mut Run<'_>, before: &Path, after: &Path) -> CliResult<Value> {
    let before = exchange::load(&read_input(before)?)?.dump;
    let after = exchange::load(&read_input(after)?)?.dump;
    let diff = diff::diff(&before, &after)?;
    say!(
        run.out,
        "{}: {} changed, {} equal, {} unknown",
        diff.layout.name(),
        diff.changed_blocks,
        diff.equal_blocks,
        diff.unknown_blocks
    );
    say!(run.out, "trailer changes: {}", diff.trailer_changes);
    if diff.identity_changed {
        say!(run.out, "card identity changed");
    }
    let mut blocks = Vec::new();
    for block in diff.blocks.iter() {
        if block.state == diff::DiffState::Equal {
            continue;
        }
        let state = diff_state_label(block.state);
        let kind = format!("{:?}", block.kind);
        say!(
            run.out,
            "block {:<3} sector {:<2} {:?} {state}",
            block.block,
            block.sector,
            block.kind
        );
        let mut bytes = Vec::new();
        for byte in &block.bytes {
            say!(
                run.out,
                "  byte {:02X}: {} -> {}",
                byte.offset,
                if byte.before_known {
                    format!("{:02X}", byte.before)
                } else {
                    "--".to_owned()
                },
                if byte.after_known {
                    format!("{:02X}", byte.after)
                } else {
                    "--".to_owned()
                }
            );
            bytes.push(json!({
                "offset": byte.offset,
                "before": byte.before_known.then_some(format!("{:02X}", byte.before)),
                "after": byte.after_known.then_some(format!("{:02X}", byte.after)),
            }));
        }
        let mut trailer = Value::Null;
        if let Some(change) = &block.trailer
            && !change.valid
        {
            let message = change.error.as_deref().unwrap_or("invalid");
            say!(run.out, "  trailer: {message}");
            trailer = json!({ "valid": false, "error": message });
        }
        blocks.push(json!({
            "block": block.block,
            "sector": block.sector,
            "kind": kind,
            "state": state,
            "bytes": bytes,
            "trailer": trailer,
        }));
    }
    Ok(json!({
        "layout": diff.layout.name(),
        "changed": diff.changed_blocks,
        "equal": diff.equal_blocks,
        "unknown": diff.unknown_blocks,
        "trailerChanges": diff.trailer_changes,
        "identityChanged": diff.identity_changed,
        "blocks": blocks,
    }))
}

fn diff_state_label(state: diff::DiffState) -> &'static str {
    match state {
        diff::DiffState::Equal => "equal",
        diff::DiffState::Changed => "changed",
        diff::DiffState::Unknown => "unknown",
    }
}

fn classic_ndef(
    run: &mut Run<'_>,
    service: &mut Service,
    command: ClassicNdefCommand,
) -> CliResult<Value> {
    match command {
        ClassicNdefCommand::Info => {
            run.out.set_command("classic.ndef.info");
            prepare(run, service)?;
            let mut keys = load_key_store(run.cli)?;
            let read = service.classic_ndef_read(
                ClassicNdefRead::new(&keys)
                    .include_common_keys(!run.cli.no_common_keys)
                    .cancel(run.cancel.clone()),
            )?;
            if let Some(card) = &read.dump.card {
                run.out.set_card(card);
            }
            for warning in &read.warnings {
                warn!(run.out, "{warning}");
            }
            capture_dump_metadata(run, &mut keys, &read.dump)?;
            print_classic_ndef(run.out, &read.info);
            Ok(json!({ "ndef": classic_ndef_json(&read.info) }))
        }
        ClassicNdefCommand::Read { output } => {
            run.out.set_command("classic.ndef.read");
            prepare(run, service)?;
            let mut keys = load_key_store(run.cli)?;
            let read = service.classic_ndef_read(
                ClassicNdefRead::new(&keys)
                    .include_common_keys(!run.cli.no_common_keys)
                    .cancel(run.cancel.clone()),
            )?;
            if let Some(card) = &read.dump.card {
                run.out.set_card(card);
            }
            for warning in &read.warnings {
                warn!(run.out, "{warning}");
            }
            capture_dump_metadata(run, &mut keys, &read.dump)?;
            print_classic_ndef(run.out, &read.info);
            let mut output_path = None;
            match &read.info.ndef.state {
                ClassicNdefState::Message(message) => {
                    for (index, record) in message.records().iter().enumerate() {
                        print_ndef_record(run.out, index, record);
                    }
                    if let Some(path) = output {
                        let bytes = message.encode()?;
                        write_output(&path, &bytes)?;
                        say!(run.out, "wrote {} ({} bytes)", path.display(), bytes.len());
                        output_path = Some(path.display().to_string());
                    }
                }
                ClassicNdefState::Empty => say!(run.out, "NDEF: empty"),
                ClassicNdefState::Unformatted => say!(run.out, "NDEF: unformatted"),
            }
            Ok(json!({
                "ndef": classic_ndef_json(&read.info),
                "output": output_path,
            }))
        }
        ClassicNdefCommand::Write {
            file,
            text,
            uri,
            language,
        } => {
            run.out.set_command("classic.ndef.write");
            prepare(run, service)?;
            let mut keys = load_key_store(run.cli)?;
            let backup = backup_store(run.cli);
            let message = build_ndef_message(file.as_deref(), text, uri, &language)?;
            let result = service.classic_ndef_write(
                ClassicNdefWrite::new(message, &keys, &backup)
                    .include_common_keys(!run.cli.no_common_keys)
                    .cancel(run.cancel.clone()),
            )?;
            if let Some(card) = &result.dump.card {
                run.out.set_card(card);
            }
            capture_dump_metadata(run, &mut keys, &result.dump)?;
            print_classic_ndef(run.out, &result.info);
            say!(
                run.out,
                "NDEF written: {} block(s), backup {}",
                result.written_blocks,
                result.backup.path.display()
            );
            Ok(json!({
                "writtenBlocks": result.written_blocks,
                "changedBlocks": result.changed_blocks,
                "backup": result.backup.path.display().to_string(),
                "ndef": classic_ndef_json(&result.info),
            }))
        }
        ClassicNdefCommand::Format { read_only } => {
            run.out.set_command("classic.ndef.format");
            require_confirmation(run, run.cli.yes, "formatting a Classic NDEF area")?;
            prepare(run, service)?;
            let mut keys = load_key_store(run.cli)?;
            let backup = backup_store(run.cli);
            let result = service.classic_ndef_format(
                ClassicNdefFormat::new(&keys, &backup)
                    .include_common_keys(!run.cli.no_common_keys)
                    .read_only(read_only)
                    .confirm(true)
                    .cancel(run.cancel.clone()),
            )?;
            if let Some(card) = &result.dump.card {
                run.out.set_card(card);
            }
            capture_dump_metadata(run, &mut keys, &result.dump)?;
            print_classic_ndef(run.out, &result.info);
            say!(
                run.out,
                "NDEF format: {} block(s) written, {} trailer(s), backup {}",
                result.written_blocks,
                result.trailer_writes,
                result.backup.path.display()
            );
            Ok(json!({
                "writtenBlocks": result.written_blocks,
                "trailers": result.trailer_writes,
                "backup": result.backup.path.display().to_string(),
                "ndef": classic_ndef_json(&result.info),
            }))
        }
    }
}

fn classic_ndef_json(info: &ClassicNdefInfo) -> Value {
    json!({
        "mad": info.layout.as_ref().map(|layout| json!({
            "version": format!("{}.{}", layout.version.major, layout.version.minor),
            "sectors": layout.sectors,
            "dataBlocks": layout.data_blocks.len(),
            "dataBytes": layout.data_len(),
            "readOnly": layout.read_only,
            "writable": layout.writable,
        })),
        "state": classic_ndef_state_name(&info.ndef.state),
        "capacity": info.ndef.capacity,
        "messageBytes": info.ndef.message_bytes,
        "records": match &info.ndef.state {
            ClassicNdefState::Message(message) => ndef_message_json(message),
            ClassicNdefState::Empty | ClassicNdefState::Unformatted => Vec::new(),
        },
    })
}

fn classic_ndef_state_name(state: &ClassicNdefState) -> &'static str {
    match state {
        ClassicNdefState::Unformatted => "unformatted",
        ClassicNdefState::Empty => "empty",
        ClassicNdefState::Message(_) => "message",
    }
}

fn ndef_message_json(message: &NdefMessage) -> Vec<Value> {
    message
        .records()
        .iter()
        .enumerate()
        .map(|(index, record)| ndef_record_json(index, record))
        .collect()
}

fn ndef_record_json(index: usize, record: &NdefRecord) -> Value {
    let mut value = json!({
        "index": index,
        "tnf": format!("{:?}", record.tnf()),
        "type": hex_bytes(record.record_type()),
        "payload": hex_bytes(record.payload()),
    });
    if let Some(object) = value.as_object_mut() {
        if let Ok(text) = TextRecord::from_record(record) {
            object.insert("kind".to_owned(), json!("text"));
            object.insert("text".to_owned(), json!(text.text));
            object.insert("language".to_owned(), json!(text.language));
        } else if let Ok(uri) = UriRecord::from_record(record) {
            object.insert("kind".to_owned(), json!("uri"));
            object.insert("uri".to_owned(), json!(uri.uri));
        } else if let Ok(wifi) = WifiRecord::from_record(record) {
            object.insert("kind".to_owned(), json!("wifi"));
            object.insert(
                "ssid".to_owned(),
                json!(String::from_utf8_lossy(&wifi.ssid)),
            );
            object.insert(
                "authentication".to_owned(),
                json!(format!("{:04X}", wifi.authentication.bits())),
            );
            object.insert(
                "encryption".to_owned(),
                json!(format!("{:04X}", wifi.encryption.bits())),
            );
        }
    }
    value
}

fn print_classic_ndef(out: &mut Output, info: &ClassicNdefInfo) {
    match &info.layout {
        Some(layout) => {
            say!(
                out,
                "mapping: {}.{}",
                layout.version.major,
                layout.version.minor
            );
            say!(
                out,
                "NFC sectors: {}",
                layout
                    .sectors
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            say!(
                out,
                "data area: {} blocks, {} bytes, {}",
                layout.data_blocks.len(),
                layout.data_len(),
                if layout.read_only {
                    "read-only"
                } else if layout.writable {
                    "read/write"
                } else {
                    "not writable"
                }
            );
            for warning in &layout.warnings {
                warn!(out, "{warning}");
            }
        }
        None => say!(out, "MAD: absent"),
    }
    match &info.ndef.state {
        ClassicNdefState::Unformatted => say!(out, "NDEF: unformatted"),
        ClassicNdefState::Empty => say!(out, "NDEF: empty, capacity {} bytes", info.ndef.capacity),
        ClassicNdefState::Message(message) => say!(
            out,
            "NDEF: {} record(s), {} / {} bytes",
            message.records().len(),
            info.ndef.message_bytes,
            info.ndef.capacity
        ),
    }
}

fn type2_auth(cli: &Cli) -> CliResult<Option<Type2Auth>> {
    match (&cli.type2_password, &cli.type2_pack) {
        (None, None) => Ok(None),
        (Some(password), Some(pack)) => {
            let password: [u8; PAGE_SIZE] = parse_hex_array(password, "Type 2 password")?;
            let pack: [u8; 2] = parse_hex_array(pack, "Type 2 PACK")?;
            Ok(Some(Type2Auth::new(password, pack)))
        }
        _ => Err(CliError::Usage(
            "--type2-password and --type2-pack must be supplied together".to_owned(),
        )),
    }
}

fn desfire(run: &mut Run<'_>, service: &mut Service, command: DesfireCommand) -> CliResult<Value> {
    prepare(run, service)?;
    match command {
        DesfireCommand::FormatNdef {
            key,
            key_type,
            key_number,
            advanced,
        } => {
            run.out.set_command("desfire.format_ndef");
            let key = parse_hex_bytes(&key, "DESFire key")?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            if key_number > 14 {
                return Err(CliError::Usage("DESFire key numbers are 0..=14".to_owned()));
            }
            if run.cli.dry_run {
                let inventory = service.desfire_inventory(run.cancel.clone())?;
                if inventory
                    .applications
                    .iter()
                    .any(|application| application.aid == NDEF_APPLICATION_AID)
                {
                    return Err(CliError::Service(NfcError::InvalidArgument(
                        "the NDEF application D27600 already exists".to_owned(),
                    )));
                }
                print_desfire_ndef_plan(run.out, key_number);
                return Ok(json!({
                    "dryRun": true,
                    "aid": hex_bytes(&NDEF_APPLICATION_AID),
                    "keyNumber": key_number,
                }));
            }
            if !advanced {
                return Err(CliError::Usage(
                    "DESFire NDEF formatting requires --advanced".to_owned(),
                ));
            }
            print_desfire_ndef_plan(run.out, key_number);
            require_confirmation(run, run.cli.yes, "formatting the DESFire NDEF application")?;
            let mut request = DesfireNdefFormatRequest::new(key)
                .key_number(key_number)
                .advanced(true)
                .confirm(true)
                .cancel(run.cancel.clone());
            if let Some(key_type) = key_type {
                request = request.key_type(key_type);
            }
            let result = service.desfire_ndef_format(request)?;
            say!(run.out, "NDEF application formatted");
            say!(run.out, "CC: {}", hex_bytes(result.cc.raw()));
            say!(
                run.out,
                "NDEF file: {}",
                nfc_app::type4::ndef_label(&result.ndef.state)
            );
            Ok(json!({
                "aid": hex_bytes(&NDEF_APPLICATION_AID),
                "cc": hex_bytes(result.cc.raw()),
                "ndef": nfc_app::type4::ndef_label(&result.ndef.state),
            }))
        }
        DesfireCommand::DeleteNdef {
            key,
            key_type,
            key_number,
            advanced,
        } => {
            run.out.set_command("desfire.delete_ndef");
            let key = parse_hex_bytes(&key, "DESFire key")?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            if key_number > 14 {
                return Err(CliError::Usage("DESFire key numbers are 0..=14".to_owned()));
            }
            if !advanced {
                return Err(CliError::Usage(
                    "deleting the DESFire NDEF application requires --advanced".to_owned(),
                ));
            }
            let inventory = service.desfire_inventory(run.cancel.clone())?;
            if !inventory
                .applications
                .iter()
                .any(|application| application.aid == NDEF_APPLICATION_AID)
            {
                return Err(CliError::Service(NfcError::InvalidArgument(
                    "the NDEF application D27600 is not present".to_owned(),
                )));
            }
            require_confirmation(
                run,
                run.cli.yes,
                "deleting the DESFire NDEF application D27600",
            )?;
            let mut request = DesfireNdefDeleteRequest::new(key)
                .key_number(key_number)
                .advanced(true)
                .confirm(true)
                .cancel(run.cancel.clone());
            if let Some(key_type) = key_type {
                request = request.key_type(key_type);
            }
            let result = service.desfire_ndef_delete(request)?;
            say!(run.out, "NDEF application D27600 deleted");
            if result.applications.is_empty() {
                say!(run.out, "applications: none");
            } else {
                for application in &result.applications {
                    say!(run.out, "application {}", hex_bytes(application));
                }
            }
            Ok(json!({
                "deleted": hex_bytes(&NDEF_APPLICATION_AID),
                "applications": result
                    .applications
                    .iter()
                    .map(|application| hex_bytes(application))
                    .collect::<Vec<_>>(),
            }))
        }
        DesfireCommand::ReplaceNdef {
            key,
            key_type,
            key_number,
            advanced,
        } => {
            run.out.set_command("desfire.replace_ndef");
            let key = parse_hex_bytes(&key, "DESFire key")?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            if key_number > 14 {
                return Err(CliError::Usage("DESFire key numbers are 0..=14".to_owned()));
            }
            if !advanced && !run.cli.dry_run {
                return Err(CliError::Usage(
                    "replacing the DESFire NDEF application requires --advanced".to_owned(),
                ));
            }
            let inventory = service.desfire_inventory(run.cancel.clone())?;
            if !inventory
                .applications
                .iter()
                .any(|application| application.aid == NDEF_APPLICATION_AID)
            {
                return Err(CliError::Service(NfcError::InvalidArgument(
                    "the NDEF application D27600 is not present; use format-ndef".to_owned(),
                )));
            }
            let read = service.type4_read(run.cancel.clone())?;
            print_desfire_ndef_replace_plan(run.out, key_number, &read);
            if run.cli.dry_run {
                return Ok(json!({
                    "dryRun": true,
                    "aid": hex_bytes(&NDEF_APPLICATION_AID),
                    "keyNumber": key_number,
                    "current": type4_read_json(&read),
                }));
            }
            require_confirmation(
                run,
                run.cli.yes,
                "replacing the DESFire NDEF application D27600",
            )?;
            let mut request = DesfireNdefReplaceRequest::new(key)
                .key_number(key_number)
                .advanced(true)
                .confirm(true)
                .cancel(run.cancel.clone());
            if let Some(key_type) = key_type {
                request = request.key_type(key_type);
            }
            let result = service.desfire_ndef_replace(request)?;
            say!(
                run.out,
                "NDEF application D27600 replaced; previous NDEF: {}",
                nfc_app::type4::ndef_label(&result.previous.state)
            );
            say!(run.out, "CC: {}", hex_bytes(result.cc.raw()));
            say!(
                run.out,
                "NDEF file: {}",
                nfc_app::type4::ndef_label(&result.ndef.state)
            );
            Ok(json!({
                "aid": hex_bytes(&NDEF_APPLICATION_AID),
                "previous": nfc_app::type4::ndef_label(&result.previous.state),
                "cc": hex_bytes(result.cc.raw()),
                "ndef": nfc_app::type4::ndef_label(&result.ndef.state),
            }))
        }
        DesfireCommand::FormatPicc {
            key,
            key_type,
            key_number,
            advanced,
        } => {
            run.out.set_command("desfire.format_picc");
            let key = parse_hex_bytes(&key, "DESFire key")?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            if key_number > 14 {
                return Err(CliError::Usage("DESFire key numbers are 0..=14".to_owned()));
            }
            if !advanced && !run.cli.dry_run {
                return Err(CliError::Usage(
                    "DESFire FormatPICC requires --advanced".to_owned(),
                ));
            }
            let inventory = service.desfire_inventory(run.cancel.clone())?;
            print_desfire_format_picc_plan(run.out, &inventory, key_number);
            if run.cli.dry_run {
                return Ok(json!({
                    "dryRun": true,
                    "applications": inventory
                        .applications
                        .iter()
                        .map(|application| hex_bytes(&application.aid))
                        .collect::<Vec<_>>(),
                }));
            }
            require_confirmation(
                run,
                run.cli.yes,
                "formatting the PICC and erasing every application",
            )?;
            let mut request = DesfireFormatPiccRequest::new(key)
                .key_number(key_number)
                .advanced(true)
                .confirm(true)
                .cancel(run.cancel.clone());
            if let Some(key_type) = key_type {
                request = request.key_type(key_type);
            }
            let result = service.desfire_format_picc(request)?;
            say!(run.out, "{} formatted", result.card);
            say!(
                run.out,
                "applications erased: {}",
                result.applications_before.len()
            );
            for application in &result.applications_before {
                say!(run.out, "  application {}", hex_bytes(application));
            }
            if result.applications_after.is_empty() {
                say!(run.out, "applications after: none");
            } else {
                for application in &result.applications_after {
                    say!(run.out, "applications after: {}", hex_bytes(application));
                }
            }
            say!(run.out, "free memory: {} bytes", result.free_memory);
            say!(run.out, "PICC key: factory state");
            Ok(json!({
                "card": result.card,
                "applicationsErased": result
                    .applications_before
                    .iter()
                    .map(|application| hex_bytes(application))
                    .collect::<Vec<_>>(),
                "applicationsAfter": result
                    .applications_after
                    .iter()
                    .map(|application| hex_bytes(application))
                    .collect::<Vec<_>>(),
                "freeMemory": result.free_memory,
            }))
        }
        DesfireCommand::NdefRead { output } => {
            run.out.set_command("desfire.ndef_read");
            let read = service.type4_read(run.cancel.clone())?;
            say_inline!(run.out, "{}", type4_read_text(&read));
            let output_path = if let Some(path) = output {
                let message = read.ndef.message().ok_or_else(|| {
                    CliError::Service(NfcError::InvalidArgument(
                        "the NDEF file is empty".to_owned(),
                    ))
                })?;
                write_output(&path, &message.encode()?)?;
                say!(run.out, "raw message written to {}", path.display());
                Some(path.display().to_string())
            } else {
                None
            };
            Ok(json!({
                "ndef": type4_read_json(&read),
                "output": output_path,
            }))
        }
        DesfireCommand::VerifyKey {
            aid,
            key,
            key_type,
            key_number,
        } => {
            run.out.set_command("desfire.verify_key");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let key = parse_hex_bytes(&key, "DESFire key")?;
            let mut request = DesfireVerifyKeyRequest::new(aid, key).key_number(key_number);
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let result = service.desfire_verify_key(run.cancel.clone(), request)?;
            say_inline!(run.out, "{}", key_verification_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "keyNumber": result.key_number,
                "keyType": result.key_type.name(),
                "secureMessaging": result.secure_messaging,
                "keyVersion": result.key_version,
            }))
        }
        DesfireCommand::Uid {
            aid,
            key,
            key_type,
            key_number,
        } => {
            run.out.set_command("desfire.uid");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let key = parse_hex_bytes(&key, "DESFire key")?;
            let mut request = DesfireUidRequest::new(aid, key).key_number(key_number);
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let result = service.desfire_card_uid(run.cancel.clone(), request)?;
            say_inline!(run.out, "{}", card_uid_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "keyNumber": result.key_number,
                "keyType": result.key_type.name(),
                "uid": compact_hex(&result.uid),
                "integrity": result.integrity.label(),
            }))
        }
        DesfireCommand::Read {
            aid,
            file,
            key,
            key_type,
            key_number,
            offset,
            length,
            start_record,
            record_count,
        } => {
            run.out.set_command("desfire.read");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let file_id = parse_hex_array::<1>(&file, "DESFire file ID")?[0];
            let mut request = nfc_app::desfire::DesfireReadRequest::new(aid, file_id)
                .key_number(key_number)
                .offset(offset)
                .length(length)
                .start_record(start_record)
                .record_count(record_count);
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let read = service.desfire_read(run.cancel.clone(), request)?;
            Ok(match read {
                DesfireReadResult::Data(read) => {
                    say!(
                        run.out,
                        "aid {} file {:02X} {}",
                        hex_bytes(&aid),
                        file_id,
                        read.settings.summary()
                    );
                    say!(
                        run.out,
                        "protection: {}",
                        read_protection_text(&read.protection)
                    );
                    say!(run.out, "data: {}", hex_bytes(&read.data));
                    say!(run.out, "ascii: {}", ascii(&read.data));
                    json!({
                        "aid": hex_bytes(&aid),
                        "fileId": file_id,
                        "kind": "data",
                        "settings": file_settings_json(&read.settings),
                        "protection": read_protection_json(&read.protection),
                        "data": hex_bytes(&read.data),
                    })
                }
                DesfireReadResult::Value(read) => {
                    say!(
                        run.out,
                        "aid {} file {:02X} {}",
                        hex_bytes(&aid),
                        file_id,
                        read.settings.summary()
                    );
                    say!(
                        run.out,
                        "protection: {}",
                        read_protection_text(&read.protection)
                    );
                    say!(run.out, "value: {}", read.value);
                    json!({
                        "aid": hex_bytes(&aid),
                        "fileId": file_id,
                        "kind": "value",
                        "settings": file_settings_json(&read.settings),
                        "protection": read_protection_json(&read.protection),
                        "value": read.value,
                    })
                }
                DesfireReadResult::Records(read) => {
                    say!(
                        run.out,
                        "aid {} file {:02X} {}",
                        hex_bytes(&aid),
                        file_id,
                        read.settings.summary()
                    );
                    say!(
                        run.out,
                        "protection: {}",
                        read_protection_text(&read.protection)
                    );
                    say!(
                        run.out,
                        "record offset: {} · records: {} × {} bytes",
                        read.start_record,
                        read.records.len(),
                        read.record_size
                    );
                    for (index, record) in read.records.iter().enumerate() {
                        say!(
                            run.out,
                            "record {}: {} | {}",
                            read.first_record as usize + index,
                            hex_bytes(record),
                            ascii(record)
                        );
                    }
                    json!({
                        "aid": hex_bytes(&aid),
                        "fileId": file_id,
                        "kind": "records",
                        "settings": file_settings_json(&read.settings),
                        "protection": read_protection_json(&read.protection),
                        "startRecord": read.start_record,
                        "firstRecord": read.first_record,
                        "recordSize": read.record_size,
                        "records": read.records.iter().map(|record| hex_bytes(record)).collect::<Vec<_>>(),
                    })
                }
            })
        }
        DesfireCommand::Write {
            aid,
            file,
            data,
            data_file,
            offset,
            key,
            key_type,
            key_number,
        } => {
            run.out.set_command("desfire.write");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let file_id = parse_hex_array::<1>(&file, "DESFire file ID")?[0];
            let data = match (data, data_file) {
                (Some(data), None) => parse_hex_bytes(&data, "DESFire data")?,
                (None, Some(path)) => fs::read(&path).map_err(|error| {
                    NfcError::Io(format!("could not read {}: {error}", path.display()))
                })?,
                _ => {
                    return Err(CliError::Usage(
                        "pass exactly one of --data or --data-file".to_owned(),
                    ));
                }
            };
            if data.is_empty() {
                return Err(CliError::Usage(
                    "DESFire write data must not be empty".to_owned(),
                ));
            }
            let mut request = nfc_app::desfire::DesfireWriteRequest::new(aid, file_id, data)
                .offset(offset)
                .key_number(key_number)
                .backup_path(
                    backup_store(run.cli)
                        .plan_path(&format!("desfire-{}-{file_id:02X}", hex_bytes(&aid))),
                );
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let result = service.desfire_write(run.cancel.clone(), request)?;
            say!(run.out, "aid {} file {:02X}", hex_bytes(&aid), file_id);
            say_inline!(run.out, "{}", nfc_app::desfire::write_text(&result));
            Ok(json!({
                "aid": hex_bytes(&aid),
                "fileId": file_id,
                "written": result.written,
                "offset": result.offset,
                "communication": result.communication.name(),
                "integrity": result.integrity.label(),
                "committed": result.committed,
                "backup": result.backup_path.map(|path| path.display().to_string()),
            }))
        }
        DesfireCommand::Value {
            aid,
            file,
            credit,
            debit,
            limited_credit,
            key,
            key_type,
            key_number,
            tmac_key,
        } => {
            run.out.set_command("desfire.value");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let file_id = parse_hex_array::<1>(&file, "DESFire file ID")?[0];
            let (operation, amount) = match (credit, debit, limited_credit) {
                (Some(amount), None, None) => (DesfireValueOperation::Credit, amount),
                (None, Some(amount), None) => (DesfireValueOperation::Debit, amount),
                (None, None, Some(amount)) => (DesfireValueOperation::LimitedCredit, amount),
                _ => {
                    return Err(CliError::Usage(
                        "pass exactly one of --credit, --debit, or --limited-credit".to_owned(),
                    ));
                }
            };
            let mut request = DesfireValueRequest::new(aid, file_id, operation, amount)
                .key_number(key_number)
                .cancel(run.cancel.clone());
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            if let Some(tmac_key) = tmac_key {
                let tmac_key = parse_hex_bytes(&tmac_key, "Transaction MAC key")?;
                let tmac_key: [u8; 16] = tmac_key.try_into().map_err(|_| {
                    CliError::Usage("the Transaction MAC key is a 16-byte AES-128 key".to_owned())
                })?;
                request = request.tmac_key(tmac_key);
            }
            let result = service.desfire_value_change(request)?;
            say!(run.out, "aid {} file {:02X}", hex_bytes(&aid), file_id);
            say_inline!(run.out, "{}", value_text(&result));
            Ok(json!({
                "aid": hex_bytes(&aid),
                "fileId": file_id,
                "operation": result.operation.name(),
                "amount": result.amount,
                "oldValue": result.old_value,
                "newValue": result.new_value,
                "committed": result.committed,
                "transactionMac": transaction_mac_json(result.transaction_mac.as_ref()),
            }))
        }
        DesfireCommand::Record {
            aid,
            file,
            append,
            update,
            clear,
            record,
            offset,
            data,
            data_file,
            key,
            key_type,
            key_number,
            tmac_key,
        } => {
            run.out.set_command("desfire.record");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let file_id = parse_hex_array::<1>(&file, "DESFire file ID")?[0];
            let operation = match (append, update, clear) {
                (true, false, false) => DesfireRecordOperation::Append,
                (false, true, false) => DesfireRecordOperation::Update {
                    record_number: record,
                    offset,
                },
                (false, false, true) => DesfireRecordOperation::Clear,
                _ => {
                    return Err(CliError::Usage(
                        "pass exactly one of --append, --update, or --clear".to_owned(),
                    ));
                }
            };
            let data = match (data, data_file) {
                (Some(data), None) => parse_hex_bytes(&data, "DESFire record data")?,
                (None, Some(path)) => fs::read(&path).map_err(|error| {
                    NfcError::Io(format!("could not read {}: {error}", path.display()))
                })?,
                (None, None) => Vec::new(),
                _ => {
                    return Err(CliError::Usage(
                        "pass exactly one of --data or --data-file".to_owned(),
                    ));
                }
            };
            if operation != DesfireRecordOperation::Clear && data.is_empty() {
                return Err(CliError::Usage(
                    "a record append or update needs --data or --data-file".to_owned(),
                ));
            }
            if operation == DesfireRecordOperation::Clear && !data.is_empty() {
                return Err(CliError::Usage("--clear takes no record data".to_owned()));
            }
            let mut request = DesfireRecordRequest::new(aid, file_id, operation)
                .key_number(key_number)
                .data(data)
                .cancel(run.cancel.clone());
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            if let Some(tmac_key) = tmac_key {
                let tmac_key = parse_hex_bytes(&tmac_key, "Transaction MAC key")?;
                let tmac_key: [u8; 16] = tmac_key.try_into().map_err(|_| {
                    CliError::Usage("the Transaction MAC key is a 16-byte AES-128 key".to_owned())
                })?;
                request = request.tmac_key(tmac_key);
            }
            let result = service.desfire_record_change(request)?;
            say!(run.out, "aid {} file {:02X}", hex_bytes(&aid), file_id);
            say_inline!(run.out, "{}", record_text(&result));
            Ok(json!({
                "aid": hex_bytes(&aid),
                "fileId": file_id,
                "operation": result.operation.name(),
                "written": hex_bytes(&result.written),
                "recordsBefore": result.before.len(),
                "recordsAfter": result.after.len(),
                "recordSize": result.record_size,
                "committed": result.committed,
                "transactionMac": transaction_mac_json(result.transaction_mac.as_ref()),
            }))
        }
        DesfireCommand::Memory {
            aid,
            key,
            key_type,
            key_number,
        } => {
            run.out.set_command("desfire.memory");
            let aid = aid
                .as_deref()
                .map(|value| parse_hex_array::<3>(value, "DESFire AID"))
                .transpose()?;
            let mut request = DesfireFreeMemoryRequest::new(aid).key_number(key_number);
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let free_memory = service.desfire_free_memory(run.cancel.clone(), request)?;
            say_inline!(run.out, "{}", free_memory_text(aid, free_memory));
            Ok(json!({
                "aid": aid.map(|aid| hex_bytes(&aid)),
                "freeMemory": free_memory,
            }))
        }
        DesfireCommand::AppCreate {
            aid,
            key,
            key_type,
            key_number,
            keys,
            app_key_type,
        } => {
            run.out.set_command("desfire.app_create");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let app_key_type = ApplicationKeyType::parse(&app_key_type).map_err(CliError::usage)?;
            let mut request = DesfireAppCreateRequest::new(aid)
                .keys(keys)
                .app_key_type(app_key_type)
                .key_number(key_number)
                .cancel(run.cancel.clone());
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let result = service.desfire_create_application(request)?;
            say_inline!(run.out, "{}", app_create_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "keys": result.keys,
                "keyType": result.app_key_type.name(),
                "freeMemoryBefore": result.free_memory_before,
                "freeMemoryAfter": result.free_memory_after,
                "applications": result
                    .applications
                    .iter()
                    .map(|application| hex_bytes(application))
                    .collect::<Vec<_>>(),
            }))
        }
        DesfireCommand::AppDelete {
            aid,
            key,
            key_type,
            key_number,
            advanced,
            manifest,
        } => {
            run.out.set_command("desfire.app_delete");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            if !advanced {
                return Err(CliError::Usage(
                    "deleting a DESFire application requires --advanced".to_owned(),
                ));
            }
            let manifest = manifest.unwrap_or_else(|| {
                backup_store(run.cli).plan_path(&format!("desfire-app-{}", hex_bytes(&aid)))
            });
            let inventory = service.desfire_inventory(run.cancel.clone())?;
            let Some(application) = inventory
                .applications
                .iter()
                .find(|application| application.aid == aid)
            else {
                return Err(CliError::Service(NfcError::InvalidArgument(format!(
                    "application {} is not listed on the card",
                    hex_bytes(&aid)
                ))));
            };
            say!(run.out, "application {}", hex_bytes(&aid));
            if application.files.is_empty() {
                say!(run.out, "files: none visible without authentication");
            } else {
                for file in &application.files {
                    match FileSettings::parse(&file.settings) {
                        Ok(settings) => {
                            say!(run.out, "file {:02X}: {}", file.file_id, settings.summary());
                        }
                        Err(error) => say!(
                            run.out,
                            "file {:02X}: undecoded ({error}) {}",
                            file.file_id,
                            hex_bytes(&file.settings)
                        ),
                    }
                }
            }
            say!(run.out, "manifest: {}", manifest.display());
            say!(
                run.out,
                "read-back: application ID list read after deletion"
            );
            require_confirmation(
                run,
                run.cli.yes,
                &format!("deleting DESFire application {}", hex_bytes(&aid)),
            )?;
            let mut request = DesfireAppDeleteRequest::new(aid)
                .key_number(key_number)
                .advanced(true)
                .confirm(true)
                .manifest_path(manifest.clone())
                .cancel(run.cancel.clone());
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let result = service.desfire_delete_application(request)?;
            say_inline!(run.out, "{}", app_delete_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "manifest": result.manifest_path.display().to_string(),
                "freeMemoryBefore": result.free_memory_before,
                "freeMemoryAfter": result.free_memory_after,
                "applications": result
                    .applications
                    .iter()
                    .map(|application| hex_bytes(application))
                    .collect::<Vec<_>>(),
            }))
        }
        DesfireCommand::FileCreate {
            aid,
            file,
            kind,
            communication,
            access_rights,
            read_access,
            write_access,
            read_write_access,
            change_access,
            iso_file,
            size,
            lower_limit,
            upper_limit,
            value,
            flags,
            record_size,
            max_records,
            tmackey,
            tmackey_version,
            key,
            key_type,
            key_number,
        } => {
            run.out.set_command("desfire.file_create");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let file_id = parse_hex_array::<1>(&file, "DESFire file ID")?[0];
            let communication = parse_desfire_communication(&communication)?;
            let access_rights = parse_access_rights(
                access_rights.as_deref(),
                read_access.as_deref(),
                write_access.as_deref(),
                read_write_access.as_deref(),
                change_access.as_deref(),
            )?
            .unwrap_or(0x0000);
            let iso_file_id = iso_file
                .as_deref()
                .map(|value| -> CliResult<u16> {
                    let bytes = parse_hex_array::<2>(value, "DESFire ISO file ID")?;
                    Ok(u16::from_le_bytes(bytes))
                })
                .transpose()?;
            let flags = flags
                .as_deref()
                .map(|value| -> CliResult<u8> {
                    let bytes = parse_hex_bytes(value, "DESFire value flags")?;
                    match bytes.as_slice() {
                        [flag] => Ok(*flag),
                        _ => Err(CliError::Usage(
                            "DESFire value flags are one byte".to_owned(),
                        )),
                    }
                })
                .transpose()?
                .unwrap_or(0);
            let tmackey = tmackey
                .as_deref()
                .map(|value| -> CliResult<[u8; 16]> {
                    let bytes = parse_hex_bytes(value, "Transaction MAC key")?;
                    <[u8; 16]>::try_from(bytes.as_slice()).map_err(|_| {
                        CliError::Usage(
                            "the Transaction MAC key is a 16-byte AES-128 key".to_owned(),
                        )
                    })
                })
                .transpose()?;
            let spec = desfire_file_spec(
                &kind,
                file_id,
                iso_file_id,
                communication,
                access_rights,
                size,
                lower_limit,
                upper_limit,
                value,
                flags,
                record_size,
                max_records,
                tmackey,
                tmackey_version,
            )?;
            let mut request = DesfireFileCreateRequest::new(aid, spec)
                .key_number(key_number)
                .cancel(run.cancel.clone());
            if let Some(key) = key {
                request = request.key(parse_hex_bytes(&key, "DESFire key")?);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(parse_auth_key_type(&key_type)?);
            }
            let result = service.desfire_create_file(request)?;
            say_inline!(run.out, "{}", file_create_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "fileId": result.file_id,
                "settings": file_settings_json(&result.settings),
                "freeMemoryBefore": result.free_memory_before,
                "freeMemoryAfter": result.free_memory_after,
                "required": result.required,
            }))
        }
        DesfireCommand::FileDelete {
            aid,
            file,
            key,
            key_type,
            key_number,
            advanced,
            manifest,
        } => {
            run.out.set_command("desfire.file_delete");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let file_id = parse_hex_array::<1>(&file, "DESFire file ID")?[0];
            if !advanced {
                return Err(CliError::Usage(
                    "deleting a DESFire file requires --advanced".to_owned(),
                ));
            }
            let key = key
                .map(|key| parse_hex_bytes(&key, "DESFire key"))
                .transpose()?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            let mut inspection = DesfireFileInspectionRequest::new(aid, file_id)
                .key_number(key_number)
                .cancel(run.cancel.clone());
            if let Some(key) = &key {
                inspection = inspection.key(key.clone());
            }
            if let Some(key_type) = key_type {
                inspection = inspection.key_type(key_type);
            }
            let inspection = service.desfire_inspect_file(inspection)?;
            let manifest = manifest.unwrap_or_else(|| {
                backup_store(run.cli)
                    .plan_path(&format!("desfire-file-{}-{file_id:02X}", hex_bytes(&aid)))
            });
            say!(
                run.out,
                "aid {} file {file_id:02X}\ncurrent: {}\nmanifest: {}\nread-back: file ID list read after deletion",
                hex_bytes(&aid),
                inspection.settings.summary(),
                manifest.display()
            );
            require_confirmation(
                run,
                run.cli.yes,
                &format!(
                    "deleting DESFire file {file_id:02X} in application {}",
                    hex_bytes(&aid)
                ),
            )?;
            let mut request = DesfireFileDeleteRequest::new(aid, file_id)
                .key_number(key_number)
                .advanced(true)
                .confirm(true)
                .manifest_path(manifest)
                .cancel(run.cancel.clone());
            if let Some(key) = key {
                request = request.key(key);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(key_type);
            }
            let result = service.desfire_delete_file(request)?;
            say_inline!(run.out, "{}", file_delete_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "fileId": result.file_id,
                "manifest": result.manifest_path.display().to_string(),
                "freeMemoryBefore": result.free_memory_before,
                "freeMemoryAfter": result.free_memory_after,
                "files": result.files,
            }))
        }
        DesfireCommand::FileSettings {
            aid,
            file,
            communication,
            access_rights,
            read_access,
            write_access,
            read_write_access,
            change_access,
            key,
            key_type,
            key_number,
            advanced,
        } => {
            run.out.set_command("desfire.file_settings");
            let aid = parse_hex_array::<3>(&aid, "DESFire AID")?;
            let file_id = parse_hex_array::<1>(&file, "DESFire file ID")?[0];
            let communication = communication
                .as_deref()
                .map(parse_desfire_communication)
                .transpose()?;
            let access_rights = parse_access_rights(
                access_rights.as_deref(),
                read_access.as_deref(),
                write_access.as_deref(),
                read_write_access.as_deref(),
                change_access.as_deref(),
            )?;
            if communication.is_none() && access_rights.is_none() {
                return Err(CliError::Usage(
                    "give --communication, access rights, or both".to_owned(),
                ));
            }
            if !advanced && !run.cli.dry_run {
                return Err(CliError::Usage(
                    "changing DESFire file settings requires --advanced".to_owned(),
                ));
            }
            let key = key
                .map(|key| parse_hex_bytes(&key, "DESFire key"))
                .transpose()?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            let mut inspection = DesfireFileInspectionRequest::new(aid, file_id)
                .key_number(key_number)
                .cancel(run.cancel.clone());
            if let Some(key) = &key {
                inspection = inspection.key(key.clone());
            }
            if let Some(key_type) = key_type {
                inspection = inspection.key_type(key_type);
            }
            let inspection = service.desfire_inspect_file(inspection)?;
            say_inline!(
                run.out,
                "{}",
                file_settings_plan_text(
                    &inspection,
                    communication,
                    access_rights,
                    key_number,
                    key.is_some(),
                )
            );
            if run.cli.dry_run {
                return Ok(json!({
                    "dryRun": true,
                    "aid": hex_bytes(&aid),
                    "fileId": file_id,
                    "current": file_settings_json(&inspection.settings),
                }));
            }
            require_confirmation(run, run.cli.yes, "changing the DESFire file settings")?;
            let mut request = DesfireFileSettingsRequest::new(aid, file_id)
                .key_number(key_number)
                .advanced(true)
                .confirm(true)
                .cancel(run.cancel.clone());
            if let Some(communication) = communication {
                request = request.communication(communication);
            }
            if let Some(access_rights) = access_rights {
                request = request.access_rights(access_rights);
            }
            if let Some(key) = key {
                request = request.key(key);
            }
            if let Some(key_type) = key_type {
                request = request.key_type(key_type);
            }
            let result = service.desfire_change_file_settings(request)?;
            say_inline!(run.out, "{}", file_settings_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "fileId": result.file_id,
                "changed": result.changed,
                "before": file_settings_json(&result.before),
                "after": file_settings_json(&result.after),
            }))
        }
        DesfireCommand::KeySettings {
            aid,
            key,
            key_type,
            key_number,
            new_settings,
            advanced,
        } => {
            run.out.set_command("desfire.key_settings");
            let aid = parse_hex_array::<3>(aid.as_deref().unwrap_or("000000"), "DESFire AID")?;
            let key = key
                .map(|key| parse_hex_bytes(&key, "DESFire key"))
                .transpose()?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            let cancel = run.cancel.clone();
            let read_request = || {
                let mut request = DesfireKeySettingsRequest::new(aid)
                    .key_number(key_number)
                    .cancel(cancel.clone());
                if let Some(key) = &key {
                    request = request.key(key.clone());
                }
                if let Some(key_type) = key_type {
                    request = request.key_type(key_type);
                }
                request
            };
            let Some(new_settings) = new_settings else {
                let result = service.desfire_key_settings(read_request())?;
                say_inline!(run.out, "{}", key_settings_text(&result));
                return Ok(json!({
                    "aid": hex_bytes(&result.aid),
                    "settings": key_settings_json(&result.before),
                    "keyVersions": result.key_versions,
                }));
            };
            if !advanced {
                return Err(CliError::Usage(
                    "changing DESFire key settings requires --advanced".to_owned(),
                ));
            }
            let new_settings = parse_hex_array::<1>(&new_settings, "DESFire KeySettings")?[0];
            let before = service.desfire_key_settings(read_request())?;
            let label = if aid == [0x00; 3] {
                "PICC".to_owned()
            } else {
                format!("application {}", hex_bytes(&aid))
            };
            say!(run.out, "{label}");
            say!(
                run.out,
                "current KeySettings: {:02X}",
                before.before.key_settings
            );
            say!(run.out, "new KeySettings: {new_settings:02X}");
            if let Some(key_type) = key_type {
                say!(
                    run.out,
                    "key: key {key_number} · {} (session only)",
                    key_type.name()
                );
            } else {
                say!(run.out, "key: key {key_number} (session only)");
            }
            say!(
                run.out,
                "read-back: KeySettings read and compared after the change"
            );
            require_confirmation(
                run,
                run.cli.yes,
                &format!("changing the KeySettings of {label}"),
            )?;
            let request = read_request()
                .new_key_settings(new_settings)
                .advanced(true)
                .confirm(true);
            let result = service.desfire_key_settings(request)?;
            say_inline!(run.out, "{}", key_settings_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "changed": result.changed,
                "before": key_settings_json(&result.before),
                "after": key_settings_json(&result.after),
                "keyVersions": result.key_versions,
            }))
        }
        DesfireCommand::ChangeKey {
            aid,
            key,
            key_type,
            auth_key_number,
            key_number,
            new_key,
            new_key_version,
            current_key,
            export,
            acknowledge_unrecorded,
            advanced,
        } => {
            run.out.set_command("desfire.change_key");
            let aid = parse_hex_array::<3>(aid.as_deref().unwrap_or("000000"), "DESFire AID")?;
            if auth_key_number > 14 || key_number > 14 {
                return Err(CliError::Usage("DESFire key numbers are 0..=14".to_owned()));
            }
            if !advanced {
                return Err(CliError::Usage(
                    "changing a DESFire key requires --advanced".to_owned(),
                ));
            }
            if export.is_none() && !acknowledge_unrecorded {
                return Err(CliError::Usage(
                    "pass --export PATH or --acknowledge-unrecorded: a lost DESFire key cannot be recovered"
                        .to_owned(),
                ));
            }
            let action = format!(
                "changing DESFire key {key_number} on application {} to version {new_key_version}",
                hex_bytes(&aid)
            );
            let key = parse_hex_bytes(&key, "DESFire key")?;
            let key_type = key_type.as_deref().map(parse_auth_key_type).transpose()?;
            let session_type = match key_type {
                Some(key_type) => key_type,
                None => AuthKeyType::from_key(&key).map_err(CliError::usage)?,
            };
            let read = DesfireKeySettingsRequest::new(aid)
                .key(key.clone())
                .key_type(session_type)
                .key_number(auth_key_number)
                .cancel(run.cancel.clone());
            let before = service.desfire_key_settings(read)?;
            let current_version = before
                .key_versions
                .get(usize::from(key_number))
                .copied()
                .flatten();
            say!(run.out, "{action}");
            say!(
                run.out,
                "current key version: {}",
                current_version.map_or("unreported".to_owned(), |version| version.to_string())
            );
            if let Some(path) = &export {
                say!(
                    run.out,
                    "the new key is written to {} before the change",
                    path.display()
                );
            } else {
                say!(
                    run.out,
                    "the new key is not exported; it must be recorded outside this session"
                );
            }
            require_confirmation(run, run.cli.yes, &action)?;
            let mut request = DesfireKeyChangeRequest::new(
                aid,
                key_number,
                parse_hex_bytes(&new_key, "new DESFire key")?,
            )
            .key(key)
            .auth_key_number(auth_key_number)
            .new_key_version(new_key_version)
            .acknowledge_unrecorded(acknowledge_unrecorded)
            .advanced(true)
            .confirm(true)
            .key_type(session_type)
            .cancel(run.cancel.clone());
            if let Some(current_key) = current_key {
                request =
                    request.current_key(parse_hex_bytes(&current_key, "current DESFire key")?);
            }
            if let Some(export) = export {
                request = request.export_path(export);
            }
            let result = service.desfire_change_key(request)?;
            say_inline!(run.out, "{}", key_change_text(&result));
            Ok(json!({
                "aid": hex_bytes(&result.aid),
                "keyNumber": result.key_number,
                "keyType": result.key_type.name(),
                "version": result.new_key_version,
                "versionBefore": result.key_version_before,
                "versionVerified": result.verified_key_version,
                "card": result.card,
                "export": result.export_path.map(|path| path.display().to_string()),
            }))
        }
        DesfireCommand::Info => {
            run.out.set_command("desfire.info");
            let inventory = service.desfire_inventory(run.cancel.clone())?;
            say_inline!(run.out, "{}", nfc_app::desfire::inventory_text(&inventory));
            Ok(desfire_inventory_json(&inventory))
        }
    }
}

fn desfire_inventory_json(inventory: &nfc_desfire::Inventory) -> Value {
    json!({
        "version": {
            "name": inventory.version.name(),
            "vendorId": format!("{:02X}", inventory.version.vendor_id),
            "productType": format!("{:02X}", inventory.version.product_type),
            "productSubtype": format!("{:02X}", inventory.version.product_subtype),
            "major": inventory.version.major,
            "minor": inventory.version.minor,
            "storageSize": inventory.version.storage_size,
            "protocol": inventory.version.protocol,
        },
        "applications": inventory
            .applications
            .iter()
            .map(|application| json!({
                "aid": hex_bytes(&application.aid),
                "files": application
                    .files
                    .iter()
                    .map(|file| match FileSettings::parse(&file.settings) {
                        Ok(settings) => json!({
                            "fileId": file.file_id,
                            "settings": file_settings_json(&settings),
                        }),
                        Err(error) => json!({
                            "fileId": file.file_id,
                            "error": error.to_string(),
                        }),
                    })
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    })
}

fn file_settings_json(settings: &FileSettings) -> Value {
    json!({
        "type": settings.file_type.name(),
        "communication": settings.communication.name(),
        "access": {
            "raw": format!("{:04X}", settings.access.raw),
            "read": settings.access.read.name(),
            "write": settings.access.write.name(),
            "readWrite": settings.access.read_write.name(),
            "change": settings.access.change.name(),
        },
        "contents": match &settings.contents {
            nfc_desfire::FileContents::Data { size } => json!({ "kind": "data", "size": size }),
            nfc_desfire::FileContents::Value {
                lower_limit,
                upper_limit,
                limited_credit_value,
                limited_credit,
                free_get_value,
            } => json!({
                "kind": "value",
                "lowerLimit": lower_limit,
                "upperLimit": upper_limit,
                "limitedCreditValue": limited_credit_value,
                "limitedCredit": limited_credit,
                "freeGetValue": free_get_value,
            }),
            nfc_desfire::FileContents::Record {
                record_size,
                max_records,
                current_records,
            } => json!({
                "kind": "record",
                "recordSize": record_size,
                "maxRecords": max_records,
                "currentRecords": current_records,
            }),
            nfc_desfire::FileContents::TransactionMac {
                key_option,
                key_version,
                counter_limit,
            } => json!({
                "kind": "transaction_mac",
                "keyOption": key_option,
                "keyVersion": key_version,
                "counterLimit": counter_limit,
            }),
        },
    })
}

fn key_settings_json(settings: &nfc_desfire::KeySettings) -> Value {
    json!({
        "keySettings": format!("{:02X}", settings.key_settings),
        "maxKeys": settings.max_keys,
        "keyType": settings.key_type.map(|key_type| key_type.name()),
        "isoFileId": settings.iso_file_id,
    })
}

fn read_protection_json(protection: &nfc_app::desfire::DesfireReadProtection) -> Value {
    json!({
        "communication": protection.communication.name(),
        "keyType": protection.key_type.map(|key_type| key_type.name()),
        "integrity": protection.integrity.label(),
    })
}

fn transaction_mac_json(result: Option<&nfc_app::desfire::DesfireTransactionMacResult>) -> Value {
    match result {
        Some(result) => json!({
            "counter": result.counter,
            "value": hex_bytes(&result.value),
            "verified": result.verified,
        }),
        None => Value::Null,
    }
}

fn type4_read_json(read: &nfc_app::type4::Type4ReadResult) -> Value {
    let state = match &read.ndef.state {
        nfc_type4::NdefState::Empty => "empty",
        nfc_type4::NdefState::Message(_) => "message",
    };
    json!({
        "cc": hex_bytes(read.cc.raw()),
        "mapping": format!("{}.{}", read.cc.major_version, read.cc.minor_version),
        "mle": read.cc.max_le,
        "mlc": read.cc.max_lc,
        "fileId": format!("{:04X}", read.ndef.file_id),
        "maxSize": read.ndef.max_size,
        "readAccess": type4_access_label(read.ndef.read_access),
        "writeAccess": type4_access_label(read.ndef.write_access),
        "state": state,
        "messageBytes": read.ndef.message_bytes,
        "records": read
            .ndef
            .message()
            .map(ndef_message_json)
            .unwrap_or_default(),
        "readOnlyReason": read.read_only_reason(),
    })
}

fn parse_auth_key_type(value: &str) -> CliResult<AuthKeyType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "des" => Ok(AuthKeyType::Des),
        "2k3des" | "tdes2" | "3des" => Ok(AuthKeyType::Tdes2),
        "3k3des" | "tdes3" => Ok(AuthKeyType::Tdes3),
        "aes" | "aes-ev1" => Ok(AuthKeyType::Aes),
        "ev2" | "aes-ev2" => Ok(AuthKeyType::AesEv2),
        other => Err(CliError::Usage(format!(
            "unknown DESFire key type {other}; use des, 2k3des, 3k3des, aes, or ev2"
        ))),
    }
}

fn parse_desfire_communication(value: &str) -> CliResult<CommunicationMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "plain" => Ok(CommunicationMode::Plain),
        "maced" | "mac" => Ok(CommunicationMode::Maced),
        "enciphered" | "encrypted" | "full" => Ok(CommunicationMode::Enciphered),
        other => Err(CliError::Usage(format!(
            "unknown DESFire communication mode {other}; use plain, maced, or enciphered"
        ))),
    }
}

fn parse_access_rights(
    raw: Option<&str>,
    read: Option<&str>,
    write: Option<&str>,
    read_write: Option<&str>,
    change: Option<&str>,
) -> CliResult<Option<u16>> {
    let named = [read, write, read_write, change];
    if raw.is_some() && named.iter().any(Option::is_some) {
        return Err(CliError::Usage(
            "--access-rights cannot be combined with the named access rights".to_owned(),
        ));
    }
    if let Some(raw) = raw {
        let bytes = parse_hex_array::<2>(raw, "DESFire access rights")?;
        return Ok(Some(u16::from_le_bytes(bytes)));
    }
    if named.iter().all(Option::is_none) {
        return Ok(None);
    }
    let right = |value: Option<&str>| match value {
        Some(value) => AccessRight::parse(value).map_err(CliError::usage),
        None => Ok(AccessRight::Key(0)),
    };
    let rights = AccessRights::from_parts(
        right(read)?,
        right(write)?,
        right(read_write)?,
        right(change)?,
    );
    Ok(Some(rights.raw))
}

#[allow(clippy::too_many_arguments)]
fn desfire_file_spec(
    kind: &str,
    file_id: u8,
    iso_file_id: Option<u16>,
    communication: CommunicationMode,
    access_rights: u16,
    size: Option<u32>,
    lower_limit: Option<i32>,
    upper_limit: Option<i32>,
    value: Option<i32>,
    flags: u8,
    record_size: Option<u32>,
    max_records: Option<u32>,
    tmackey: Option<[u8; 16]>,
    tmackey_version: u8,
) -> CliResult<DesfireFileSpec> {
    let communication = communication.raw();
    match kind.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "standard" => Ok(DesfireFileSpec::Standard {
            file_id,
            iso_file_id,
            communication,
            access_rights,
            size: size.ok_or_else(|| CliError::Usage("standard files need --size".to_owned()))?,
        }),
        "backup" => Ok(DesfireFileSpec::Backup {
            file_id,
            iso_file_id,
            communication,
            access_rights,
            size: size.ok_or_else(|| CliError::Usage("backup files need --size".to_owned()))?,
        }),
        "value" => Ok(DesfireFileSpec::Value {
            file_id,
            iso_file_id,
            communication,
            access_rights,
            lower_limit: lower_limit
                .ok_or_else(|| CliError::Usage("value files need --lower-limit".to_owned()))?,
            upper_limit: upper_limit
                .ok_or_else(|| CliError::Usage("value files need --upper-limit".to_owned()))?,
            value: value.unwrap_or(0),
            flags,
        }),
        "linear-record" | "linear" | "record" => Ok(DesfireFileSpec::LinearRecord {
            file_id,
            iso_file_id,
            communication,
            access_rights,
            record_size: record_size
                .ok_or_else(|| CliError::Usage("record files need --record-size".to_owned()))?,
            max_records: max_records
                .ok_or_else(|| CliError::Usage("record files need --max-records".to_owned()))?,
        }),
        "cyclic-record" | "cyclic" => Ok(DesfireFileSpec::CyclicRecord {
            file_id,
            iso_file_id,
            communication,
            access_rights,
            record_size: record_size
                .ok_or_else(|| CliError::Usage("record files need --record-size".to_owned()))?,
            max_records: max_records
                .ok_or_else(|| CliError::Usage("record files need --max-records".to_owned()))?,
        }),
        "transaction-mac" | "tmac" | "mac" => Ok(DesfireFileSpec::TransactionMac {
            file_id,
            iso_file_id,
            communication,
            access_rights,
            key: tmackey.ok_or_else(|| {
                CliError::Usage("Transaction MAC files need --tmackey (16-byte AES key)".to_owned())
            })?,
            key_version: tmackey_version,
        }),
        other => Err(CliError::Usage(format!(
            "unknown DESFire file type {other}; use standard, backup, value, linear-record, cyclic-record, or transaction-mac"
        ))),
    }
}

fn print_desfire_ndef_plan(out: &Output, key_number: u8) {
    say!(
        out,
        "NDEF Tag Application D27600 (native AID {})",
        hex_bytes(&NDEF_APPLICATION_AID)
    );
    say!(out, "  DF name: {}", hex_bytes(&NDEF_DF_NAME));
    say!(
        out,
        "  CC: DESFire file {:02X} / ISO E103, {} bytes, read free and write key 0",
        NDEF_CC_FILE,
        NDEF_CC_SIZE
    );
    say!(
        out,
        "  NDEF: DESFire file {:02X} / ISO E104, {} bytes, read and write free",
        NDEF_DATA_FILE,
        NDEF_DATA_SIZE
    );
    say!(
        out,
        "  CC content: {}",
        hex_bytes(&NDEF_CAPABILITY_CONTAINER)
    );
    say!(out, "  PICC key: key {key_number} (session only)");
    say!(
        out,
        "No full-card backup exists; deleting the NDEF application is the undo path."
    );
}

fn print_desfire_ndef_replace_plan(
    out: &Output,
    key_number: u8,
    read: &nfc_app::type4::Type4ReadResult,
) {
    say!(
        out,
        "Replace NDEF Tag Application D27600 (native AID {})",
        hex_bytes(&NDEF_APPLICATION_AID)
    );
    say!(
        out,
        "  current CC: {} · NDEF file: {}",
        hex_bytes(read.cc.raw()),
        nfc_app::type4::ndef_label(&read.ndef.state)
    );
    say!(out, "  the current NDEF message is read before deletion");
    say!(
        out,
        "  delete only the exact AN11004 layout, then create it again"
    );
    say!(
        out,
        "  CC: file {:02X} / ISO E103, {NDEF_CC_SIZE} bytes",
        NDEF_CC_FILE
    );
    say!(
        out,
        "  NDEF: file {:02X} / ISO E104, {NDEF_DATA_SIZE} bytes",
        NDEF_DATA_FILE
    );
    say!(out, "  PICC key: key {key_number} (session only)");
    say!(
        out,
        "No full-card backup exists; a failure after deletion leaves no NDEF application."
    );
}

fn print_desfire_format_picc_plan(
    out: &Output,
    inventory: &nfc_desfire::Inventory,
    key_number: u8,
) {
    say!(out, "Format the PICC: erase every application and file");
    say!(out, "  card: {}", inventory.version.name());
    if inventory.applications.is_empty() {
        say!(out, "  applications: none");
    } else {
        say!(out, "  applications to erase:");
        for application in &inventory.applications {
            say!(
                out,
                "    {} · {} file(s)",
                hex_bytes(&application.aid),
                application.files.len()
            );
        }
    }
    say!(out, "  PICC key: key {key_number} (session only)");
    say!(out, "  PICC key after the reset: factory state");
    say!(
        out,
        "FormatPICC is a full reset; it is never part of NDEF provisioning."
    );
}

fn type2(run: &mut Run<'_>, service: &mut Service, command: Type2Command) -> CliResult<Value> {
    let auth = type2_auth(run.cli)?;
    match command {
        Type2Command::Info => {
            run.out.set_command("type2.info");
            prepare(run, service)?;
            let read = service.type2_read(Type2ReadRequest::new(run.cancel.clone()).auth(auth))?;
            run.out.set_card(&read.card);
            print_type2_info(run.out, &read.image, &read.warnings);
            Ok(type2_info_json(&read.image))
        }
        Type2Command::Read { page } => {
            run.out.set_command("type2.read");
            prepare(run, service)?;
            if let Some(page) = page {
                let read =
                    service.type2_read(Type2ReadRequest::new(run.cancel.clone()).auth(auth))?;
                run.out.set_card(&read.card);
                let image = &read.image;
                let data = image.page(page).ok_or_else(|| {
                    CliError::Service(NfcError::InvalidArgument(format!(
                        "page {page} is not readable"
                    )))
                })?;
                say!(
                    run.out,
                    "page {page} {:>14}: {}",
                    image.role(page).label(),
                    hex_bytes(data)
                );
                return Ok(json!({
                    "page": page,
                    "role": image.role(page).label(),
                    "hex": hex_bytes(data),
                }));
            }
            let read = service.type2_read(Type2ReadRequest::new(run.cancel.clone()).auth(auth))?;
            run.out.set_card(&read.card);
            let mut pages = Vec::new();
            for (page, data) in read.image.pages() {
                let data_text = data
                    .map(|data| hex_bytes(data))
                    .unwrap_or_else(|| "--".to_owned());
                say!(
                    run.out,
                    "page {:<3} {:>14} {data_text}",
                    page,
                    read.image.role(page).label()
                );
                pages.push(json!({
                    "page": page,
                    "role": read.image.role(page).label(),
                    "hex": data.map(|data| hex_bytes(data)),
                }));
            }
            for warning in &read.warnings {
                warn!(run.out, "{warning}");
            }
            Ok(json!({ "pages": pages }))
        }
        Type2Command::Write {
            page,
            data,
            advanced,
        } => {
            run.out.set_command("type2.write");
            if advanced {
                require_confirmation(run, run.cli.yes, "writing an Advanced Type 2 page")?;
            }
            prepare(run, service)?;
            let backup = backup_store(run.cli);
            let data: [u8; PAGE_SIZE] = parse_hex_array(&data, "page data")?;
            let result = service.type2_write_page(
                Type2PageWrite::new(page, data, &backup)
                    .advanced(advanced)
                    .confirm(advanced)
                    .auth(auth)
                    .cancel(run.cancel.clone()),
            )?;
            let backup_path = result
                .backup
                .as_ref()
                .map(|backup| backup.path.display().to_string());
            if result.changed {
                say!(
                    run.out,
                    "page {} written and verified: {}",
                    result.page,
                    hex_bytes(&result.written)
                );
                if let Some(backup) = &result.backup {
                    say!(run.out, "backup: {}", backup.path.display());
                }
            } else {
                say!(
                    run.out,
                    "page {} already contains the requested value",
                    result.page
                );
            }
            Ok(json!({
                "page": result.page,
                "changed": result.changed,
                "verified": true,
                "hex": hex_bytes(&result.written),
                "previous": hex_bytes(&result.previous),
                "backup": backup_path,
            }))
        }
        Type2Command::Dump { output } => {
            run.out.set_command("type2.dump");
            prepare(run, service)?;
            let read = service.type2_read(Type2ReadRequest::new(run.cancel.clone()).auth(auth))?;
            run.out.set_card(&read.card);
            let bytes = type2::export(&read.image)?;
            write_output(&output, &bytes)?;
            say!(
                run.out,
                "{} {}: {}/{} pages written to {}",
                read.image.layout.product.name(),
                read.image.uid_hex(),
                read.image.known_pages(),
                read.image.layout.page_count(),
                output.display()
            );
            for warning in &read.warnings {
                warn!(run.out, "{warning}");
            }
            Ok(json!({
                "path": output.display().to_string(),
                "product": read.image.layout.product.name(),
                "uid": compact_hex(&read.image.uid),
                "pagesRead": read.image.known_pages(),
                "pageCount": read.image.layout.page_count(),
                "bytes": bytes.len(),
            }))
        }
        Type2Command::Restore {
            input,
            include_configuration,
            include_capability_container,
            include_locks,
        } => {
            run.out.set_command("type2.restore");
            require_confirmation(run, run.cli.yes, "restoring a Type 2 image")?;
            prepare(run, service)?;
            let source = type2::load(&read_input(&input)?)?;
            let backup = backup_store(run.cli);
            let result = service.type2_restore(
                Type2Restore::new(source, &backup)
                    .options(RestoreOptions {
                        restore_user_memory: true,
                        restore_capability_container: include_capability_container,
                        restore_configuration: include_configuration,
                        restore_lock_bytes: include_locks,
                    })
                    .auth(auth)
                    .confirm(true)
                    .cancel(run.cancel.clone()),
            )?;
            say!(
                run.out,
                "restore complete: {} written, {} unchanged",
                result.summary.written_pages,
                result.summary.unchanged_pages
            );
            say!(run.out, "backup: {}", result.backup.path.display());
            for warning in &result.warnings {
                warn!(run.out, "{warning}");
            }
            Ok(json!({
                "written": result.summary.written_pages,
                "unchanged": result.summary.unchanged_pages,
                "backup": result.backup.path.display().to_string(),
            }))
        }
        Type2Command::Lock { pages } => {
            run.out.set_command("type2.lock");
            require_confirmation(run, run.cli.yes, "locking Type 2 pages")?;
            prepare(run, service)?;
            let pages = parse_page_list(&pages)?;
            let backup = backup_store(run.cli);
            let result = service.type2_lock(
                Type2Lock::new(pages.clone(), &backup)
                    .confirm(true)
                    .auth(auth)
                    .cancel(run.cancel.clone()),
            )?;
            if result.update.is_empty() {
                say!(run.out, "the requested pages are already locked");
            } else {
                say!(run.out, "lock bits written and verified");
            }
            let backup_path = result
                .backup
                .as_ref()
                .map(|backup| backup.path.display().to_string());
            if let Some(backup) = &result.backup {
                say!(run.out, "backup: {}", backup.path.display());
            }
            Ok(json!({
                "pages": pages,
                "alreadyLocked": result.update.is_empty(),
                "staticChanged": result.update.static_changed,
                "dynamicChanged": result.update.dynamic_changed,
                "backup": backup_path,
            }))
        }
        Type2Command::Uid { uid, advanced } => {
            run.out.set_command("type2.uid");
            type2_uid(run, service, &uid, advanced)
        }
        Type2Command::Password {
            auth0,
            access,
            pwd,
            pack,
        } => {
            run.out.set_command("type2.password");
            require_confirmation(run, run.cli.yes, "changing Type 2 password configuration")?;
            prepare(run, service)?;
            let access: [u8; 1] = parse_hex_array(&access, "ACCESS")?;
            let password: [u8; PAGE_SIZE] = parse_hex_array(&pwd, "PWD")?;
            let pack: [u8; 2] = parse_hex_array(&pack, "PACK")?;
            let backup = backup_store(run.cli);
            let result = service.type2_password(
                Type2Password::new(auth0, access[0], password, pack, &backup)
                    .current(auth)
                    .confirm(true)
                    .cancel(run.cancel.clone()),
            )?;
            say!(run.out, "password configuration written and verified");
            say!(run.out, "backup: {}", result.backup.path.display());
            Ok(json!({
                "auth0": auth0,
                "access": format!("{:02X}", access[0]),
                "verified": true,
                "backup": result.backup.path.display().to_string(),
            }))
        }
    }
}

fn amiibo_keys(cli: &Cli, required: bool) -> CliResult<Option<AmiiboKeys>> {
    match (&cli.amiibo_keys, &cli.amiibo_data_key, &cli.amiibo_tag_key) {
        (Some(combined), None, None) => Ok(Some(load_keys(combined)?)),
        (None, Some(data), Some(tag)) => Ok(Some(load_key_parts(data, tag)?)),
        (None, None, None) if !required => Ok(None),
        (None, None, None) => Err(CliError::Usage(
            "supply --amiibo-keys or both --amiibo-data-key and --amiibo-tag-key".to_owned(),
        )),
        _ => Err(CliError::Usage(
            "use --amiibo-keys or the two --amiibo-data-key/--amiibo-tag-key files, not both forms"
                .to_owned(),
        )),
    }
}

fn amiibo_identity_json(identity: AmiiboIdentity) -> Value {
    json!({
        "gameCharacterId": format!("{:04X}", identity.game_character_id),
        "gameId": identity.game_id,
        "characterId": identity.character_id,
        "variant": identity.variant,
        "figureType": identity.figure_type,
        "figureTypeLabel": identity.figure_type_label(),
        "modelNumber": format!("{:04X}", identity.model_number),
        "series": identity.series,
        "formatVersion": identity.format_version,
        "writeCount": identity.write_count,
        "appId": format!("{:08X}", identity.app_id),
        "initialized": identity.initialized(),
    })
}

fn print_amiibo_identity(out: &mut Output, identity: AmiiboIdentity) {
    say!(
        out,
        "identity: game 0x{:04X} character 0x{:02X} variant 0x{:02X} type {} model 0x{:04X} series 0x{:02X} format {}",
        identity.game_character_id,
        identity.character_id,
        identity.variant,
        identity.figure_type_label(),
        identity.model_number,
        identity.series,
        identity.format_version
    );
    say!(
        out,
        "write count {} · app ID 0x{:08X}",
        identity.write_count,
        identity.app_id
    );
}

fn tunion(run: &mut Run<'_>, service: &mut Service, command: TunionCommand) -> CliResult<Value> {
    match command {
        TunionCommand::Read => tunion_read(run, service),
    }
}

fn tunion_read(run: &mut Run<'_>, service: &mut Service) -> CliResult<Value> {
    run.out.set_command("tunion.read");
    prepare(run, service)?;
    let result = service.tunion_read(run.cancel.clone())?;
    run.out.set_card(&result.card);
    print_card(run.out, &result.card);
    let data = result.data;
    for warning in &data.warnings {
        warn!(run.out, "{warning}");
    }
    say!(run.out, "aid: {}", data.aid_hex());
    say!(run.out, "label: {}", data.label);
    if let Some(version) = data.app_version_hex() {
        say!(run.out, "app version: {version}");
    }
    say!(run.out, "card type: {}", data.card_type.label());
    say!(run.out, "city: {}", data.city_id);
    say!(run.out, "card: {}", data.masked_card_number());
    if let Some(issued) = data.issued {
        say!(run.out, "issued: {issued}");
    }
    if let Some(expires) = data.expires {
        say!(run.out, "expires: {expires}");
    }
    say!(
        run.out,
        "balance: {}",
        nfc_pboc::format_cny(data.balance_cents)
    );
    say!(run.out, "transactions: {}", data.transactions.len());
    for transaction in &data.transactions {
        let timestamp = transaction
            .timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "invalid".to_owned());
        say!(
            run.out,
            "  #{} · {timestamp} · {} · {} · terminal {}",
            transaction.sequence,
            nfc_pboc::format_cny(transaction.amount_cents),
            transaction.kind.label(),
            transaction.terminal_id
        );
    }
    say!(run.out, "travels: {}", data.travels.len());
    for travel in &data.travels {
        let timestamp = travel
            .timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "invalid".to_owned());
        say!(
            run.out,
            "  {} · {timestamp} · {} · balance {} · station {} · terminal {}",
            travel.kind.label(),
            nfc_pboc::format_cny(travel.amount_cents),
            nfc_pboc::format_cny(travel.balance_cents),
            travel.station_id,
            travel.terminal_id
        );
    }
    if !data.complete {
        warn!(run.out, "the T-Union read is incomplete");
    }
    let transactions: Vec<Value> = data
        .transactions
        .iter()
        .map(|transaction| {
            json!({
                "sequence": transaction.sequence,
                "amountCents": transaction.amount_cents,
                "kind": transaction.kind.label(),
                "kindCode": transaction.kind.code(),
                "terminalId": transaction.terminal_id,
                "timestamp": transaction.timestamp.map(|value| value.to_string()),
            })
        })
        .collect();
    let travels: Vec<Value> = data
        .travels
        .iter()
        .map(|travel| {
            json!({
                "kind": travel.kind.label(),
                "kindCode": travel.kind.code(),
                "terminalId": travel.terminal_id,
                "subType": travel.sub_type,
                "stationId": travel.station_id,
                "amountCents": travel.amount_cents,
                "balanceCents": travel.balance_cents,
                "timestamp": travel.timestamp.map(|value| value.to_string()),
                "cityId": travel.city_id,
                "institutionId": travel.institution_id,
            })
        })
        .collect();
    Ok(json!({
        "aid": data.aid_hex(),
        "label": data.label,
        "appVersion": data.app_version_hex(),
        "cardType": data.card_type.label(),
        "cityId": data.city_id,
        "cardNumber": data.masked_card_number(),
        "issued": data.issued.map(|value| value.to_string()),
        "expires": data.expires.map(|value| value.to_string()),
        "balanceCents": data.balance_cents,
        "currency": "CNY",
        "complete": data.complete,
        "transactions": transactions,
        "travels": travels,
    }))
}

fn emv(run: &mut Run<'_>, service: &mut Service, command: EmvCommand) -> CliResult<Value> {
    match command {
        EmvCommand::Read => emv_read(run, service),
    }
}

fn emv_read(run: &mut Run<'_>, service: &mut Service) -> CliResult<Value> {
    run.out.set_command("emv.read");
    prepare(run, service)?;
    let result = service.emv_read(run.cancel.clone())?;
    run.out.set_card(&result.card);
    print_card(run.out, &result.card);
    let data = result.data;
    for warning in &data.warnings {
        warn!(run.out, "{warning}");
    }
    say!(run.out, "application: {} ({})", data.label, data.aid_hex());
    say!(
        run.out,
        "priority: {}",
        data.priority
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_owned())
    );
    if let Some(version) = data.app_version_hex() {
        say!(run.out, "app version: {version}");
    }
    say!(
        run.out,
        "card: {}",
        data.pan_masked.as_deref().unwrap_or("—")
    );
    say!(
        run.out,
        "cardholder: {}",
        data.cardholder_name.as_deref().unwrap_or("—")
    );
    say!(
        run.out,
        "expires: {}",
        data.expires
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_owned())
    );
    say!(
        run.out,
        "effective: {}",
        data.effective
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_owned())
    );
    say!(
        run.out,
        "PAN sequence: {}",
        data.pan_sequence
            .map(|value| format!("{value:02}"))
            .unwrap_or_else(|| "—".to_owned())
    );
    say!(
        run.out,
        "service code: {}",
        data.service_code_text().unwrap_or_else(|| "—".to_owned())
    );
    say!(
        run.out,
        "AIP: {} ({})",
        data.aip_hex(),
        data.aip_capabilities().join(", ")
    );
    say!(
        run.out,
        "ATC: {}",
        data.atc
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_owned())
    );
    say!(
        run.out,
        "issuer country: {}",
        data.issuer_country_hex().unwrap_or_else(|| "—".to_owned())
    );
    say!(
        run.out,
        "application usage: {}",
        data.application_usage_hex()
            .unwrap_or_else(|| "—".to_owned())
    );
    say!(run.out, "applications: {}", data.applications.len());
    for application in &data.applications {
        say!(
            run.out,
            "  {} ({}) · priority {}",
            application.display_label(),
            application.aid_hex(),
            application
                .priority
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".to_owned())
        );
    }
    say!(run.out, "records: {}", data.records_read);
    if !data.complete {
        warn!(run.out, "the EMV read is incomplete");
    }
    let applications: Vec<Value> = data
        .applications
        .iter()
        .map(|application| {
            json!({
                "aid": application.aid_hex(),
                "label": application.label,
                "priority": application.priority,
            })
        })
        .collect();
    Ok(json!({
        "aid": data.aid_hex(),
        "label": &data.label,
        "priority": data.priority,
        "appVersion": data.app_version_hex(),
        "pan": data.pan_masked.as_deref(),
        "cardholderName": data.cardholder_name.as_deref(),
        "expires": data.expires.map(|value| value.to_string()),
        "effective": data.effective.map(|value| value.to_string()),
        "panSequence": data.pan_sequence,
        "serviceCode": data.service_code_text(),
        "aip": data.aip_hex(),
        "aipCapabilities": data.aip_capabilities(),
        "atc": data.atc,
        "issuerCountry": data.issuer_country_hex(),
        "applicationUsage": data.application_usage_hex(),
        "recordsRead": data.records_read,
        "complete": data.complete,
        "applications": applications,
    }))
}

fn amiibo(run: &mut Run<'_>, service: &mut Service, command: AmiiboCommand) -> CliResult<Value> {
    match command {
        AmiiboCommand::Read { output } => {
            run.out.set_command("amiibo.read");
            prepare(run, service)?;
            let keys = amiibo_keys(run.cli, false)?;
            let auth = type2_auth(run.cli)?;
            let read = service.amiibo_read(
                AmiiboReadRequest::new(run.cancel.clone())
                    .auth(auth)
                    .keys(keys),
            )?;
            run.out.set_card(&read.card);
            let uid = read.raw.uid()?;
            say!(
                run.out,
                "NTAG215 · UID {} · 135/135 pages",
                compact_hex(&uid)
            );
            if let Some(identity) = read.identity {
                say!(run.out, "signature: valid");
                print_amiibo_identity(run.out, identity);
            } else {
                say!(run.out, "signature: not checked (no key files)");
            }
            if let Some(output) = &output {
                write_output(output, read.raw.as_bytes())?;
                say!(run.out, "raw dump: {}", output.display());
            }
            for warning in &read.warnings {
                warn!(run.out, "{warning}");
            }
            Ok(json!({
                "uid": compact_hex(&uid),
                "product": read.image.layout.product.name(),
                "pagesRead": read.image.known_pages(),
                "pageCount": read.image.layout.page_count(),
                "signature": if read.identity.is_some() { "valid" } else { "not_checked" },
                "identity": read.identity.map(amiibo_identity_json),
                "warnings": read.warnings,
                "output": output.as_ref().map(|path| path.display().to_string()),
            }))
        }
        AmiiboCommand::Decrypt { input, output } => {
            run.out.set_command("amiibo.decrypt");
            let keys = amiibo_keys(run.cli, true)?
                .ok_or_else(|| CliError::Usage("Amiibo key files are required".to_owned()))?;
            let result = service.amiibo_decrypt(AmiiboFileRequest {
                keys,
                input: input.clone(),
                output: output.clone(),
                cancel: run.cancel.clone(),
            })?;
            say!(
                run.out,
                "decrypted {} · UID {} · {} bytes",
                input.display(),
                compact_hex(&result.source_uid),
                result.input_bytes
            );
            print_amiibo_identity(run.out, result.identity);
            if let Some(path) = &result.output {
                say!(run.out, "plaintext: {}", path.display());
            }
            for warning in &result.warnings {
                warn!(run.out, "{warning}");
            }
            Ok(json!({
                "sourceUid": compact_hex(&result.source_uid),
                "identity": amiibo_identity_json(result.identity),
                "input": input.display().to_string(),
                "output": result.output.as_ref().map(|path| path.display().to_string()),
                "bytes": result.output_bytes,
                "warnings": result.warnings,
            }))
        }
        AmiiboCommand::Encrypt { input, output } => {
            run.out.set_command("amiibo.encrypt");
            let keys = amiibo_keys(run.cli, true)?
                .ok_or_else(|| CliError::Usage("Amiibo key files are required".to_owned()))?;
            let result = service.amiibo_encrypt(AmiiboFileRequest {
                keys,
                input: input.clone(),
                output: output.clone(),
                cancel: run.cancel.clone(),
            })?;
            say!(
                run.out,
                "encrypted {} · UID {} · {} bytes",
                input.display(),
                compact_hex(&result.source_uid),
                result.output_bytes
            );
            print_amiibo_identity(run.out, result.identity);
            if let Some(path) = &result.output {
                say!(run.out, "raw dump: {}", path.display());
            }
            Ok(json!({
                "sourceUid": compact_hex(&result.source_uid),
                "identity": amiibo_identity_json(result.identity),
                "input": input.display().to_string(),
                "output": result.output.as_ref().map(|path| path.display().to_string()),
                "bytes": result.output_bytes,
            }))
        }
        AmiiboCommand::Write {
            input,
            plain,
            advanced,
        } => {
            run.out.set_command("amiibo.write");
            if !advanced {
                return Err(CliError::Usage("Amiibo writes need --advanced".to_owned()));
            }
            prepare(run, service)?;
            let keys = amiibo_keys(run.cli, true)?
                .ok_or_else(|| CliError::Usage("Amiibo key files are required".to_owned()))?;
            let auth = type2_auth(run.cli)?;
            let backup = backup_store(run.cli);
            let plan = service.amiibo_write_preflight(AmiiboWriteRequest {
                keys,
                source: input.clone(),
                format: if plain {
                    AmiiboSourceFormat::Plain
                } else {
                    AmiiboSourceFormat::Raw
                },
                auth,
                advanced: true,
                backup,
                cancel: run.cancel.clone(),
            })?;
            run.out.set_card(plan.card());
            say!(
                run.out,
                "target NTAG215 · UID {} · source UID {}",
                compact_hex(&plan.target_uid()),
                compact_hex(&plan.source_uid())
            );
            print_amiibo_identity(run.out, plan.identity());
            say!(
                run.out,
                "{} pages · {} · backup {}",
                plan.pages().len(),
                plan.credential_policy(),
                plan.backup_path().display()
            );
            say!(
                run.out,
                "the static lock and configuration pages become read-only after the write"
            );
            for warning in plan.warnings() {
                warn!(run.out, "{warning}");
            }
            let plan_json = json!({
                "dryRun": run.cli.dry_run,
                "targetUid": compact_hex(&plan.target_uid()),
                "sourceUid": compact_hex(&plan.source_uid()),
                "identity": amiibo_identity_json(plan.identity()),
                "writePages": plan.pages().len(),
                "credentials": plan.credential_policy(),
                "backup": plan.backup_path().display().to_string(),
                "warnings": plan.warnings(),
            });
            if run.cli.dry_run {
                return Ok(plan_json);
            }
            require_confirmation(run, run.cli.yes, "writing an Amiibo")?;
            let result = service.amiibo_write_execute(
                AmiiboWriteExecution::new(plan.token())
                    .confirm(true)
                    .cancel(run.cancel.clone()),
            )?;
            say!(
                run.out,
                "Amiibo written and verified: {} pages · UID {}",
                result.written_pages,
                compact_hex(&result.target_uid)
            );
            say!(run.out, "backup: {}", result.backup.path.display());
            for warning in &result.warnings {
                warn!(run.out, "{warning}");
            }
            Ok(json!({
                "dryRun": false,
                "verified": true,
                "written": result.written_pages,
                "targetUid": compact_hex(&result.target_uid),
                "sourceUid": compact_hex(&result.source_uid),
                "identity": amiibo_identity_json(result.identity),
                "backup": result.backup.path.display().to_string(),
                "warnings": result.warnings,
            }))
        }
    }
}

fn ndef(run: &mut Run<'_>, service: &mut Service, command: NdefCommand) -> CliResult<Value> {
    prepare(run, service)?;
    let auth = type2_auth(run.cli)?;
    match command {
        NdefCommand::Read { output } => {
            run.out.set_command("ndef.read");
            let read =
                service.type2_ndef_read(Type2ReadRequest::new(run.cancel.clone()).auth(auth))?;
            run.out.set_card(&read.card);
            let mut output_path = None;
            match &read.ndef.state {
                NdefState::Unformatted => say!(run.out, "NDEF: unformatted"),
                NdefState::Empty => say!(run.out, "NDEF: empty"),
                NdefState::Message(message) => {
                    say!(
                        run.out,
                        "NDEF: {} record(s), {} bytes, capacity {}",
                        message.records().len(),
                        read.ndef.message_bytes,
                        read.ndef.capacity
                    );
                    for (index, record) in message.records().iter().enumerate() {
                        print_ndef_record(run.out, index, record);
                    }
                    if let Some(path) = output {
                        write_output(&path, &message.encode()?)?;
                        say!(run.out, "raw message written to {}", path.display());
                        output_path = Some(path.display().to_string());
                    }
                }
            }
            for warning in &read.warnings {
                warn!(run.out, "{warning}");
            }
            Ok(json!({
                "ndef": type2_ndef_json(&read.ndef),
                "output": output_path,
            }))
        }
        NdefCommand::Write {
            file,
            text,
            uri,
            language,
        } => {
            run.out.set_command("ndef.write");
            let message = build_ndef_message(file.as_deref(), text, uri, &language)?;
            let backup = backup_store(run.cli);
            let result = service.type2_ndef_write(
                Type2NdefWrite::new(message, &backup)
                    .auth(auth)
                    .cancel(run.cancel.clone()),
            )?;
            say!(
                run.out,
                "NDEF written and verified: {} page(s) written, {} unchanged",
                result.summary.written_pages,
                result.summary.unchanged_pages
            );
            say!(run.out, "backup: {}", result.backup.path.display());
            Ok(json!({
                "written": result.summary.written_pages,
                "unchanged": result.summary.unchanged_pages,
                "backup": result.backup.path.display().to_string(),
            }))
        }
        NdefCommand::Format { otp } => {
            run.out.set_command("ndef.format");
            require_confirmation(run, run.cli.yes, "formatting the NDEF data area")?;
            if otp {
                require_confirmation(
                    run,
                    run.cli.yes,
                    "writing the one-way EV1 capability container",
                )?;
            }
            let backup = backup_store(run.cli);
            let result = service.type2_ndef_format(
                Type2Format::new(&backup)
                    .confirm(true)
                    .allow_otp_cc(otp)
                    .auth(auth)
                    .cancel(run.cancel.clone()),
            )?;
            say!(
                run.out,
                "NDEF formatted: {} page(s) written, {} unchanged",
                result.summary.written_pages,
                result.summary.unchanged_pages
            );
            say!(run.out, "backup: {}", result.backup.path.display());
            Ok(json!({
                "written": result.summary.written_pages,
                "unchanged": result.summary.unchanged_pages,
                "otp": otp,
                "backup": result.backup.path.display().to_string(),
            }))
        }
    }
}

fn type2_uid(
    run: &mut Run<'_>,
    service: &mut Service,
    uid_text: &str,
    advanced: bool,
) -> CliResult<Value> {
    if !advanced {
        return Err(CliError::Usage("UID writes require --advanced".to_owned()));
    }
    let target = parse_uid_hex(uid_text)?;
    prepare(run, service)?;
    let plan = service.type2_uid_preflight(&target, run.cancel.clone())?;
    run.out.set_card(&plan.card);
    say!(
        run.out,
        "{} · current UID {}, target UID {}",
        plan.card.card_type.label(),
        hex_bytes(&plan.current_uid),
        hex_bytes(&plan.target_uid)
    );
    for page in 0..3u8 {
        say!(
            run.out,
            "page {page}: {} -> {}",
            hex_bytes(&plan.current_pages[usize::from(page)]),
            hex_bytes(&plan.new_pages[usize::from(page)])
        );
    }
    if run.cli.dry_run {
        say!(run.out, "dry run: no card writes were performed");
        return Ok(json!({
            "dryRun": true,
            "currentUid": hex_bytes(&plan.current_uid),
            "targetUid": hex_bytes(&plan.target_uid),
            "pages": (0..3u8)
                .map(|page| json!({
                    "page": page,
                    "old": hex_bytes(&plan.current_pages[usize::from(page)]),
                    "new": hex_bytes(&plan.new_pages[usize::from(page)]),
                }))
                .collect::<Vec<_>>(),
        }));
    }
    require_confirmation(run, run.cli.yes, "changing the Type 2 UID")?;
    let backup = backup_store(run.cli);
    let result = service.type2_uid_execute(
        Type2UidExecution::new(plan, &backup)
            .confirm(true)
            .cancel(run.cancel.clone()),
    )?;
    run.out.set_card(&result.card_after);
    say!(
        run.out,
        "UID changed: {} -> {}",
        result.card_before.uid_hex(),
        result.card_after.uid_hex()
    );
    for page in 0..3u8 {
        say!(
            run.out,
            "page {page} verified: {}",
            hex_bytes(&result.pages[usize::from(page)])
        );
    }
    say!(run.out, "backup: {}", result.backup.path.display());
    Ok(json!({
        "changed": true,
        "uidBefore": compact_hex(&result.card_before.uid),
        "uidAfter": compact_hex(&result.card_after.uid),
        "pages": result
            .pages
            .iter()
            .enumerate()
            .map(|(page, data)| json!({
                "page": page,
                "hex": hex_bytes(data),
            }))
            .collect::<Vec<_>>(),
        "backup": result.backup.path.display().to_string(),
    }))
}

fn type2_info_json(image: &Type2Image) -> Value {
    let ndef = match nfc_type2::read_ndef(image) {
        Ok(ndef) => type2_ndef_json(&ndef),
        Err(error) => json!({ "state": "invalid", "error": error.to_string() }),
    };
    let locks = match LockBits::from_image(image) {
        Ok(locks) => {
            let dynamic = image
                .layout
                .dynamic_lock
                .map(|_| hex_bytes(&locks.dynamic_lock));
            let locked = (0..image.layout.total_pages)
                .filter(|page| locks.is_locked(&image.layout, *page))
                .collect::<Vec<_>>();
            json!({
                "static": hex_bytes(&locks.static_lock),
                "dynamic": dynamic,
                "lockedPages": locked,
            })
        }
        Err(error) => json!({ "error": error.to_string() }),
    };
    let configuration = match Configuration::from_image(image) {
        Ok(config) => json!({
            "auth0": config.auth0,
            "access": format!("{:02X}", config.access),
            "cfgLocked": config.cfg_locked(),
            "authLim": config.auth_lim(),
        }),
        Err(error) => json!({ "error": error.to_string() }),
    };
    json!({
        "product": image.layout.product.name(),
        "uid": compact_hex(&image.uid),
        "pagesRead": image.known_pages(),
        "pageCount": image.layout.page_count(),
        "userFirst": image.layout.user_first,
        "userLast": image.layout.user_last,
        "userBytes": image.layout.user_bytes(),
        "cc": if image.cc.is_present() {
            json!({
                "hex": hex_bytes(&image.cc.to_bytes()),
                "size": image.cc.size,
                "capacity": image.cc.ndef_capacity(),
            })
        } else {
            Value::Null
        },
        "ndef": ndef,
        "locks": locks,
        "configuration": configuration,
    })
}

fn type2_ndef_json(ndef: &nfc_type2::Type2Ndef) -> Value {
    let (state, records) = match &ndef.state {
        NdefState::Unformatted => ("unformatted", Vec::new()),
        NdefState::Empty => ("empty", Vec::new()),
        NdefState::Message(message) => ("message", ndef_message_json(message)),
    };
    json!({
        "state": state,
        "capacity": ndef.capacity,
        "messageBytes": ndef.message_bytes,
        "readOnly": ndef.read_only,
        "records": records,
    })
}

fn print_type2_info(out: &mut Output, image: &Type2Image, warnings: &[String]) {
    say!(out, "product: {}", image.layout.product.name());
    say!(out, "uid: {}", image.uid_hex());
    say!(
        out,
        "pages: {}/{} readable",
        image.known_pages(),
        image.layout.page_count()
    );
    say!(
        out,
        "user memory: pages {}..={} ({} bytes)",
        image.layout.user_first,
        image.layout.user_last,
        image.layout.user_bytes()
    );
    if image.cc.is_present() {
        say!(
            out,
            "capability container: {}",
            hex_bytes(&image.cc.to_bytes())
        );
        say!(out, "NDEF data area: {} bytes", image.cc.size);
        say!(out, "NDEF capacity: {} bytes", image.cc.ndef_capacity());
    } else {
        say!(out, "capability container: none (unformatted)");
        say!(out, "NDEF data area: none");
        say!(out, "NDEF capacity: none");
    }
    match nfc_type2::read_ndef(image) {
        Ok(ndef) => {
            let state = match &ndef.state {
                NdefState::Unformatted => "unformatted".to_owned(),
                NdefState::Empty => "empty".to_owned(),
                NdefState::Message(message) => {
                    format!(
                        "{} record(s), {} bytes",
                        message.records().len(),
                        ndef.message_bytes
                    )
                }
            };
            say!(out, "NDEF: {state}");
            if image.cc.is_present() {
                say!(out, "NDEF read-only: {}", ndef.read_only);
            }
        }
        Err(error) => say!(out, "NDEF: invalid ({error})"),
    }
    match LockBits::from_image(image) {
        Ok(locks) => {
            let dynamic = image
                .layout
                .dynamic_lock
                .map_or_else(|| "none".to_owned(), |_| hex_bytes(&locks.dynamic_lock));
            say!(
                out,
                "static lock: {}  dynamic lock: {dynamic}",
                hex_bytes(&locks.static_lock)
            );
            let locked: Vec<String> = (0..image.layout.total_pages)
                .filter(|page| locks.is_locked(&image.layout, *page))
                .map(|page| page.to_string())
                .collect();
            if !locked.is_empty() {
                say!(out, "locked pages: {}", locked.join(", "));
            }
        }
        Err(error) => say!(out, "locks: unavailable ({error})"),
    }
    match Configuration::from_image(image) {
        Ok(config) => {
            say!(
                out,
                "AUTH0: {:02X}  ACCESS: {:02X}  CFGLCK: {}  AUTHLIM: {}",
                config.auth0,
                config.access,
                config.cfg_locked(),
                config.auth_lim()
            );
        }
        Err(error) => say!(out, "configuration: {error}"),
    }
    for warning in warnings {
        warn!(out, "{warning}");
    }
}

fn print_ndef_record(out: &Output, index: usize, record: &NdefRecord) {
    say!(out, "{}", ndef_record_text(index, record));
}

fn ndef_record_text(index: usize, record: &NdefRecord) -> String {
    if let Ok(text) = TextRecord::from_record(record) {
        return format!(
            "record {index}: text {:?} (language {})",
            text.text, text.language
        );
    }
    if let Ok(uri) = UriRecord::from_record(record) {
        return format!("record {index}: uri {}", uri.uri);
    }
    if let Ok(wifi) = WifiRecord::from_record(record) {
        let ssid = String::from_utf8_lossy(&wifi.ssid);
        let key = wifi
            .network_key
            .as_ref()
            .map(|key| String::from_utf8_lossy(key).to_string());
        return format!(
            "record {index}: wifi ssid {ssid:?} auth 0x{:04X} encryption 0x{:04X}{}",
            wifi.authentication.bits(),
            wifi.encryption.bits(),
            key.map(|key| format!(" key {key:?}")).unwrap_or_default()
        );
    }
    format!(
        "record {index}: tnf {:?} type {} payload {}",
        record.tnf(),
        hex_bytes(record.record_type()),
        hex_bytes(record.payload())
    )
}

fn type4_access_label(access: u8) -> String {
    match access {
        0x00 => "free".to_owned(),
        0xFF => "denied".to_owned(),
        other => format!("proprietary {other:#04X}"),
    }
}

fn type4_read_text(read: &nfc_app::type4::Type4ReadResult) -> String {
    let mut text = format!(
        "CC {} · mapping {}.{} · MLe {} · MLc {}\n",
        hex_bytes(read.cc.raw()),
        read.cc.major_version,
        read.cc.minor_version,
        read.cc.max_le,
        read.cc.max_lc
    );
    for tlv in read.cc.tlvs() {
        match tlv {
            CcTlv::ProprietaryFile(control) => text.push_str(&format!(
                "CC TLV: proprietary file {:04X} · {} bytes · read {} · write {}\n",
                control.file_id,
                control.max_size,
                type4_access_label(control.read_access),
                type4_access_label(control.write_access)
            )),
            CcTlv::Unknown { tag, value } => text.push_str(&format!(
                "CC TLV: unknown tag {tag:#04X} · {} bytes · {}\n",
                value.len(),
                hex_bytes(value)
            )),
            CcTlv::NdefFile(_) | CcTlv::Null | CcTlv::Terminator => {}
        }
    }
    let state = match &read.ndef.state {
        nfc_type4::NdefState::Empty => "empty".to_owned(),
        nfc_type4::NdefState::Message(message) => format!(
            "{} record(s), {} bytes",
            message.records().len(),
            read.ndef.message_bytes
        ),
    };
    text.push_str(&format!(
        "NDEF file {:04X} · {} bytes · read {} · write {} · {}\n",
        read.ndef.file_id,
        read.ndef.max_size,
        type4_access_label(read.ndef.read_access),
        type4_access_label(read.ndef.write_access),
        state
    ));
    if let Some(message) = read.ndef.message() {
        for (index, record) in message.records().iter().enumerate() {
            text.push_str(&ndef_record_text(index, record));
            text.push('\n');
        }
    }
    if let Some(reason) = read.read_only_reason() {
        text.push_str(&format!("read-only: {reason}\n"));
    }
    text
}

fn build_ndef_message(
    file: Option<&Path>,
    text: Option<String>,
    uri: Option<String>,
    language: &str,
) -> CliResult<NdefMessage> {
    let supplied =
        usize::from(file.is_some()) + usize::from(text.is_some()) + usize::from(uri.is_some());
    if supplied != 1 {
        return Err(CliError::Usage(
            "pass exactly one of --file, --text, or --uri".to_owned(),
        ));
    }
    if let Some(path) = file {
        return Ok(NdefMessage::decode(&read_input(path)?)?);
    }
    if let Some(text) = text {
        let record = TextRecord::new(text, language)
            .map_err(CliError::usage)?
            .to_record()
            .map_err(CliError::usage)?;
        return NdefMessage::single(record).map_err(CliError::usage);
    }
    let uri = uri.expect("one NDEF source is present");
    NdefMessage::single(UriRecord::new(uri).map_err(CliError::usage)?.to_record()?)
        .map_err(CliError::usage)
}

fn require_confirmation(run: &mut Run<'_>, yes: bool, action: &str) -> CliResult<()> {
    if yes {
        return Ok(());
    }
    let stdin = io::stdin();
    if !run.out.mode().is_human() || !stdin.is_terminal() {
        return Err(CliError::Usage(format!(
            "{action} needs confirmation; pass --yes"
        )));
    }
    eprint!("{action}. Continue? [Y/n] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    stdin.read_line(&mut answer)?;
    if confirmation_answer(&answer) {
        Ok(())
    } else {
        Err(CliError::Service(NfcError::Cancelled))
    }
}

fn confirmation_answer(answer: &str) -> bool {
    let answer = answer.trim().to_ascii_lowercase();
    answer.is_empty() || answer == "y" || answer == "yes"
}

fn parse_page_list(value: &str) -> CliResult<Vec<u8>> {
    let mut pages = Vec::new();
    for part in value.split([',', ' ']).filter(|part| !part.is_empty()) {
        if let Some((first, last)) = part.split_once('-') {
            let first: u8 = first
                .parse()
                .map_err(|_| CliError::Usage(format!("invalid page {first:?}")))?;
            let last: u8 = last
                .parse()
                .map_err(|_| CliError::Usage(format!("invalid page {last:?}")))?;
            if first > last {
                return Err(CliError::Usage(format!("page range {part:?} is reversed")));
            }
            pages.extend(first..=last);
        } else {
            let page: u8 = part
                .parse()
                .map_err(|_| CliError::Usage(format!("invalid page {part:?}")))?;
            pages.push(page);
        }
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

fn parse_hex_bytes(value: &str, label: &str) -> CliResult<Vec<u8>> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.is_empty() || !compact.len().is_multiple_of(2) {
        return Err(CliError::Usage(format!(
            "{label} must contain an even number of hexadecimal digits"
        )));
    }
    let mut output = Vec::with_capacity(compact.len() / 2);
    for index in (0..compact.len()).step_by(2) {
        output.push(
            (hex_digit(compact.as_bytes()[index])? << 4)
                | hex_digit(compact.as_bytes()[index + 1])?,
        );
    }
    Ok(output)
}

fn parse_hex_array<const N: usize>(value: &str, label: &str) -> CliResult<[u8; N]> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != N * 2 {
        return Err(CliError::Usage(format!(
            "{label} must contain {} hexadecimal digits",
            N * 2
        )));
    }
    let mut output = [0u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = (hex_digit(compact.as_bytes()[index * 2])? << 4)
            | hex_digit(compact.as_bytes()[index * 2 + 1])?;
    }
    Ok(output)
}

fn management_store(cli: &Cli, create: bool) -> CliResult<(KeyStore, PathBuf)> {
    let path = cli
        .key_store
        .clone()
        .or_else(KeyStore::default_path)
        .ok_or_else(|| {
            CliError::Usage(
                "no key store path; pass --key-store or set XDG_CONFIG_HOME/HOME".to_owned(),
            )
        })?;
    if path.exists() {
        Ok((KeyStore::load(&path)?, path))
    } else if !create && cli.key_store.is_some() {
        Err(CliError::Service(NfcError::Io(format!(
            "key store {} does not exist",
            path.display()
        ))))
    } else {
        Ok((KeyStore::new(), path))
    }
}

fn load_key_store(cli: &Cli) -> CliResult<KeyStore> {
    let mut store = if let Some(path) = &cli.key_store {
        KeyStore::load_or_default(path)?
    } else if let Some(path) = KeyStore::default_path() {
        KeyStore::load_or_default(&path)?
    } else {
        KeyStore::new()
    };
    for hex in &cli.key {
        let key = Key::from_hex(hex).map_err(CliError::usage)?;
        store.add(KeyEntry::new(None, KeyKind::Any, key, KeySource::Manual))?;
    }
    for path in &cli.key_file {
        let report = store.import_text(path)?;
        report_import_issues_to_stderr(path, &report);
    }
    Ok(store)
}

/// Import issues discovered while loading the ambient key store are
/// diagnostics: they cannot reach the borrowing `Output` from this helper.
fn report_import_issues_to_stderr(path: &Path, report: &nfc_app::classic::KeyImportReport) {
    if report.issues.is_empty() {
        return;
    }
    let mut stderr = io::stderr().lock();
    let _ = writeln!(
        stderr,
        "warning: {}: {} line(s) ignored, {} issue(s)",
        path.display(),
        report.ignored,
        report.issues.len()
    );
    for issue in &report.issues {
        let _ = writeln!(stderr, "  line {}: {}", issue.line, issue.message);
    }
}

fn report_import_issues(out: &mut Output, path: &Path, report: &nfc_app::classic::KeyImportReport) {
    if report.issues.is_empty() {
        return;
    }
    warn!(
        out,
        "{}: {} line(s) ignored, {} issue(s)",
        path.display(),
        report.ignored,
        report.issues.len()
    );
    for issue in &report.issues {
        note!(out, "  line {}: {}", issue.line, issue.message);
    }
}

fn persist_key_store(cli: &Cli, store: &KeyStore) -> Result<Option<PathBuf>> {
    let path = cli.key_store.clone().or_else(KeyStore::default_path);
    let Some(path) = path else {
        return Ok(None);
    };
    store.save(&path)?;
    Ok(Some(path))
}

fn capture_keys(
    run: &mut Run<'_>,
    store: &mut KeyStore,
    card: &CardInfo,
    keys: &[AuthenticatedKey],
) -> CliResult<()> {
    if keys.is_empty() {
        return Ok(());
    }
    if let Err(error) = store.record_verified(card, keys) {
        warn!(run.out, "captured keys were not recorded: {error}");
        return Ok(());
    }
    match persist_key_store(run.cli, store) {
        Ok(Some(path)) => say!(
            run.out,
            "captured {} key(s) in {}",
            keys.len(),
            path.display()
        ),
        Ok(None) => {}
        Err(error) => warn!(run.out, "captured keys were not saved: {error}"),
    }
    Ok(())
}

fn capture_dump_metadata(
    run: &mut Run<'_>,
    store: &mut KeyStore,
    dump: &nfc_classic::Dump,
) -> CliResult<()> {
    let Some(card) = &dump.card else {
        return Ok(());
    };
    if dump.metadata.keys.is_empty() {
        return Ok(());
    }
    let recorded = match store.record_dump_keys(card, &dump.metadata.keys) {
        Ok(recorded) => recorded,
        Err(error) => {
            warn!(run.out, "dump keys were not recorded: {error}");
            return Ok(());
        }
    };
    match persist_key_store(run.cli, store) {
        Ok(Some(path)) => say!(
            run.out,
            "captured {recorded} dump key(s) in {}",
            path.display()
        ),
        Ok(None) => {}
        Err(error) => warn!(run.out, "dump keys were not saved: {error}"),
    }
    Ok(())
}

fn backup_store(cli: &Cli) -> BackupStore {
    match &cli.backup_dir {
        Some(directory) => BackupStore::new(directory),
        None => BackupStore::default(),
    }
}

fn parse_key_types(value: &str) -> CliResult<KeyTypes> {
    let normalized = value.to_ascii_uppercase().replace([' ', ',', '+'], "");
    match normalized.as_str() {
        "A" => Ok(KeyTypes::a_only()),
        "B" => Ok(KeyTypes::b_only()),
        "AB" | "BA" | "" => Ok(KeyTypes::ALL),
        other => Err(CliError::Usage(format!(
            "unknown key types {other:?}; use A, B, or AB"
        ))),
    }
}

fn parse_block_data(value: &str) -> CliResult<[u8; BLOCK_SIZE]> {
    let text = if let Some(path) = value.strip_prefix('@') {
        fs::read_to_string(path)
            .map_err(|error| NfcError::Io(format!("could not read {path}: {error}")))?
    } else {
        value.to_owned()
    };
    let compact: String = text
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != BLOCK_SIZE * 2 {
        return Err(CliError::Usage(format!(
            "block data must contain {} hexadecimal digits",
            BLOCK_SIZE * 2
        )));
    }
    let mut data = [0u8; BLOCK_SIZE];
    let bytes = compact.as_bytes();
    for index in 0..BLOCK_SIZE {
        data[index] = (hex_digit(bytes[index * 2])? << 4) | hex_digit(bytes[index * 2 + 1])?;
    }
    Ok(data)
}

fn hex_digit(character: u8) -> CliResult<u8> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(CliError::Usage(
            "block data contains a non-hexadecimal character".to_owned(),
        )),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

fn read_input(path: &Path) -> Result<Vec<u8>> {
    fs::read(path)
        .map_err(|error| NfcError::Io(format!("could not read {}: {error}", path.display())))
}

fn write_output(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            NfcError::Io(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    fs::write(path, bytes)
        .map_err(|error| NfcError::Io(format!("could not write {}: {error}", path.display())))?;
    restrict_permissions(path);
    Ok(())
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

fn build_service(mock: bool) -> Service {
    if mock {
        mock_service()
    } else {
        Service::with_transport(Arc::new(DriverRegistry))
    }
}

fn mock_device(name: &str, conn_string: &str) -> DeviceInfo {
    DeviceInfo::new(name, conn_string).with_physical_id(conn_string)
}

fn mock_service() -> Service {
    let transport = MockTransport::new();
    let (classic_card, classic_blocks) = mock_classic_card();
    let handle = transport.add(
        mock_device("Mock Classic 1K", "mock:classic-1k"),
        Some(classic_card),
    );
    for (block, data) in classic_blocks {
        handle.set_block(block, data);
    }
    let (ndef_card, ndef_blocks) = mock_classic_ndef_card();
    let ndef_handle = transport.add(
        mock_device("Mock Classic NDEF 1K", "mock:classic-ndef"),
        Some(ndef_card),
    );
    for (block, data) in ndef_blocks {
        ndef_handle.set_block(block, data);
    }
    let ntag = transport.add(
        mock_device("Mock NTAG215", "mock:ntag215"),
        Some(CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )),
    );
    for page in 0..135u8 {
        ntag.set_page(page, [0u8; PAGE_SIZE]);
    }
    ntag.set_page(0, [0x04, 0x5B, 0x1C, 0x2A]);
    ntag.set_page(1, [0x3B, 0x4C, 0x80, 0x00]);
    ntag.set_page(3, [0xE1, 0x10, 0x3E, 0x00]);
    ntag.set_page(4, [0x03, 0x00, 0xFE, 0x00]);
    ntag.set_page(130, [0x00, 0x00, 0x00, 0xBD]);
    ntag.set_page(131, [0x00, 0x00, 0x00, 0xFF]);
    ntag.set_page(132, [0x00, 0x00, 0x00, 0x00]);
    ntag.set_password([0xFF; PAGE_SIZE], [0x00, 0x00]);
    let amiibo = transport.add(
        mock_device("Mock blank Amiibo NTAG215", "mock:amiibo"),
        Some(CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )),
    );
    for page in 0..135u8 {
        amiibo.set_page(page, [0u8; PAGE_SIZE]);
    }
    amiibo.set_type2_version(Some([0x00, 0x04, 0x04, 0x02, 0x01, 0x00, 0x11, 0x03]));
    amiibo.set_page(0, [0x04, 0x5B, 0x1C, 0xCB]);
    amiibo.set_page(1, [0x2A, 0x3B, 0x4C, 0x80]);
    amiibo.set_page(2, [0xDD, 0x48, 0x00, 0x00]);
    amiibo.set_page(131, [0x00, 0x00, 0x00, 0xFF]);
    amiibo.set_password([0xFF; PAGE_SIZE], [0x00, 0x00]);
    let type4 = transport.add(
        mock_device("Mock Type 4 DESFire", "mock:type4-desfire"),
        Some(CardInfo::new(
            vec![0x04, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26],
            [0x00, 0x44],
            0x20,
            CardType::Iso14443,
        )),
    );
    let message = NdefMessage::single(
        TextRecord::new("NFC Foundry Type 4", "en")
            .expect("text record")
            .to_record()
            .expect("NDEF record"),
    )
    .expect("NDEF message");
    let encoded = message.encode().expect("encode NDEF");
    let mut ndef_file = (encoded.len() as u16).to_be_bytes().to_vec();
    ndef_file.extend_from_slice(&encoded);
    type4.set_type4_tag(MockType4Tag::new(mock_type4_cc(), ndef_file));
    transport.add(
        mock_device("Mock MIFARE Mini", "mock:mini"),
        Some(CardInfo::new(
            vec![0x04, 0x11, 0x22, 0x33],
            [0x00, 0x04],
            0x09,
            CardType::MifareMini,
        )),
    );
    transport.add(mock_device("Mock empty reader", "mock:empty"), None);
    let desfire = MockDesfireTransport::with_tag(
        MockDesfire::new([0x00; 8], AuthKeyType::Des)
            .with_key_version(0)
            .with_file(
                0x01,
                vec![0x00, 0x00, 0xEE, 0xEE, 0x20, 0x00, 0x00],
                vec![0u8; 32],
            ),
    );
    Service::with_transport(Arc::new(MockDevices {
        standard: transport,
        desfire,
        tunion: MockTUnionTransport::new(),
        emv: MockEmvTransport::new(),
    }))
}

/// Routes the scripted Classic/Type 2/Type 4 devices, the DESFire card, the
/// T-Union card, and the EMV payment card through one mock transport.
struct MockDevices {
    standard: MockTransport,
    desfire: MockDesfireTransport,
    tunion: MockTUnionTransport,
    emv: MockEmvTransport,
}

impl Transport for MockDevices {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        let mut devices = self.standard.enumerate()?;
        devices.extend(self.desfire.enumerate()?);
        devices.extend(self.tunion.enumerate()?);
        devices.extend(self.emv.enumerate()?);
        Ok(devices)
    }

    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>> {
        let is_desfire = self
            .desfire
            .enumerate()?
            .iter()
            .any(|device| device.conn_string == conn_string);
        if is_desfire {
            return self.desfire.open(conn_string);
        }
        let is_tunion = self
            .tunion
            .enumerate()?
            .iter()
            .any(|device| device.conn_string == conn_string);
        if is_tunion {
            return self.tunion.open(conn_string);
        }
        let is_emv = self
            .emv
            .enumerate()?
            .iter()
            .any(|device| device.conn_string == conn_string);
        if is_emv {
            return self.emv.open(conn_string);
        }
        self.standard.open(conn_string)
    }
}

fn mock_type4_cc() -> Vec<u8> {
    vec![
        0x00, 0x1B, 0x20, 0x00, 0x3A, 0x00, 0x34, // header
        0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00, 0x00, // NDEF file E104
        0x05, 0x06, 0xE1, 0x05, 0x00, 0x10, 0x00, 0x00, // proprietary file E105
        0x06, 0x02, 0xAA, 0xBB, // unknown TLV
    ]
}

fn mock_classic_ndef_card() -> (CardInfo, Vec<(u8, [u8; BLOCK_SIZE])>) {
    let (card, mut blocks) = mock_classic_card();
    let mad = Mad::blank(
        Layout::Classic1K,
        MadVersion::Mad1,
        &(1..=15).collect::<Vec<_>>(),
    )
    .expect("blank MAD");
    let mut by_block: std::collections::BTreeMap<u8, [u8; BLOCK_SIZE]> = blocks.drain(..).collect();
    for (block, data) in mad.encode(Layout::Classic1K).expect("encode MAD") {
        by_block.insert(block as u8, data);
    }
    let mut mad_trailer = [0xFF; BLOCK_SIZE];
    mad_trailer[..6].copy_from_slice(&[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]);
    mad_trailer[6..9].copy_from_slice(&[0x78, 0x77, 0x88]);
    mad_trailer[9] = 0xC1;
    by_block.insert(3, mad_trailer);
    for sector in 1..=15u8 {
        let mut trailer = [0xFF; BLOCK_SIZE];
        trailer[..6].copy_from_slice(&[0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7]);
        trailer[6..9].copy_from_slice(&[0x7F, 0x07, 0x88]);
        trailer[9] = 0x40;
        by_block.insert(sector * 4 + 3, trailer);
    }
    let message = NdefMessage::single(
        TextRecord::new("classic mock", "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message");
    let area = format_ndef_area(45 * BLOCK_SIZE).expect("area");
    let area = nfc_classic_ndef::layout::encode_ndef_area(
        &nfc_classic_ndef::ClassicNdefLayout {
            sectors: (1..=15).collect(),
            data_blocks: (1..=15)
                .flat_map(|sector| {
                    let first = sector * 4;
                    first..first + 3
                })
                .collect(),
            version: nfc_classic_ndef::MappingVersion::V1_0,
            read_only: false,
            writable: true,
            warnings: Vec::new(),
        },
        &area,
        &message,
    )
    .expect("NDEF area");
    let mut offset = 0;
    for sector in 1..=15u8 {
        for block in sector * 4..sector * 4 + 3 {
            let chunk: [u8; BLOCK_SIZE] =
                area[offset..offset + BLOCK_SIZE].try_into().expect("block");
            by_block.insert(block, chunk);
            offset += BLOCK_SIZE;
        }
    }
    (card, by_block.into_iter().collect())
}

fn mock_classic_card() -> (CardInfo, Vec<(u8, [u8; BLOCK_SIZE])>) {
    let uid = [0x04, 0xA2, 0x24, 0x8A];
    let card = CardInfo::new(uid.to_vec(), [0x00, 0x04], 0x08, CardType::MifareClassic1K);
    let mut blocks = Vec::with_capacity(Layout::Classic1K.block_count());
    let mut block_zero = [0u8; BLOCK_SIZE];
    block_zero[0..4].copy_from_slice(&uid);
    block_zero[4] = bcc_for_uid(&uid).expect("mock UID has a BCC");
    block_zero[5] = 0x08;
    block_zero[6..8].copy_from_slice(&[0x00, 0x04]);
    for block in 0..Layout::Classic1K.block_count() as u8 {
        let data = if block == 0 {
            block_zero
        } else if block % 4 == 3 {
            let mut trailer = [0xFF; BLOCK_SIZE];
            trailer[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
            trailer[9] = 0x69;
            trailer
        } else {
            [block; BLOCK_SIZE]
        };
        blocks.push((block, data));
    }
    (card, blocks)
}

fn supported_drivers() -> Vec<nfc_app::DriverInfo> {
    let mut drivers = vec![
        nfc_app::DriverInfo::new("PN532 UART", nfc_drivers::PN532_UART_PREFIX),
        nfc_app::DriverInfo::new("ACR122U USB", nfc_drivers::ACR122_USB_PREFIX),
    ];
    if nfc_drivers::ACR122_PCSC_ENABLED {
        drivers.push(nfc_app::DriverInfo::new(
            "ACR122U PC/SC",
            nfc_drivers::ACR122_PCSC_PREFIX,
        ));
    }
    drivers
}

fn pcsc_info() -> nfc_app::PcscInfo {
    let status = nfc_drivers::pcsc_status();
    nfc_app::PcscInfo {
        enabled: status.enabled,
        available: status.available,
        error: status.error,
        readers: status.readers,
    }
}

fn diag(run: &mut Run<'_>, service: &mut Service) -> CliResult<Value> {
    let report = service.self_check(
        supported_drivers(),
        pcsc_info(),
        run.cli.hardnested_tables.as_deref(),
    );
    say!(run.out, "NFC Foundry {}", report.app.version);
    say!(run.out, "nfctl {}", report.app.version);
    say!(run.out, "commit: {}", report.app.commit);
    say!(run.out, "build date: {}", report.app.build_date);
    say!(
        run.out,
        "platform: {} · {}",
        report.platform.os,
        report.platform.arch
    );
    say!(run.out, "drivers: {}", driver_summary(&report.drivers));
    if !report.pcsc.enabled {
        say!(run.out, "pcsc: not built");
    } else if report.pcsc.available {
        if report.pcsc.readers.is_empty() {
            say!(run.out, "pcsc: pcscd available · no ACR122U reader");
        } else {
            say!(
                run.out,
                "pcsc: pcscd available · {}",
                report.pcsc.readers.join(", ")
            );
        }
    } else {
        say!(
            run.out,
            "pcsc: unavailable ({})",
            report
                .pcsc
                .error
                .as_deref()
                .unwrap_or("PC/SC context failed")
        );
    }
    if !report.readers.available {
        say!(
            run.out,
            "readers: unavailable ({})",
            report
                .readers
                .error
                .as_deref()
                .unwrap_or("enumeration failed")
        );
    } else if report.readers.devices.is_empty() {
        say!(run.out, "readers: none found");
    } else {
        for device in &report.readers.devices {
            say!(run.out, "reader: {}\t{}", device.name, device.conn_string);
        }
    }
    if let Some(selected) = &report.readers.selected {
        say!(run.out, "selected: {selected} · {}", report.readers.status);
        match &report.readers.firmware {
            Some(firmware) => say!(run.out, "firmware: {firmware}"),
            None => say!(run.out, "firmware: reader not opened"),
        }
        say!(
            run.out,
            "capabilities: {}",
            capability_summary(&report.readers.capabilities)
        );
    }
    match (
        &report.hardnested_tables.path,
        &report.hardnested_tables.error,
    ) {
        (Some(path), _) => say!(run.out, "hardnested tables: {path}"),
        (None, Some(error)) => say!(run.out, "hardnested tables: not found ({error})"),
        (None, None) => say!(run.out, "hardnested tables: not found"),
    }
    say!(
        run.out,
        "build: {} · {} · {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let document = report.to_json().map_err(|error| {
        CliError::Internal(format!("could not serialize the self-check: {error}"))
    })?;
    Ok(serde_json::from_str(&document)?)
}

fn driver_summary(drivers: &[nfc_app::DriverInfo]) -> String {
    drivers
        .iter()
        .map(|driver| format!("{}<{}>", driver.name, driver.conn_string_prefix))
        .collect::<Vec<_>>()
        .join(", ")
}

fn capability_summary(capabilities: &nfc_app::CapabilityReport) -> String {
    let mut parts = Vec::new();
    if capabilities.classic_read {
        parts.push("classic read");
    }
    if capabilities.classic_write {
        parts.push("classic write");
    }
    if capabilities.type2_read {
        parts.push("type 2 read");
    }
    if capabilities.type2_write {
        parts.push("type 2 write");
    }
    if capabilities.raw_bytes {
        parts.push("raw bytes");
    }
    if capabilities.raw_bits {
        parts.push("raw bits");
    }
    if capabilities.manufacturer_write {
        parts.push("manufacturer write");
    }
    if capabilities.recovery_parity {
        parts.push("recovery parity");
    }
    if parts.is_empty() {
        "not opened".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn card_health(run: &mut Run<'_>, input: &Path, format: Option<&str>) -> CliResult<Value> {
    let bytes = read_input(input)?;
    let mut load = match format {
        Some(name) => exchange::load_as(&bytes, DumpFormat::parse(name).map_err(CliError::usage)?)?,
        None => exchange::load(&bytes)?,
    };
    let sidecar = adopt_sidecar(input, load.format, &mut load.dump);
    let mut store = load_key_store(run.cli)?;
    if let Some(card) = &load.dump.card {
        store.record_dump_keys(card, &load.dump.metadata.keys)?;
        run.out.set_card(card);
    }
    let health = classic_health(&load.dump, &store, &[]);
    say!(
        run.out,
        "image: {} · {} · {}",
        input.display(),
        load.format.name(),
        load.dump.layout.name()
    );
    if let Some(path) = &sidecar.path {
        say!(
            run.out,
            "sidecar: {} ({} key(s))",
            path.display(),
            sidecar.keys
        );
    }
    for warning in &sidecar.warnings {
        warn!(run.out, "{warning}");
    }
    if let Some(identity) = &health.identity {
        say!(
            run.out,
            "card: {} · UID {} ({} bytes) · ATQA {:02X}{:02X} · SAK {:02X}",
            identity.card_type.label(),
            identity.uid,
            identity.uid_length,
            identity.atqa[0],
            identity.atqa[1],
            identity.sak
        );
    } else {
        say!(run.out, "card: identity not recorded");
    }
    let layout = match health.layout {
        Some(Layout::Classic1K) => "MIFARE Classic 1K",
        Some(Layout::Classic4K) => "MIFARE Classic 4K",
        None => "unknown",
    };
    say!(run.out, "layout: {layout}");
    say!(
        run.out,
        "completeness: {}/{} blocks · {}/{} sectors · {}",
        health.readable_blocks,
        health.total_blocks,
        health.readable_sectors,
        health.total_sectors,
        if health.complete {
            "complete"
        } else {
            "incomplete"
        }
    );
    if !health.skipped_sectors.is_empty() {
        say!(
            run.out,
            "skipped sectors: {}",
            sector_list(&health.skipped_sectors)
        );
    }
    say!(
        run.out,
        "keys: {}/{} sectors covered (A {}, B {}, both {}) · {} dump key(s)",
        health.key_coverage.any_sectors,
        health.key_coverage.sectors,
        health.key_coverage.key_a_sectors,
        health.key_coverage.key_b_sectors,
        health.key_coverage.both_sectors,
        health.dump_keys
    );
    if let Some(device) = &health.device {
        say!(run.out, "read by: {device}");
    }
    if let (Some(started), Some(finished)) = (health.started_at, health.finished_at) {
        say!(run.out, "read at: {started}..={finished} (unix seconds)");
    }
    if health.dirty_blocks.is_empty() {
        say!(run.out, "dirty blocks: none");
    } else {
        say!(
            run.out,
            "dirty blocks: {}",
            sector_list(&health.dirty_blocks)
        );
    }
    for warning in &load.warnings {
        warn!(run.out, "{warning}");
    }
    for warning in &health.warnings {
        warn!(run.out, "{warning}");
    }
    for error in &health.errors {
        note!(run.out, "error: {error}");
    }
    if health.errors.is_empty() {
        say!(run.out, "validation: ok");
    } else {
        say!(run.out, "validation: {} error(s)", health.errors.len());
    }
    let result = json!({
        "path": input.display().to_string(),
        "format": load.format.name(),
        "layout": layout,
        "identity": health.identity.as_ref().map(|identity| json!({
            "type": output::card_type_code(identity.card_type),
            "uid": identity.uid.replace(' ', ""),
            "uidLength": identity.uid_length,
            "atqa": format!("{:02X}{:02X}", identity.atqa[0], identity.atqa[1]),
            "sak": format!("{:02X}", identity.sak),
        })),
        "readableBlocks": health.readable_blocks,
        "totalBlocks": health.total_blocks,
        "readableSectors": health.readable_sectors,
        "totalSectors": health.total_sectors,
        "complete": health.complete,
        "skippedSectors": health.skipped_sectors,
        "keyCoverage": {
            "sectors": health.key_coverage.sectors,
            "any": health.key_coverage.any_sectors,
            "keyA": health.key_coverage.key_a_sectors,
            "keyB": health.key_coverage.key_b_sectors,
            "both": health.key_coverage.both_sectors,
            "dumpKeys": health.dump_keys,
        },
        "device": health.device,
        "startedAt": health.started_at,
        "finishedAt": health.finished_at,
        "dirtyBlocks": health.dirty_blocks,
        "warnings": health.warnings,
        "errors": health.errors,
        "valid": health.errors.is_empty(),
    });
    if !health.errors.is_empty() {
        return Err(CliError::Validation(format!(
            "{} validation error(s): {}",
            health.errors.len(),
            health.errors.join("; ")
        )));
    }
    Ok(result)
}

fn sector_list(values: &[usize]) -> String {
    values
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn recovery_strategy_name(strategy: RecoveryStrategy) -> &'static str {
    match strategy {
        RecoveryStrategy::Auto => "automatic",
        RecoveryStrategy::Dictionary => "dictionary",
        RecoveryStrategy::DarkSide => "darkside",
        RecoveryStrategy::Nested => "nested",
        RecoveryStrategy::StaticNested => "static nested",
        RecoveryStrategy::HardNested => "hardnested",
        RecoveryStrategy::Fm11rf08s => "FM11RF08S",
    }
}

/// Developer-only raw PN532 console.
///
/// Compiled with `developer-tools` only and never exposed by the GUI. Each
/// line is one native command frame; the session does not select a card or run
/// any workflow.
#[cfg(feature = "developer-tools")]
fn pn532_shell(run: &mut Run<'_>, service: &mut Service) -> CliResult<Value> {
    if !run.out.mode().is_human() {
        return Err(CliError::Usage(
            "`pn532 shell` is interactive and does not support --json or --jsonl".to_owned(),
        ));
    }
    prepare(run, service)?;
    warn!(
        run.out,
        "developer console: raw PN532 commands bypass card workflows and can leave the reader in an unknown state"
    );
    say!(run.out, "Hex command bytes, `help`, or `quit`.");
    let mut lease = service.begin_diagnostics(run.cancel.clone())?;
    let interactive = io::stdin().is_terminal();
    loop {
        run.cancel.check()?;
        if interactive {
            say_inline!(run.out, "pn532> ");
            io::stdout().flush()?;
        }
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }
        let input = line.trim();
        match input {
            "" => continue,
            "quit" | "exit" => break,
            "help" | "?" => {
                say!(
                    run.out,
                    "one line is one native command frame; quit or exit leaves the console"
                );
                continue;
            }
            _ => {}
        }
        let command = match parse_hex_bytes(input, "command") {
            Ok(command) => command,
            Err(error) => {
                note!(run.out, "error: {}", error.message());
                continue;
            }
        };
        match lease.raw_command(&command) {
            Ok(response) if response.is_empty() => say!(run.out, "response: (empty)"),
            Ok(response) => say!(run.out, "response: {}", hex_bytes(&response)),
            Err(error) => note!(run.out, "error: {error}"),
        }
    }
    Ok(json!({}))
}

/// Developer-only raw APDU exchange. The request is never echoed: an
/// experimental APDU may carry key material, and the envelope must not export
/// it.
#[cfg(feature = "developer-tools")]
fn apdu_raw(run: &mut Run<'_>, service: &mut Service, raw: &str) -> CliResult<Value> {
    let apdu = parse_hex_bytes(raw, "APDU")?;
    prepare(run, service)?;
    warn!(
        run.out,
        "raw APDU is a developer diagnostic, not a safe write path"
    );
    let mut lease = service.begin_diagnostics(run.cancel.clone())?;
    let (card, response) = lease.raw_apdu(&apdu)?;
    run.out.set_card(&card);
    let data = compact_hex(&response.data);
    let sw = format!("{:02X}{:02X}", response.sw[0], response.sw[1]);
    if data.is_empty() {
        say!(run.out, "data: (none)");
    } else {
        say!(run.out, "data: {data}");
    }
    say!(run.out, "sw: {sw}");
    Ok(json!({
        "data": data,
        "sw": sw,
        "statusOk": response.is_ok(),
    }))
}

fn list(run: &mut Run<'_>, service: &mut Service) -> CliResult<Value> {
    run.out.set_command("list");
    service.refresh_devices()?;
    let devices: Vec<Value> = service
        .devices()
        .iter()
        .map(|device| {
            json!({
                "name": device.name,
                "connString": device.conn_string,
            })
        })
        .collect();
    if service.devices().is_empty() {
        say!(run.out, "no readers found");
    } else {
        for device in service.devices() {
            say!(run.out, "{}\t{}", device.name, device.conn_string);
        }
    }
    Ok(json!({ "devices": devices }))
}

fn prepare(run: &mut Run<'_>, service: &mut Service) -> CliResult<()> {
    let environment = environment_device()?;
    service.refresh_devices()?;
    let conn_string = resolve_device(
        run.cli.device.as_deref(),
        environment.as_deref(),
        service.devices(),
    )
    .map_err(selection_error)?;
    service
        .select_device(&conn_string)
        .map_err(|error| match error {
            NfcError::InvalidArgument(message) => CliError::Usage(message),
            other => CliError::Service(other),
        })?;
    run.out.set_device(&conn_string);
    let snapshot = service.reader_state();
    run.out.verbose(format_args!(
        "reader: {conn_string} · {} · {}",
        snapshot.status.label(),
        capability_summary(&nfc_app::CapabilityReport::from(snapshot.capabilities))
    ));
    if let Some(firmware) = &snapshot.firmware {
        run.out.verbose(format_args!("firmware: {firmware}"));
    }
    Ok(())
}

/// Reads `NFC_FOUNDRY_DEVICE`. An unset or empty value leaves selection to
/// auto-selection; a non-Unicode value is a usage error because it can never
/// match an enumerated connstring.
fn environment_device() -> CliResult<Option<String>> {
    match std::env::var_os(select::DEVICE_ENV) {
        None => Ok(None),
        Some(value) => match value.into_string() {
            Ok(value) if value.is_empty() => Ok(None),
            Ok(value) => Ok(Some(value)),
            Err(_) => Err(CliError::Usage(format!(
                "{} must be valid UTF-8",
                select::DEVICE_ENV
            ))),
        },
    }
}

fn selection_error(error: SelectionError) -> CliError {
    let message = error.message();
    match error {
        SelectionError::NoReaders => CliError::NoReader(message),
        SelectionError::Ambiguous { .. } | SelectionError::Unknown { .. } => {
            CliError::Usage(message)
        }
    }
}

fn watch_card(run: &mut Run<'_>, service: &mut Service) -> CliResult<()> {
    let mut last_error: Option<String> = None;
    loop {
        run.cancel.check()?;
        let update = service.poll_reader();
        for event in update.events {
            match event {
                ReaderEvent::CardAppeared(card) => {
                    run.out.set_card(&card);
                    print_card(run.out, &card);
                    run.out.event(json!({
                        "type": "card",
                        "event": "appeared",
                        "card": output::card_facts(&card),
                    }));
                }
                ReaderEvent::CardRemoved(card) => {
                    run.out.set_card(&card);
                    say!(run.out, "card removed: {}", card.uid_hex());
                    run.out.event(json!({
                        "type": "card",
                        "event": "removed",
                        "card": output::card_facts(&card),
                    }));
                }
                ReaderEvent::StatusChanged(ReaderStatus::Recovering) => {
                    say!(run.out, "reader recovering");
                }
                ReaderEvent::StatusChanged(ReaderStatus::Error) => {
                    say!(run.out, "reader error");
                }
                ReaderEvent::StatusChanged(_) => {}
            }
        }
        let error = update.snapshot.error;
        if error != last_error {
            if let Some(message) = &error {
                note!(run.out, "reader: {message}");
            }
            last_error = error;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn print_card(out: &Output, card: &CardInfo) {
    say!(out, "type: {}", card.card_type.label());
    say!(out, "uid: {}", card.uid_hex());
    say!(out, "atqa: {:02X} {:02X}", card.atqa[0], card.atqa[1]);
    say!(out, "sak: {:02X}", card.sak);
    if card.card_type == CardType::MifareMini {
        say!(
            out,
            "note: MIFARE Mini is identified only; this product does not read or write it"
        );
    }
    if card.card_type == CardType::Unknown {
        say!(
            out,
            "note: only UID, ATQA, and SAK are available; a UID-only phone emulation does not implement MIFARE commands"
        );
    }
    if card.card_type == CardType::Iso14443 {
        say!(
            out,
            "note: ISO/IEC 14443-4 card; the Classic and Type 2 read paths do not apply"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{confirmation_answer, parse_hex_bytes};

    #[test]
    fn desfire_key_parser_accepts_compact_and_spaced_hex() {
        assert_eq!(
            parse_hex_bytes("0000000000000000", "key").expect("des"),
            vec![0u8; 8]
        );
        assert_eq!(
            parse_hex_bytes("00:00 00 00 00 00 00 00 00 00 00 00 00 00 00 00", "key")
                .expect("2k3des"),
            vec![0u8; 16]
        );
        assert!(parse_hex_bytes("ABC", "key").is_err());
        assert!(parse_hex_bytes("", "key").is_err());
        assert!(parse_hex_bytes("GG", "key").is_err());
    }

    #[test]
    fn confirmation_accepts_yes_in_any_case() {
        for answer in ["", " ", "y", "Y", "yes", "YES", " Yes ", "yEs"] {
            assert!(confirmation_answer(answer), "{answer:?}");
        }
        for answer in ["n", "N", "no", "1", "yeah"] {
            assert!(!confirmation_answer(answer), "{answer:?}");
        }
    }
}
