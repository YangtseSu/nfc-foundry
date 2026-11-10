use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use nfc_app::Service;
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicNdefFormat, ClassicNdefRead,
    ClassicNdefWrite, ClassicRead, ClassicRestore, KeyEntry, KeyKind, KeySource, KeyStore,
    KeyTypes, common_keys,
};
use nfc_app::type2::{
    Type2Format, Type2Lock, Type2NdefWrite, Type2PageWrite, Type2Password, Type2Restore,
};
use nfc_classic::{DumpFormat, Layout, bcc_for_uid, diff, exchange};
use nfc_classic_ndef::{
    ClassicNdefInfo, Mad, MadVersion, NdefState as ClassicNdefState, format_ndef_area,
};
use nfc_core::{
    BLOCK_SIZE, CancelToken, CardInfo, CardType, Key, KeyType, NfcError, PAGE_SIZE, Result,
};
use nfc_drivers::DriverRegistry;
use nfc_ndef::{NdefMessage, NdefRecord, TextRecord, UriRecord};
use nfc_recover::auto::RecoveryStrategy;
use nfc_recover::{RecoveryProgress, RecoveryRequest};
use nfc_transport::DeviceInfo;
use nfc_transport::mock::MockTransport;
use nfc_type2::{
    Configuration, LockBits, NdefState, RestoreOptions, Type2Image, exchange as type2,
};

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
    /// Exact reader connstring.
    #[arg(long, global = true, value_name = "CONNSTRING")]
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
    /// Do not try the built-in common keys.
    #[arg(long, global = true)]
    no_common_keys: bool,
    /// Hardnested table directory. Defaults to the installed resource path.
    #[arg(long, global = true, value_name = "DIR")]
    hardnested_tables: Option<PathBuf>,
    /// Hardnested acquisition cap; zero uses the engine default.
    #[arg(long, global = true, value_name = "N", default_value_t = 0)]
    max_nonces: usize,
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
    /// Run the self-check.
    Diag,
}

