use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use nfc_app::Service;
use nfc_core::{CardInfo, CardType, NfcError, Result};
use nfc_drivers::DriverRegistry;
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
    let mut service = build_service(cli.mock);
    match cli.command {
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

fn build_service(mock: bool) -> Service {
    if mock {
        mock_service()
    } else {
        Service::with_transport(Arc::new(DriverRegistry))
    }
}

fn mock_service() -> Service {
    let transport = MockTransport::new();
    transport.add(
        DeviceInfo::new("Mock Classic 1K", "mock:classic-1k"),
        Some(CardInfo::new(
            vec![0x04, 0xA2, 0x24, 0x8A, 0x1B, 0x2C, 0x80],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )),
    );
    transport.add(
        DeviceInfo::new("Mock NTAG215", "mock:ntag215"),
        Some(CardInfo::new(
            vec![0x04, 0x5B, 0x1C, 0x2A, 0x3B, 0x4C, 0x80],
            [0x44, 0x00],
            0x00,
            CardType::Type2,
        )),
    );
    Service::with_transport(Arc::new(transport))
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
