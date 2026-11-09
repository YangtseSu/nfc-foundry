use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nfc_app::Service;
use nfc_core::{NfcError, Result};

#[derive(Debug, Parser)]
#[command(
    name = "nfctl",
    version,
    about = "NFC Foundry command-line workbench",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
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

#[derive(Debug, Subcommand)]
enum CardCommand {
    Info,
}

#[derive(Debug, Subcommand)]
enum ClassicCommand {
    Read,
    Write,
    Dump,
    Restore,
    Keyscan,
    Diff,
    Clone,
    Format,
    Trailer,
    Value,
}

#[derive(Debug, Subcommand)]
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

#[derive(Debug, Subcommand)]
enum NdefCommand {
    Read,
    Write,
    Format,
}

#[derive(Debug, Subcommand)]
enum RecoveryCommand {
    Auto,
    Darkside,
    Nested,
    StaticNested,
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
    let service = Service::new();
    match cli.command {
        Command::List => {
            if service.devices().is_empty() {
                println!("no readers found");
            } else {
                for device in service.devices() {
                    println!("{}\t{}", device.name, device.conn_string);
                }
            }
            Ok(())
        }
        Command::Scan { watch: _ } => Err(NfcError::NotImplemented("nfctl scan")),
        Command::Card {
            command: CardCommand::Info,
        } => Err(NfcError::NotImplemented("nfctl card info")),
        Command::Classic { command } => match command {
            ClassicCommand::Read => Err(NfcError::NotImplemented("nfctl classic read")),
            ClassicCommand::Write => Err(NfcError::NotImplemented("nfctl classic write")),
            ClassicCommand::Dump => Err(NfcError::NotImplemented("nfctl classic dump")),
            ClassicCommand::Restore => Err(NfcError::NotImplemented("nfctl classic restore")),
            ClassicCommand::Keyscan => Err(NfcError::NotImplemented("nfctl classic keyscan")),
            ClassicCommand::Diff => Err(NfcError::NotImplemented("nfctl classic diff")),
            ClassicCommand::Clone => Err(NfcError::NotImplemented("nfctl classic clone")),
            ClassicCommand::Format => Err(NfcError::NotImplemented("nfctl classic format")),
            ClassicCommand::Trailer => Err(NfcError::NotImplemented("nfctl classic trailer")),
            ClassicCommand::Value => Err(NfcError::NotImplemented("nfctl classic value")),
        },
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
        Command::Recover { command } => match command {
            RecoveryCommand::Auto => Err(NfcError::NotImplemented("nfctl recover auto")),
            RecoveryCommand::Darkside => Err(NfcError::NotImplemented("nfctl recover darkside")),
            RecoveryCommand::Nested => Err(NfcError::NotImplemented("nfctl recover nested")),
            RecoveryCommand::StaticNested => {
                Err(NfcError::NotImplemented("nfctl recover staticnested"))
            }
            RecoveryCommand::HardNested => {
                Err(NfcError::NotImplemented("nfctl recover hardnested"))
            }
            RecoveryCommand::Fm11rf08s => Err(NfcError::NotImplemented("nfctl recover fm11rf08s")),
        },
        Command::Diag => {
            println!("nfctl {} (skeleton)", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