#[derive(Clone, Debug, Subcommand)]
enum CardCommand {
    Info,
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
        #[arg(long, default_value = "project", value_name = "FORMAT")]
        format: String,
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
    /// Restore a dump after a backup.
    Restore {
        #[arg(long, short, value_name = "PATH")]
        input: PathBuf,
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Required; restore mutates the card.
        #[arg(long)]
        confirm: bool,
        /// Continue after a failed block instead of stopping.
        #[arg(long)]
        continue_on_error: bool,
    },
    /// Try candidate keys against every sector.
    Keyscan {
        /// Save discovered keys to this store.
        #[arg(long, value_name = "PATH")]
        save: Option<PathBuf>,
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
        /// Use I_OWN_THIS_CARD.
        #[arg(long, value_name = "TOKEN")]
        confirm: Option<String>,
        /// Write the read-only GPB and access bits.
        #[arg(long)]
        read_only: bool,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum KeysCommand {
    /// List stored keys.
    List,
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
        /// Required for Advanced pages; use I_OWN_THIS_CARD.
        #[arg(long, value_name = "TOKEN")]
        confirm: Option<String>,
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
        /// Use I_OWN_THIS_CARD.
        #[arg(long, value_name = "TOKEN")]
        confirm: Option<String>,
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
        /// Use I_OWN_THIS_CARD.
        #[arg(long, value_name = "TOKEN")]
        confirm: Option<String>,
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
        /// Use I_OWN_THIS_CARD.
        #[arg(long, value_name = "TOKEN")]
        confirm: Option<String>,
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
        /// Use I_OWN_THIS_CARD.
        #[arg(long, value_name = "TOKEN")]
        confirm: Option<String>,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum RecoveryCommand {
    /// Classify the card and try dictionary, nested, or darkside recovery.
    Auto {
        #[arg(long, default_value = "4", value_name = "BLOCK")]
        known_block: usize,
        #[arg(long, default_value = "0", value_name = "BLOCK")]
        target_block: usize,
        #[arg(long, default_value = "A", value_name = "A|B")]
        target_type: String,
    },
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
        target_type: String,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let mut service = build_service(cli.mock);
    match cli.command.clone() {
        Command::List => list(&mut service),
        Command::Scan { watch } => {
            prepare(&mut service, cli.device.as_deref())?;
            if watch {
                watch_card(&mut service)
            } else {
                print_card(&service.scan()?);
                Ok(())
            }
        }
        Command::Card {
            command: CardCommand::Info,
        } => {
            prepare(&mut service, cli.device.as_deref())?;
            print_card(&service.card_info()?);
            Ok(())
        }
        Command::Classic { command } => classic(&cli, &mut service, command),
        Command::Type2 { command } => type2(&cli, &mut service, command),
        Command::Ndef { command } => ndef(&cli, &mut service, command),
        Command::Recover { command } => recovery(&cli, &mut service, command),
        Command::Diag => diag(),
    }
}

fn recovery(cli: &Cli, service: &mut Service, command: RecoveryCommand) -> Result<()> {
    let (strategy, known_block, target_block, target_type) = match command {
        RecoveryCommand::Auto {
            known_block,
            target_block,
            target_type,
        } => (
            RecoveryStrategy::Auto,
            known_block,
            target_block,
            target_type,
        ),
        RecoveryCommand::Darkside {
            target_block,
            target_type,
        } => (RecoveryStrategy::DarkSide, 4, target_block, target_type),
        RecoveryCommand::Nested {
            known_block,
            target_block,
            target_type,
        } => (
            RecoveryStrategy::Nested,
            known_block,
            target_block,
            target_type,
        ),
        RecoveryCommand::StaticNested {
            known_block,
            target_block,
            target_type,
        } => (
            RecoveryStrategy::StaticNested,
            known_block,
            target_block,
            target_type,
        ),
        RecoveryCommand::HardNested {
            known_block,
            target_block,
            target_type,
        } => (
            RecoveryStrategy::HardNested,
            known_block,
            target_block,
            target_type,
        ),
        RecoveryCommand::Fm11rf08s {
            known_block,
            target_type,
        } => (RecoveryStrategy::Fm11rf08s, known_block, 0, target_type),
    };
    prepare(service, cli.device.as_deref())?;
    let candidates = recovery_candidates(cli)?;
    if candidates.is_empty() {
        eprintln!("warning: no candidate keys; nested and hardnested recovery need a known key");
    }
    let printer = Arc::new(CliProgress::default());
    let sink = Arc::clone(&printer);
    let request = RecoveryRequest {
        known_block: u8::try_from(known_block).map_err(|_| {
            NfcError::InvalidArgument(format!("known block {known_block} is outside 0..=255"))
        })?,
        target_block: u8::try_from(target_block).map_err(|_| {
            NfcError::InvalidArgument(format!("target block {target_block} is outside 0..=255"))
        })?,
        target_key_type: parse_key_type(&target_type)?,
        known_key_type: KeyType::A,
        candidates,
        cancel: CancelToken::new(),
        progress: Some(Arc::new(move |event| sink.report(event))),
        tables_dir: cli.hardnested_tables.clone(),
        max_nonces: cli.max_nonces,
    };
    let result = service.recover(strategy, request)?;
    if let Some(profile) = &result.profile {
        println!(
            "profile: prng={:?} known_key={}",
            profile.prng, profile.has_known_key
        );
    }
    for (key_type, key) in &result.keys {
        println!("verified key {}: {}", key_type.label(), key.to_hex());
    }
    println!("strategy: {:?}", result.attempts);
    if !result.note.is_empty() {
        println!("{}", result.note);
    }
    Ok(())
}

#[derive(Default)]
struct CliProgress {
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
    fn report(&self, event: RecoveryProgress) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let now = std::time::Instant::now();
        let stage = match event {
            RecoveryProgress::Acquiring { .. } => 1,
            RecoveryProgress::Filtering { .. } => 2,
            RecoveryProgress::BruteForce { .. } => 3,
        };
        let elapsed = state
            .last
            .is_none_or(|last| now.duration_since(last).as_secs() >= 2);
        match event {
            RecoveryProgress::Acquiring {
                received,
                distinct_nonces,
                first_bytes,
                expected_states,
            } => {
                if state.last_stage != stage
                    || elapsed
                    || received.saturating_sub(state.last_received) >= 64
                {
                    eprintln!(
                        "acquire: {received} frames, {distinct_nonces} nonces, {first_bytes}/256 first bytes, estimated {expected_states:.3e} states"
                    );
                    state.last = Some(now);
                    state.last_received = received;
                }
            }
            RecoveryProgress::Filtering { expected_states } => {
                if state.last_stage != stage {
                    eprintln!("filter: estimated {expected_states:.3e} states");
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
                if state.last_stage != stage
                    || percent >= state.last_percent + 10
                    || (elapsed && percent != state.last_percent)
                {
                    eprintln!(
                        "brute force: {percent}% ({tested_states}/{total_states}), expected {expected_states:.3e} states"
                    );
                    state.last_percent = percent;
                    state.last = Some(now);
                }
            }
        }
        state.last_stage = stage;
    }
}

fn parse_key_type(value: &str) -> Result<KeyType> {
    match value.trim().to_ascii_uppercase().as_str() {
        "A" => Ok(KeyType::A),
        "B" => Ok(KeyType::B),
        other => Err(NfcError::InvalidArgument(format!(
            "unknown key type {other:?}; use A or B"
        ))),
    }
}

fn recovery_candidates(cli: &Cli) -> Result<Vec<(KeyType, Key)>> {
    let store = load_key_store(cli)?;
    let mut candidates = Vec::new();
    for entry in store.entries() {
        for key_type in [KeyType::A, KeyType::B] {
            if entry.kind.includes(key_type) {
                candidates.push((key_type, entry.key));
            }
        }
    }
    if !cli.no_common_keys {
        for key in common_keys() {
            for key_type in [KeyType::A, KeyType::B] {
                candidates.push((key_type, key));
            }
        }
    }
    candidates.sort_by_key(|(key_type, key)| (key_type.label(), key.to_hex()));
    candidates.dedup();
    Ok(candidates)
}

fn classic(cli: &Cli, service: &mut Service, command: ClassicCommand) -> Result<()> {
    match command {
        ClassicCommand::Read { block } => classic_read(cli, service, block),
        ClassicCommand::Dump { output, format } => classic_dump(cli, service, &output, &format),
        ClassicCommand::Write {
            block,
            data,
            confirm_trailer,
        } => classic_write(cli, service, block, &data, confirm_trailer),
        ClassicCommand::Restore {
            input,
            format,
            confirm,
            continue_on_error,
        } => classic_restore(
            cli,
            service,
            &input,
            format.as_deref(),
            confirm,
            continue_on_error,
        ),
        ClassicCommand::Keyscan { save, types } => {
            classic_keyscan(cli, service, save.as_deref(), &types)
        }
        ClassicCommand::Keys { command } => classic_keys(cli, command),
        ClassicCommand::Diff { before, after } => classic_diff(&before, &after),
        ClassicCommand::Ndef { command } => classic_ndef(cli, service, command),
    }
}

fn classic_read(cli: &Cli, service: &mut Service, block: usize) -> Result<()> {
    prepare(service, cli.device.as_deref())?;
    let keys = load_key_store(cli)?;
    let data = service.classic_read_block(block, &keys, !cli.no_common_keys, CancelToken::new())?;
    println!("block {block}: {}", hex_bytes(&data));
    println!("ascii: {}", ascii(&data));
    Ok(())
}

fn classic_dump(cli: &Cli, service: &mut Service, output: &Path, format: &str) -> Result<()> {
    prepare(service, cli.device.as_deref())?;
    let keys = load_key_store(cli)?;
    let format = DumpFormat::parse(format)?;
    let result =
        service.classic_read(ClassicRead::new(&keys).include_common_keys(!cli.no_common_keys))?;
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    let bytes = exchange::export(&result.dump, format)?;
    write_output(output, &bytes)?;
    let readability = result.dump.readability();
    println!(
        "{}: {}/{} blocks, {}/{} sectors readable",
        result.dump.layout.name(),
        readability.readable_blocks,
        readability.total_blocks,
        readability.readable_sectors,
        readability.total_sectors
    );
    if !readability.skipped_sectors.is_empty() {
        println!(
            "skipped sectors: {}",
            readability
                .skipped_sectors
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("wrote {} ({} bytes)", output.display(), bytes.len());
    Ok(())
}

fn classic_write(
    cli: &Cli,
    service: &mut Service,
    block: usize,
    data: &str,
    confirm_trailer: bool,
) -> Result<()> {
    prepare(service, cli.device.as_deref())?;
    let keys = load_key_store(cli)?;
    let backup = backup_store(cli);
    let block_data = parse_block_data(data)?;
    let result = service.classic_write_block(
        ClassicBlockWrite::new(block, block_data, &keys, &backup)
            .include_common_keys(!cli.no_common_keys)
            .confirm_trailer(confirm_trailer),
    )?;
    if result.changed {
        println!("block {} written and verified", result.block);
        if let Some(backup) = result.backup {
            println!("backup: {}", backup.path.display());
        }
    } else {
        println!("block {} already contains the requested data", result.block);
    }
    Ok(())
}

fn classic_restore(
    cli: &Cli,
    service: &mut Service,
    input: &Path,
    format: Option<&str>,
    confirm: bool,
    continue_on_error: bool,
) -> Result<()> {
    prepare(service, cli.device.as_deref())?;
    let bytes = fs::read(input)
        .map_err(|error| NfcError::Io(format!("could not read {}: {error}", input.display())))?;
    let load = match format {
        Some(format) => exchange::load_as(&bytes, DumpFormat::parse(format)?)?,
        None => exchange::load(&bytes)?,
    };
    for warning in &load.warnings {
        eprintln!("warning: {warning}");
    }
    let keys = load_key_store(cli)?;
    let backup = backup_store(cli);
    let result = service.classic_restore(
        ClassicRestore::new(load.dump, &keys, &backup)
            .include_common_keys(!cli.no_common_keys)
            .confirmed(confirm)
            .continue_on_failure(continue_on_error),
    )?;
    println!(
        "restore: {} written, {} unchanged, {} unknown skipped, {} trailers",
        result.written_blocks,
        result.unchanged_blocks,
        result.skipped_unknown_blocks,
        result.trailer_writes
    );
    if let Some(backup) = result.backup {
        println!("backup: {}", backup.path.display());
    }
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    for failure in &result.failures {
        eprintln!("failure: {failure}");
    }
    if !result.failures.is_empty() {
        return Err(NfcError::VerificationFailed(format!(
            "restore finished with {} failure(s)",
            result.failures.len()
        )));
    }
    Ok(())
}

fn classic_keyscan(
    cli: &Cli,
    service: &mut Service,
    save: Option<&Path>,
    types: &str,
) -> Result<()> {
    prepare(service, cli.device.as_deref())?;
    let keys = load_key_store(cli)?;
    let key_types = parse_key_types(types)?;
    let result = service.classic_keyscan(
        ClassicKeyscan::new(&keys, Vec::new())
            .include_common_keys(!cli.no_common_keys)
            .key_types(key_types),
    )?;
    for sector in &result.sectors {
        match (sector.key_a, sector.key_b) {
            (Some(a), Some(b)) => println!(
                "sector {:<2} A={} B={}",
                sector.sector,
                a.to_hex(),
                b.to_hex()
            ),
            (Some(a), None) => println!("sector {:<2} A={}", sector.sector, a.to_hex()),
            (None, Some(b)) => println!("sector {:<2} B={}", sector.sector, b.to_hex()),
            (None, None) => println!("sector {:<2} no key found", sector.sector),
        }
    }
    println!(
        "attempts: {}, verified: {}",
        result.attempts, result.verified
    );

    let save_path = save
        .map(Path::to_path_buf)
        .or_else(|| cli.key_store.clone());
    if let Some(path) = save_path {
        let mut store = KeyStore::load_or_default(&path)?;
        let added = store.merge(&result.store)?;
        store.save(&path)?;
        println!("saved {added} key(s) to {}", path.display());
    } else {
        println!("pass --save or --key-store to keep discovered keys");
    }
    Ok(())
}

fn classic_keys(cli: &Cli, command: KeysCommand) -> Result<()> {
    match command {
        KeysCommand::List => {
            let (store, _path) = management_store(cli, false)?;
            if store.is_empty() {
                println!("key store is empty");
                return Ok(());
            }
            for entry in store.entries() {
                println!(
                    "{:<12} {:<4} {} ({})",
                    entry.sector_label(),
                    entry.kind.label(),
                    entry.key.to_hex(),
                    entry.source.label()
                );
            }
            Ok(())
        }
        KeysCommand::Import { file } => {
            let (mut store, path) = management_store(cli, true)?;
            let added = store.import_text(&file)?;
            store.save(&path)?;
            println!("imported {added} key(s) into {}", path.display());
            Ok(())
        }
        KeysCommand::Add {
            hex,
            sector,
            key_type,
        } => {
            let (mut store, path) = management_store(cli, true)?;
            let key = Key::from_hex(&hex)?;
            let kind = KeyKind::parse(&key_type)?;
            let added = store.add(KeyEntry::new(sector, kind, key, KeySource::Manual))?;
            store.save(&path)?;
            if added {
                println!("added key to {}", path.display());
            } else {
                println!("key is already present in {}", path.display());
            }
            Ok(())
        }
    }
}

fn classic_diff(before: &Path, after: &Path) -> Result<()> {
    let before = exchange::load(&read_input(before)?)?.dump;
    let after = exchange::load(&read_input(after)?)?.dump;
    let diff = diff::diff(&before, &after)?;
    println!(
        "{}: {} changed, {} equal, {} unknown",
        diff.layout.name(),
        diff.changed_blocks,
        diff.equal_blocks,
        diff.unknown_blocks
    );
    println!("trailer changes: {}", diff.trailer_changes);
    if diff.identity_changed {
        println!("card identity changed");
    }
    for block in diff.blocks.iter() {
        if block.state == diff::DiffState::Equal {
            continue;
        }
        let state = match block.state {
            diff::DiffState::Equal => "equal",
            diff::DiffState::Changed => "changed",
            diff::DiffState::Unknown => "unknown",
        };
        println!(
            "block {:<3} sector {:<2} {:?} {state}",
            block.block, block.sector, block.kind
        );
        for byte in &block.bytes {
            println!(
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
        }
        if let Some(trailer) = &block.trailer
            && !trailer.valid
        {
            println!(
                "  trailer: {}",
                trailer.error.as_deref().unwrap_or("invalid")
            );
        }
    }
    Ok(())
}

fn classic_ndef(cli: &Cli, service: &mut Service, command: ClassicNdefCommand) -> Result<()> {
    match command {
        ClassicNdefCommand::Info => {
            prepare(service, cli.device.as_deref())?;
            let keys = load_key_store(cli)?;
            let read = service.classic_ndef_read(
                ClassicNdefRead::new(&keys).include_common_keys(!cli.no_common_keys),
            )?;
            for warning in &read.warnings {
                eprintln!("warning: {warning}");
            }
            print_classic_ndef(&read.info);
            Ok(())
        }
        ClassicNdefCommand::Read { output } => {
            prepare(service, cli.device.as_deref())?;
            let keys = load_key_store(cli)?;
            let read = service.classic_ndef_read(
                ClassicNdefRead::new(&keys).include_common_keys(!cli.no_common_keys),
            )?;
            for warning in &read.warnings {
                eprintln!("warning: {warning}");
            }
            print_classic_ndef(&read.info);
            match &read.info.ndef.state {
                ClassicNdefState::Message(message) => {
                    for (index, record) in message.records().iter().enumerate() {
                        print_ndef_record(index, record);
                    }
                    if let Some(path) = output {
                        let bytes = message.encode()?;
                        write_output(&path, &bytes)?;
                        println!("wrote {} ({} bytes)", path.display(), bytes.len());
                    }
                }
                ClassicNdefState::Empty => println!("NDEF: empty"),
                ClassicNdefState::Unformatted => println!("NDEF: unformatted"),
            }
            Ok(())
        }
        ClassicNdefCommand::Write {
            file,
            text,
            uri,
            language,
        } => {
            prepare(service, cli.device.as_deref())?;
            let keys = load_key_store(cli)?;
            let backup = backup_store(cli);
            let message = build_ndef_message(file.as_deref(), text, uri, &language)?;
            let result = service.classic_ndef_write(
                ClassicNdefWrite::new(message, &keys, &backup)
                    .include_common_keys(!cli.no_common_keys),
            )?;
            print_classic_ndef(&result.info);
            println!(
                "NDEF written: {} block(s), backup {}",
                result.written_blocks,
                result.backup.path.display()
            );
            Ok(())
        }
        ClassicNdefCommand::Format { confirm, read_only } => {
            prepare(service, cli.device.as_deref())?;
            if confirm.as_deref() != Some(CONFIRMATION) {
                return Err(NfcError::InvalidArgument(format!(
                    "formatting replaces sector keys and access bits; pass --confirm {CONFIRMATION}"
                )));
            }
            let keys = load_key_store(cli)?;
            let backup = backup_store(cli);
            let result = service.classic_ndef_format(
                ClassicNdefFormat::new(&keys, &backup)
                    .include_common_keys(!cli.no_common_keys)
                    .read_only(read_only)
                    .confirm(true),
            )?;
            print_classic_ndef(&result.info);
            println!(
                "NDEF format: {} block(s) written, {} trailer(s), backup {}",
                result.written_blocks,
                result.trailer_writes,
                result.backup.path.display()
            );
            Ok(())
        }
    }
}

fn print_classic_ndef(info: &ClassicNdefInfo) {
    match &info.layout {
        Some(layout) => {
            println!("mapping: {}.{}", layout.version.major, layout.version.minor);
            println!(
                "NFC sectors: {}",
                layout
                    .sectors
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
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
                eprintln!("warning: {warning}");
            }
        }
        None => println!("MAD: absent"),
    }
    match &info.ndef.state {
        ClassicNdefState::Unformatted => println!("NDEF: unformatted"),
        ClassicNdefState::Empty => println!("NDEF: empty, capacity {} bytes", info.ndef.capacity),
        ClassicNdefState::Message(message) => println!(
            "NDEF: {} record(s), {} / {} bytes",
            message.records().len(),
            info.ndef.message_bytes,
            info.ndef.capacity
        ),
    }
}

const CONFIRMATION: &str = "I_OWN_THIS_CARD";

fn type2(cli: &Cli, service: &mut Service, command: Type2Command) -> Result<()> {
    match command {
        Type2Command::Info => {
            prepare(service, cli.device.as_deref())?;
            let read = service.type2_read(CancelToken::new())?;
            print_type2_info(&read.image, &read.warnings);
            Ok(())
        }
        Type2Command::Read { page } => {
            prepare(service, cli.device.as_deref())?;
            if let Some(page) = page {
                let image = service.type2_read(CancelToken::new())?.image;
                let data = image.page(page).ok_or_else(|| {
                    NfcError::InvalidArgument(format!("page {page} is not readable"))
                })?;
                println!(
                    "page {page} {:>14}: {}",
                    image.role(page).label(),
                    hex_bytes(data)
                );
                return Ok(());
            }
            let read = service.type2_read(CancelToken::new())?;
            for (page, data) in read.image.pages() {
                let data = data
                    .map(|data| hex_bytes(data))
                    .unwrap_or_else(|| "--".to_owned());
                println!(
                    "page {:<3} {:>14} {data}",
                    page,
                    read.image.role(page).label()
                );
            }
            for warning in &read.warnings {
                println!("warning: {warning}");
            }
            Ok(())
        }
        Type2Command::Write {
            page,
            data,
            advanced,
            confirm,
        } => {
            prepare(service, cli.device.as_deref())?;
            let backup = backup_store(cli);
            let data: [u8; PAGE_SIZE] = parse_hex_array(&data, "page data")?;
            if advanced && confirm.as_deref() != Some(CONFIRMATION) {
                return Err(NfcError::InvalidArgument(format!(
                    "writing an Advanced page needs --confirm {CONFIRMATION}"
                )));
            }
            let result = service.type2_write_page(
                Type2PageWrite::new(page, data, &backup)
                    .advanced(advanced)
                    .confirm(confirm.as_deref() == Some(CONFIRMATION)),
            )?;
            if result.changed {
                println!(
                    "page {} written and verified: {}",
                    result.page,
                    hex_bytes(&result.written)
                );
                if let Some(backup) = result.backup {
                    println!("backup: {}", backup.path.display());
                }
            } else {
                println!("page {} already contains the requested value", result.page);
            }
            Ok(())
        }
        Type2Command::Dump { output } => {
            prepare(service, cli.device.as_deref())?;
            let read = service.type2_read(CancelToken::new())?;
            let bytes = type2::export(&read.image)?;
            write_output(&output, &bytes)?;
            println!(
                "{} {}: {}/{} pages written to {}",
                read.image.layout.product.name(),
                read.image.uid_hex(),
                read.image.known_pages(),
                read.image.layout.page_count(),
                output.display()
            );
            for warning in &read.warnings {
                println!("warning: {warning}");
            }
            Ok(())
        }
        Type2Command::Restore {
            input,
            confirm,
            include_configuration,
            include_capability_container,
            include_locks,
        } => {
            prepare(service, cli.device.as_deref())?;
            require_confirmation(confirm.as_deref(), "restoring a Type 2 image")?;
            let source = type2::load(&read_input(&input)?)?;
            let backup = backup_store(cli);
            let result = service.type2_restore(
                Type2Restore::new(source, &backup)
                    .options(RestoreOptions {
                        restore_user_memory: true,
                        restore_capability_container: include_capability_container,
                        restore_configuration: include_configuration,
                        restore_lock_bytes: include_locks,
                    })
                    .confirm(true),
            )?;
            println!(
                "restore complete: {} written, {} unchanged",
                result.summary.written_pages, result.summary.unchanged_pages
            );
            println!("backup: {}", result.backup.path.display());
            for warning in &result.warnings {
                println!("warning: {warning}");
            }
            Ok(())
        }
        Type2Command::Lock { pages, confirm } => {
            prepare(service, cli.device.as_deref())?;
            require_confirmation(confirm.as_deref(), "locking Type 2 pages")?;
            let pages = parse_page_list(&pages)?;
            let backup = backup_store(cli);
            let result = service.type2_lock(Type2Lock::new(pages, &backup).confirm(true))?;
            if result.update.is_empty() {
                println!("the requested pages are already locked");
            } else {
                println!("lock bits written and verified");
            }
            if let Some(backup) = result.backup {
                println!("backup: {}", backup.path.display());
            }
            Ok(())
        }
        Type2Command::Password {
            auth0,
            access,
            pwd,
            pack,
            confirm,
        } => {
            prepare(service, cli.device.as_deref())?;
            require_confirmation(confirm.as_deref(), "changing Type 2 password configuration")?;
            let access: [u8; 1] = parse_hex_array(&access, "ACCESS")?;
            let password: [u8; PAGE_SIZE] = parse_hex_array(&pwd, "PWD")?;
            let pack: [u8; 2] = parse_hex_array(&pack, "PACK")?;
            let backup = backup_store(cli);
            let result = service.type2_password(
                Type2Password::new(auth0, access[0], password, pack, &backup).confirm(true),
            )?;
            println!("password configuration written and verified");
            println!("backup: {}", result.backup.path.display());
            Ok(())
        }
    }
}

fn ndef(cli: &Cli, service: &mut Service, command: NdefCommand) -> Result<()> {
    prepare(service, cli.device.as_deref())?;
    match command {
        NdefCommand::Read { output } => {
            let read = service.type2_ndef_read(CancelToken::new())?;
            match &read.ndef.state {
                NdefState::Unformatted => println!("NDEF: unformatted"),
                NdefState::Empty => println!("NDEF: empty"),
                NdefState::Message(message) => {
                    println!(
                        "NDEF: {} record(s), {} bytes, capacity {}",
                        message.records().len(),
                        read.ndef.message_bytes,
                        read.ndef.capacity
                    );
                    for (index, record) in message.records().iter().enumerate() {
                        print_ndef_record(index, record);
                    }
                    if let Some(path) = output {
                        write_output(&path, &message.encode()?)?;
                        println!("raw message written to {}", path.display());
                    }
                }
            }
            for warning in &read.warnings {
                println!("warning: {warning}");
            }
            Ok(())
        }
        NdefCommand::Write {
            file,
            text,
            uri,
            language,
        } => {
            let message = build_ndef_message(file.as_deref(), text, uri, &language)?;
            let backup = backup_store(cli);
            let result = service.type2_ndef_write(Type2NdefWrite::new(message, &backup))?;
            println!(
                "NDEF written and verified: {} page(s) written, {} unchanged",
                result.summary.written_pages, result.summary.unchanged_pages
            );
            println!("backup: {}", result.backup.path.display());
            Ok(())
        }
        NdefCommand::Format { confirm } => {
            require_confirmation(confirm.as_deref(), "formatting the NDEF data area")?;
            let backup = backup_store(cli);
            let result = service.type2_ndef_format(Type2Format::new(&backup).confirm(true))?;
            println!(
                "NDEF formatted: {} page(s) written, {} unchanged",
                result.summary.written_pages, result.summary.unchanged_pages
            );
            println!("backup: {}", result.backup.path.display());
            Ok(())
        }
    }
}

fn print_type2_info(image: &Type2Image, warnings: &[String]) {
    println!("product: {}", image.layout.product.name());
    println!("uid: {}", image.uid_hex());
    println!(
        "pages: {}/{} readable",
        image.known_pages(),
        image.layout.page_count()
    );
    println!(
        "user memory: pages {}..={} ({} bytes)",
        image.layout.user_first,
        image.layout.user_last,
        image.layout.user_bytes()
    );
    println!("capability container: {}", hex_bytes(&image.cc.to_bytes()));
    println!("NDEF data area: {} bytes", image.cc.size);
    println!("NDEF capacity: {} bytes", image.cc.ndef_capacity());
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
            println!("NDEF: {state}");
            println!("NDEF read-only: {}", ndef.read_only);
        }
        Err(error) => println!("NDEF: invalid ({error})"),
    }
    match LockBits::from_image(image) {
        Ok(locks) => {
            println!(
                "static lock: {}  dynamic lock: {}",
                hex_bytes(&locks.static_lock),
                hex_bytes(&locks.dynamic_lock)
            );
            let locked: Vec<String> = (0..image.layout.total_pages)
                .filter(|page| locks.is_locked(&image.layout, *page))
                .map(|page| page.to_string())
                .collect();
            if !locked.is_empty() {
                println!("locked pages: {}", locked.join(", "));
            }
        }
        Err(error) => println!("locks: unavailable ({error})"),
    }
    match Configuration::from_image(image) {
        Ok(config) => {
            println!(
                "AUTH0: {:02X}  ACCESS: {:02X}  CFGLCK: {}  AUTHLIM: {}",
                config.auth0,
                config.access,
                config.cfg_locked(),
                config.auth_lim()
            );
        }
        Err(error) => println!("configuration: {error}"),
    }
    for warning in warnings {
        println!("warning: {warning}");
    }
}

fn print_ndef_record(index: usize, record: &NdefRecord) {
    if let Ok(text) = TextRecord::from_record(record) {
        println!(
            "record {index}: text {:?} (language {})",
            text.text, text.language
        );
        return;
    }
    if let Ok(uri) = UriRecord::from_record(record) {
        println!("record {index}: uri {}", uri.uri);
        return;
    }
    println!(
        "record {index}: tnf {:?} type {} payload {}",
        record.tnf(),
        hex_bytes(record.record_type()),
        hex_bytes(record.payload())
    );
}

fn build_ndef_message(
    file: Option<&Path>,
    text: Option<String>,
    uri: Option<String>,
    language: &str,
) -> Result<NdefMessage> {
    let supplied =
        usize::from(file.is_some()) + usize::from(text.is_some()) + usize::from(uri.is_some());
    if supplied != 1 {
        return Err(NfcError::InvalidArgument(
            "pass exactly one of --file, --text, or --uri".to_owned(),
        ));
    }
    if let Some(path) = file {
        return NdefMessage::decode(&read_input(path)?);
    }
    if let Some(text) = text {
        let record = TextRecord::new(text, language)?.to_record()?;
        return NdefMessage::single(record);
    }
    let uri = uri.expect("one NDEF source is present");
    NdefMessage::single(UriRecord::new(uri)?.to_record()?)
}

fn require_confirmation(token: Option<&str>, action: &str) -> Result<()> {
    if token == Some(CONFIRMATION) {
        Ok(())
    } else {
        Err(NfcError::InvalidArgument(format!(
            "{action} needs --confirm {CONFIRMATION}"
        )))
    }
}

fn parse_page_list(value: &str) -> Result<Vec<u8>> {
    let mut pages = Vec::new();
    for part in value.split([',', ' ']).filter(|part| !part.is_empty()) {
        if let Some((first, last)) = part.split_once('-') {
            let first: u8 = first
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {first:?}")))?;
            let last: u8 = last
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {last:?}")))?;
            if first > last {
                return Err(NfcError::InvalidArgument(format!(
                    "page range {part:?} is reversed"
                )));
            }
            pages.extend(first..=last);
        } else {
            let page: u8 = part
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {part:?}")))?;
            pages.push(page);
        }
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

fn parse_hex_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != N * 2 {
        return Err(NfcError::InvalidArgument(format!(
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

fn management_store(cli: &Cli, create: bool) -> Result<(KeyStore, PathBuf)> {
    let path = cli
        .key_store
        .clone()
        .or_else(KeyStore::default_path)
        .ok_or_else(|| {
            NfcError::InvalidArgument(
                "no key store path; pass --key-store or set XDG_CONFIG_HOME/HOME".to_owned(),
            )
        })?;
    if path.exists() {
        Ok((KeyStore::load(&path)?, path))
    } else if !create && cli.key_store.is_some() {
        Err(NfcError::Io(format!(
            "key store {} does not exist",
            path.display()
        )))
    } else {
        Ok((KeyStore::new(), path))
    }
}

fn load_key_store(cli: &Cli) -> Result<KeyStore> {
    let mut store = if let Some(path) = &cli.key_store {
        KeyStore::load(path)?
    } else if let Some(path) = KeyStore::default_path() {
        KeyStore::load_or_default(&path)?
    } else {
        KeyStore::new()
    };
    for hex in &cli.key {
        let key = Key::from_hex(hex)?;
        store.add(KeyEntry::new(None, KeyKind::Any, key, KeySource::Manual))?;
    }
    for path in &cli.key_file {
        store.import_text(path)?;
    }
    Ok(store)
}

fn backup_store(cli: &Cli) -> BackupStore {
    match &cli.backup_dir {
        Some(directory) => BackupStore::new(directory),
        None => BackupStore::default(),
    }
}

fn parse_key_types(value: &str) -> Result<KeyTypes> {
    let normalized = value.to_ascii_uppercase().replace([' ', ',', '+'], "");
    match normalized.as_str() {
        "A" => Ok(KeyTypes::a_only()),
        "B" => Ok(KeyTypes::b_only()),
        "AB" | "BA" | "" => Ok(KeyTypes::ALL),
        other => Err(NfcError::InvalidArgument(format!(
            "unknown key types {other:?}; use A, B, or AB"
        ))),
    }
}

fn parse_block_data(value: &str) -> Result<[u8; BLOCK_SIZE]> {
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
        return Err(NfcError::InvalidArgument(format!(
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

fn hex_digit(character: u8) -> Result<u8> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(NfcError::InvalidArgument(
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

fn mock_service() -> Service {
    let transport = MockTransport::new();
    let (classic_card, classic_blocks) = mock_classic_card();
    let handle = transport.add(
        DeviceInfo::new("Mock Classic 1K", "mock:classic-1k"),
        Some(classic_card),
    );
    for (block, data) in classic_blocks {
        handle.set_block(block, data);
    }
    let (ndef_card, ndef_blocks) = mock_classic_ndef_card();
    let ndef_handle = transport.add(
        DeviceInfo::new("Mock Classic NDEF 1K", "mock:classic-ndef"),
        Some(ndef_card),
    );
    for (block, data) in ndef_blocks {
        ndef_handle.set_block(block, data);
    }
    let ntag = transport.add(
        DeviceInfo::new("Mock NTAG215", "mock:ntag215"),
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
    Service::with_transport(Arc::new(transport))
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

fn diag() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!("NFC Foundry {version}");
    println!("nfctl {version}");
    println!(
        "drivers: {}<port>, {}<bus>:<address>",
        nfc_drivers::PN532_UART_PREFIX,
        nfc_drivers::ACR122_USB_PREFIX
    );
    match nfc_recover::hardnested::tables::resolve_dir(None) {
        Ok(path) => println!("hardnested tables: {}", path.display()),
        Err(error) => println!("hardnested tables: not found ({error})"),
    }
    Ok(())
}

fn list(service: &mut Service) -> Result<()> {
    service.refresh_devices()?;
    if service.devices().is_empty() {
        println!("no readers found");
    } else {
        for device in service.devices() {
            println!("{}\t{}", device.name, device.conn_string);
        }
    }
    Ok(())
}

fn prepare(service: &mut Service, device: Option<&str>) -> Result<()> {
    service.refresh_devices()?;
    let conn_string = device.ok_or_else(|| {
        NfcError::InvalidArgument("pass --device <connstring> to select a reader".to_owned())
    })?;
    service.select_device(conn_string)
}

fn watch_card(service: &mut Service) -> Result<()> {
    let mut last: Option<CardInfo> = None;
    loop {
        match service.scan() {
            Ok(card) => {
                if last
                    .as_ref()
                    .is_none_or(|previous| !previous.same_identity(&card))
                {
                    print_card(&card);
                    last = Some(card);
                }
            }
            Err(NfcError::NoCard) => {
                if last.take().is_some() {
                    println!("card removed");
                }
            }
            Err(error) => return Err(error),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn print_card(card: &CardInfo) {
    println!("type: {}", card_type_label(card.card_type));
    println!("uid: {}", card.uid_hex());
    println!("atqa: {:02X} {:02X}", card.atqa[0], card.atqa[1]);
    println!("sak: {:02X}", card.sak);
}

fn card_type_label(card_type: CardType) -> &'static str {
    match card_type {
        CardType::Unknown => "unknown",
        CardType::MifareClassic1K => "MIFARE Classic 1K",
        CardType::MifareClassic4K => "MIFARE Classic 4K",
        CardType::Type2 => "NFC Forum Type 2",
        CardType::Iso14443 => "ISO/IEC 14443A",
    }
}
