//! Sidecar metadata for image-only dump exports.
//!
//! Raw, MCT, and Proxmark3 files carry no provenance. The sidecar records the
//! card identity, device, timestamps, and authenticated sector keys next to the
//! image. The image bytes stay authoritative: a sidecar is applied only when
//! its layout, completeness, and card identity agree with the loaded image.

use std::fs;
use std::path::{Path, PathBuf};

use nfc_classic::{Dump, DumpFormat, exchange};
use nfc_core::Result;

use crate::fsutil::write_atomic;

/// Sidecar suffix appended to the image path, e.g. `card-dump.bin.nfcx.json`.
pub const SIDECAR_SUFFIX: &str = ".nfcx.json";

pub fn sidecar_path(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_os_string();
    name.push(SIDECAR_SUFFIX);
    PathBuf::from(name)
}

/// Writes the sidecar for an image-only export. Project exports keep metadata
/// inline and never get a sidecar. The caller treats a sidecar error as a
/// warning so a successful image write is not invalidated.
pub fn write_sidecar(image: &Path, format: DumpFormat, dump: &Dump) -> Result<Option<PathBuf>> {
    if !is_image_only(format.resolve(dump)) {
        return Ok(None);
    }
    let bytes = exchange::encode_sidecar(dump)?;
    let path = sidecar_path(image);
    write_atomic(&path, &bytes)?;
    Ok(Some(path))
}

/// What happened while applying a sidecar to a loaded image.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidecarAdoption {
    /// Set when a sidecar file exists, even if it was rejected.
    pub path: Option<PathBuf>,
    pub applied: bool,
    /// Number of sector keys restored from the sidecar.
    pub keys: usize,
    pub warnings: Vec<String>,
}

/// Applies `<image>.nfcx.json` metadata to a raw, MCT, or Proxmark3 load.
///
/// A missing sidecar is normal. A malformed, mismatched, or unknown-version
/// sidecar yields a warning and leaves the image untouched. Project images are
/// authoritative and never consult a sidecar.
pub fn adopt_sidecar(image: &Path, format: DumpFormat, dump: &mut Dump) -> SidecarAdoption {
    let mut outcome = SidecarAdoption::default();
    if !is_image_only(format) {
        return outcome;
    }
    let path = sidecar_path(image);
    if !path.exists() {
        return outcome;
    }
    outcome.path = Some(path.clone());
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            outcome.warnings.push(format!(
                "sidecar {} could not be read: {error}; metadata ignored",
                path.display()
            ));
            return outcome;
        }
    };
    let sidecar = match exchange::parse_sidecar(&bytes) {
        Ok(sidecar) => sidecar,
        Err(error) => {
            outcome.warnings.push(format!(
                "sidecar {} is not usable: {error}; image loaded without metadata",
                path.display()
            ));
            return outcome;
        }
    };
    outcome.warnings.extend(
        sidecar
            .warnings
            .iter()
            .map(|warning| format!("sidecar {}: {warning}", path.display())),
    );
    if !sidecar.usable {
        return outcome;
    }
    if sidecar.layout != dump.layout {
        outcome.warnings.push(format!(
            "sidecar {} describes {}, but the image is {}; metadata ignored",
            path.display(),
            sidecar.layout.name(),
            dump.layout.name()
        ));
        return outcome;
    }
    if let Some(complete) = sidecar.complete
        && complete != dump.complete()
    {
        outcome.warnings.push(format!(
            "sidecar {} records complete={complete}, but the image is {}; metadata ignored",
            path.display(),
            if dump.complete() {
                "complete"
            } else {
                "partial"
            }
        ));
        return outcome;
    }
    if let Some(sidecar_card) = &sidecar.card {
        match &dump.card {
            Some(image_card) if !image_card.uid.is_empty() => {
                if image_card.identity_key() != sidecar_card.identity_key() {
                    outcome.warnings.push(format!(
                        "sidecar {} records card {}, but the image is {}; metadata ignored",
                        path.display(),
                        sidecar_card.identity_key(),
                        image_card.identity_key()
                    ));
                    return outcome;
                }
            }
            _ => dump.card = Some(sidecar_card.clone()),
        }
    }
    outcome.keys = sidecar.metadata.keys.len();
    dump.metadata = sidecar.metadata;
    outcome.applied = true;
    outcome
}

fn is_image_only(format: DumpFormat) -> bool {
    matches!(
        format,
        DumpFormat::Raw | DumpFormat::Mct | DumpFormat::Proxmark3
    )
}

#[cfg(test)]
mod tests {
    use nfc_classic::{DumpKey, DumpMetadata, Layout};
    use nfc_core::{CardInfo, CardType, Key, KeyType};

    use super::*;

    fn card(uid: [u8; 4]) -> CardInfo {
        CardInfo::new(uid.to_vec(), [0x00, 0x04], 0x08, CardType::MifareClassic1K)
    }

    fn complete_dump() -> Dump {
        Dump::from_raw(&[0u8; Layout::Classic1K.dump_size()]).expect("image")
    }

    fn keyed_dump() -> Dump {
        let mut dump = complete_dump().with_card(card([0x01, 0x02, 0x03, 0x04]));
        dump.metadata = DumpMetadata {
            device: Some("mock:classic".to_owned()),
            started_at: Some(1_762_819_200),
            finished_at: Some(1_762_819_205),
            keys: vec![DumpKey {
                sector: 0,
                key_type: KeyType::A,
                key: Key::from_hex("FFFFFFFFFFFF").expect("key"),
                source: Some("read_context".to_owned()),
            }],
        };
        dump
    }

