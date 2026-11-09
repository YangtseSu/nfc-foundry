use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use nfc_app::Service;
use nfc_app::classic::{
    BackupStore, ClassicBlockWrite, ClassicKeyscan, ClassicRead, ClassicRestore, KeyEntry, KeyKind,
    KeySource, KeyStore, KeyTypes, common_keys,
};
use nfc_classic::{DumpFormat, Layout, bcc_for_uid, diff, exchange};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, CardType, Key, KeyType, NfcError, Result};
use nfc_drivers::DriverRegistry;
use nfc_recover::RecoveryRequest;
use nfc_recover::auto::RecoveryStrategy;
use nfc_transport::DeviceInfo;
use nfc_transport::mock::MockTransport;

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
    Info,
    Read,
    Write,
    Dump,
    Restore,
    Copy,
    Erase,
    Lock,
    Password,
}

#[derive(Clone, Debug, Subcommand)]
enum NdefCommand {
    Read,
    Write,
    Format,
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
    #[command(alias = "hardnested")]
    HardNested,
    Fm11rf08s,
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
        Command::Type2 { command } => match command {
            Type2Command::Info => Err(NfcError::NotImplemented("nfctl type2 info")),
            Type2Command::Read => Err(NfcError::NotImplemented("nfctl type2 read")),
            Type2Command::Write => Err(NfcError::NotImplemented("nfctl type2 write")),
            Type2Command::Dump => Err(NfcError::NotImplemented("nfctl type2 dump")),
            Type2Command::Restore => Err(NfcError::NotImplemented("nfctl type2 restore")),
            Type2Command::Copy => Err(NfcError::NotImplemented("nfctl type2 copy")),
            Type2Command::Erase => Err(NfcError::NotImplemented("nfctl type2 erase")),
            Type2Command::Lock => Err(NfcError::NotImplemented("nfctl type2 lock")),
            Type2Command::Password => Err(NfcError::NotImplemented("nfctl type2 password")),
        },
        Command::Ndef { command } => match command {
            NdefCommand::Read => Err(NfcError::NotImplemented("nfctl ndef read")),
            NdefCommand::Write => Err(NfcError::NotImplemented("nfctl ndef write")),
            NdefCommand::Format => Err(NfcError::NotImplemented("nfctl ndef format")),
        },
        Command::Recover { command } => recovery(&cli, &mut service, command),
        Command::Diag => {
            println!("nfctl {} (skeleton)", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
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
        RecoveryCommand::HardNested => {
            return Err(NfcError::NotImplemented("nfctl recover hardnested"));
        }
        RecoveryCommand::Fm11rf08s => {
            return Err(NfcError::NotImplemented("nfctl recover fm11rf08s"));
        }
    };
    prepare(service, cli.device.as_deref())?;
    let candidates = recovery_candidates(cli)?;
    if candidates.is_empty() {
        eprintln!("warning: no candidate keys; nested recovery needs a known key");
    }
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
    transport.add(
        DeviceInfo::new("Mock NTAG215", "mock:ntag215"),
        Some(CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
            [0x00, 0x44],
            0x00,
            CardType::Type2,
        )),
    );
    Service::with_transport(Arc::new(transport))
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