    #[test]
    fn sidecar_path_appends_to_the_full_image_name() {
        assert_eq!(
            sidecar_path(Path::new("/tmp/card-dump.bin")),
            PathBuf::from("/tmp/card-dump.bin.nfcx.json")
        );
    }

    #[test]
    fn write_sidecar_is_atomic_private_and_skips_project_images() {
        let directory = tempfile::tempdir().expect("dir");
        let image = directory.path().join("card-dump.bin");
        let dump = keyed_dump();
        let path = write_sidecar(&image, DumpFormat::Raw, &dump)
            .expect("write")
            .expect("sidecar");
        assert_eq!(path, sidecar_path(&image));
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).expect("mode").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let restored = exchange::parse_sidecar(&fs::read(&path).expect("bytes")).expect("parse");
        assert_eq!(restored.card, Some(card([0x01, 0x02, 0x03, 0x04])));
        assert_eq!(restored.metadata, dump.metadata);
        assert_eq!(fs::read_dir(directory.path()).expect("entries").count(), 1);

        assert_eq!(
            write_sidecar(&image, DumpFormat::Project, &dump).expect("project"),
            None
        );
    }

    #[test]
    fn missing_sidecar_is_normal() {
        let directory = tempfile::tempdir().expect("dir");
        let image = directory.path().join("card-dump.bin");
        let mut dump = complete_dump();
        let outcome = adopt_sidecar(&image, DumpFormat::Raw, &mut dump);
        assert_eq!(outcome, SidecarAdoption::default());
        assert_eq!(dump, complete_dump());
    }

    #[test]
    fn sidecar_restores_card_identity_and_keys_on_a_raw_image() {
        let directory = tempfile::tempdir().expect("dir");
        let image = directory.path().join("card-dump.bin");
        let source = keyed_dump();
        write_sidecar(&image, DumpFormat::Raw, &source).expect("write");
        let mut loaded = complete_dump();
        let outcome = adopt_sidecar(&image, DumpFormat::Raw, &mut loaded);
        assert!(outcome.applied, "{:?}", outcome.warnings);
        assert_eq!(outcome.keys, 1);
        assert_eq!(loaded.card, source.card);
        assert_eq!(loaded.metadata, source.metadata);
    }

    #[test]
    fn sidecar_is_ignored_for_project_images() {
        let directory = tempfile::tempdir().expect("dir");
        let image = directory.path().join("card-dump.nfc-foundry.json");
        let source = keyed_dump();
        write_sidecar(&image, DumpFormat::Raw, &source).expect("write");
        let mut loaded = source.clone();
        loaded.metadata = DumpMetadata::default();
        let outcome = adopt_sidecar(&image, DumpFormat::Project, &mut loaded);
        assert_eq!(outcome, SidecarAdoption::default());
        assert_eq!(loaded.metadata, DumpMetadata::default());
    }

    #[test]
    fn corrupt_sidecar_warns_and_leaves_the_image_alone() {
        let directory = tempfile::tempdir().expect("dir");
        let image = directory.path().join("card-dump.bin");
        fs::write(sidecar_path(&image), b"{not json").expect("write");
        let mut loaded = complete_dump();
        let outcome = adopt_sidecar(&image, DumpFormat::Raw, &mut loaded);
        assert!(!outcome.applied);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(loaded, complete_dump());
    }

    #[test]
    fn mismatched_sidecar_is_rejected() {
        let directory = tempfile::tempdir().expect("dir");
        let image = directory.path().join("card-dump.bin");

        let mut other_layout =
            Dump::from_raw(&[0u8; Layout::Classic4K.dump_size()]).expect("image");
        other_layout.card = Some(card([0x01, 0x02, 0x03, 0x04]));
        other_layout.metadata = keyed_dump().metadata;
        write_sidecar(&image, DumpFormat::Raw, &other_layout).expect("write");
        let mut loaded = complete_dump();
        let outcome = adopt_sidecar(&image, DumpFormat::Raw, &mut loaded);
        assert!(!outcome.applied);
        assert!(outcome.warnings[0].contains("describes classic_4k"));
        assert_eq!(loaded, complete_dump());

        let source = keyed_dump();
        write_sidecar(&image, DumpFormat::Raw, &source).expect("write");
        let mut loaded = complete_dump().with_card(card([0x04, 0x03, 0x02, 0x01]));
        let outcome = adopt_sidecar(&image, DumpFormat::Raw, &mut loaded);
        assert!(!outcome.applied);
        assert!(outcome.warnings[0].contains("records card"));
        assert!(loaded.metadata.keys.is_empty());
    }

    #[test]
    fn unknown_version_sidecar_only_warns() {
        let directory = tempfile::tempdir().expect("dir");
        let image = directory.path().join("card-dump.bin");
        fs::write(
            sidecar_path(&image),
            br#"{"version":9,"layout":"classic_1k","keys":[{"sector":0,"type":"A","key":"FFFFFFFFFFFF"}]}"#,
        )
        .expect("write");
        let mut loaded = complete_dump();
        let outcome = adopt_sidecar(&image, DumpFormat::Raw, &mut loaded);
        assert!(!outcome.applied);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("version 9"));
        assert!(loaded.metadata.keys.is_empty());
    }
}
