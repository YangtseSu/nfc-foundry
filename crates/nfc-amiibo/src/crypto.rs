//! Amiibo AES-128-CTR encryption and HMAC-SHA256 signatures.
//!
//! The implementation follows the published Amiibo key-derivation format
//! documented by the MIT-licensed `socram8888/amiitool`: HMAC-SHA256 DRBG
//! over a prepared 78-byte seed, then AES-CTR over the data region. This is
//! independent Rust code; no amiitool source or Nintendo key material is
//! copied. Only user-supplied master keys reach this module.

use aes::Aes128;
use cipher::{BlockCipherEncrypt, KeyInit};
use hmac::{Hmac, Mac};
use nfc_core::{NfcError, Result};
use sha2::Sha256;

use crate::keys::KeyMaterial;

const SEED_SIZE: usize = 64;
const PREPARED_SEED_SIZE: usize = 78;
const DATA_OFFSET: usize = 0x02C;
const DATA_LEN: usize = 0x188;
const TAG_HMAC_OFFSET: usize = 0x1B4;
const DATA_HMAC_OFFSET: usize = 0x008;

#[derive(Clone, Copy)]
struct DerivedKeys {
    aes_key: [u8; 16],
    aes_iv: [u8; 16],
    hmac_key: [u8; 16],
}

/// Build the 64-byte keygen seed from the internal plaintext.
fn seed(plain: &[u8; 540]) -> [u8; SEED_SIZE] {
    let mut seed = [0u8; SEED_SIZE];
    seed[..2].copy_from_slice(&plain[0x029..0x02B]);
    seed[0x10..0x18].copy_from_slice(&plain[0x1D4..0x1DC]);
    seed[0x18..0x20].copy_from_slice(&plain[0x1D4..0x1DC]);
    seed[0x20..0x40].copy_from_slice(&plain[0x1E8..0x208]);
    seed
}

fn derive(master: KeyMaterial<'_>, seed: &[u8; SEED_SIZE]) -> Result<DerivedKeys> {
    let mut prepared = [0u8; PREPARED_SEED_SIZE];
    let mut length = 0;
    let mut push = |bytes: &[u8]| {
        prepared[length..length + bytes.len()].copy_from_slice(bytes);
        length += bytes.len();
    };
    push(master.type_string);
    let leading = 16 - master.magic_size;
    push(&seed[..leading]);
    push(&master.magic[..master.magic_size]);
    push(&seed[16..32]);
    for (index, value) in seed[32..64].iter().enumerate() {
        prepared[length + index] = value ^ master.xor_pad[index];
    }
    length += 32;
    if length != PREPARED_SEED_SIZE {
        return Err(NfcError::InvalidArgument(
            "the Amiibo key record produced an invalid seed length".to_owned(),
        ));
    }

    let mut output = [0u8; 48];
    for (iteration, chunk) in output.chunks_mut(32).enumerate() {
        let mut mac =
            <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(&master.hmac_key).map_err(|_| {
                NfcError::InvalidArgument("the Amiibo master HMAC key is invalid".to_owned())
            })?;
        let counter = u16::try_from(iteration).map_err(|_| {
            NfcError::InvalidArgument("the Amiibo DRBG counter overflowed".to_owned())
        })?;
        mac.update(&counter.to_be_bytes());
        mac.update(&prepared[..length]);
        let digest = mac.finalize().into_bytes();
        let take = chunk.len().min(digest.len());
        chunk[..take].copy_from_slice(&digest[..take]);
    }

    Ok(DerivedKeys {
        aes_key: output[..16].try_into().map_err(|_| {
            NfcError::InvalidArgument("the derived AES key length is invalid".to_owned())
        })?,
        aes_iv: output[16..32].try_into().map_err(|_| {
            NfcError::InvalidArgument("the derived AES IV length is invalid".to_owned())
        })?,
        hmac_key: output[32..48].try_into().map_err(|_| {
            NfcError::InvalidArgument("the derived HMAC key length is invalid".to_owned())
        })?,
    })
}

fn increment_counter(counter: &mut [u8; 16]) {
    for byte in counter.iter_mut().rev() {
        let (value, carry) = byte.overflowing_add(1);
        *byte = value;
        if !carry {
            break;
        }
    }
}

fn apply_ctr(keys: DerivedKeys, data: &mut [u8]) -> Result<()> {
    let cipher = Aes128::new_from_slice(&keys.aes_key)
        .map_err(|_| NfcError::InvalidArgument("the derived AES key is invalid".to_owned()))?;
    let mut counter = keys.aes_iv;
    for chunk in data.chunks_mut(16) {
        let mut block = counter;
        cipher.encrypt_block((&mut block).into());
        for (byte, key) in chunk.iter_mut().zip(block.iter()) {
            *byte ^= key;
        }
        increment_counter(&mut counter);
    }
    Ok(())
}

fn hmac(key: &[u8; 16], parts: &[&[u8]]) -> Result<[u8; 32]> {
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key)
        .map_err(|_| NfcError::InvalidArgument("the Amiibo HMAC key is invalid".to_owned()))?;
    for part in parts {
        mac.update(part);
    }
    let digest = mac.finalize().into_bytes();
    digest
        .as_slice()
        .try_into()
        .map_err(|_| NfcError::InvalidArgument("the Amiibo HMAC output is invalid".to_owned()))
}

fn tag_hmac(keys: DerivedKeys, plain: &[u8; 540]) -> Result<[u8; 32]> {
    hmac(&keys.hmac_key, &[&plain[0x1D4..0x208]])
}

fn data_hmac(keys: DerivedKeys, plain: &[u8; 540], tag_signature: &[u8; 32]) -> Result<[u8; 32]> {
    hmac(
        &keys.hmac_key,
        &[&plain[0x029..0x1B4], tag_signature, &plain[0x1D4..0x208]],
    )
}

/// Convert raw tag bytes into the internal layout.
fn tag_to_internal(tag: &[u8; 540]) -> [u8; 540] {
    let mut internal = [0u8; 540];
    internal[0x000..0x008].copy_from_slice(&tag[0x008..0x010]);
    internal[0x008..0x028].copy_from_slice(&tag[0x080..0x0A0]);
    internal[0x028..0x04C].copy_from_slice(&tag[0x010..0x034]);
    internal[0x04C..0x1B4].copy_from_slice(&tag[0x0A0..0x208]);
    internal[0x1B4..0x1D4].copy_from_slice(&tag[0x034..0x054]);
    internal[0x1D4..0x1DC].copy_from_slice(&tag[0x000..0x008]);
    internal[0x1DC..0x208].copy_from_slice(&tag[0x054..0x080]);
    internal[0x208..0x21C].copy_from_slice(&tag[0x208..0x21C]);
    internal
}

/// Convert internal bytes back to the raw tag layout.
fn internal_to_tag(internal: &[u8; 540]) -> [u8; 540] {
    let mut tag = [0u8; 540];
    tag[0x000..0x008].copy_from_slice(&internal[0x1D4..0x1DC]);
    tag[0x008..0x010].copy_from_slice(&internal[0x000..0x008]);
    tag[0x010..0x034].copy_from_slice(&internal[0x028..0x04C]);
    tag[0x034..0x054].copy_from_slice(&internal[0x1B4..0x1D4]);
    tag[0x054..0x080].copy_from_slice(&internal[0x1DC..0x208]);
    tag[0x080..0x0A0].copy_from_slice(&internal[0x008..0x028]);
    tag[0x0A0..0x208].copy_from_slice(&internal[0x04C..0x1B4]);
    tag[0x208..0x21C].copy_from_slice(&internal[0x208..0x21C]);
    tag
}

/// Decrypt and verify one raw tag.
///
/// The returned internal image keeps the final 20 raw bytes in place so that
/// [`encrypt`] reproduces the input exactly.
pub fn decrypt(keys: &crate::AmiiboKeys, tag: &[u8; 540]) -> Result<[u8; 540]> {
    let internal = tag_to_internal(tag);
    let seed = seed(&internal);
    let data_keys = derive(keys.data(), &seed)?;
    let tag_keys = derive(keys.tag(), &seed)?;

    let mut plain = internal;
    apply_ctr(data_keys, &mut plain[DATA_OFFSET..DATA_OFFSET + DATA_LEN])?;

    let tag_signature = tag_hmac(tag_keys, &plain)?;
    plain[TAG_HMAC_OFFSET..TAG_HMAC_OFFSET + 32].copy_from_slice(&tag_signature);
    let data_signature = data_hmac(data_keys, &plain, &tag_signature)?;
    plain[DATA_HMAC_OFFSET..DATA_HMAC_OFFSET + 32].copy_from_slice(&data_signature);

    let tag_ok = plain[TAG_HMAC_OFFSET..TAG_HMAC_OFFSET + 32]
        == internal[TAG_HMAC_OFFSET..TAG_HMAC_OFFSET + 32];
    let data_ok = plain[DATA_HMAC_OFFSET..DATA_HMAC_OFFSET + 32]
        == internal[DATA_HMAC_OFFSET..DATA_HMAC_OFFSET + 32];
    if !tag_ok || !data_ok {
        return Err(NfcError::VerificationFailed(
            "the Amiibo signatures did not verify with these key files".to_owned(),
        ));
    }
    Ok(plain)
}

/// Encrypt and sign one internal plaintext image.
pub fn encrypt(keys: &crate::AmiiboKeys, plain: &[u8; 540]) -> Result<[u8; 540]> {
    let seed = seed(plain);
    let data_keys = derive(keys.data(), &seed)?;
    let tag_keys = derive(keys.tag(), &seed)?;

    let tag_signature = tag_hmac(tag_keys, plain)?;
    let data_signature = data_hmac(data_keys, plain, &tag_signature)?;

    let mut cipher = *plain;
    cipher[TAG_HMAC_OFFSET..TAG_HMAC_OFFSET + 32].copy_from_slice(&tag_signature);
    cipher[DATA_HMAC_OFFSET..DATA_HMAC_OFFSET + 32].copy_from_slice(&data_signature);
    apply_ctr(data_keys, &mut cipher[DATA_OFFSET..DATA_OFFSET + DATA_LEN])?;

    let mut tag = internal_to_tag(&cipher);
    // The final 20 bytes are outside the crypto region but are still part of
    // the raw tag image.
    tag[0x208..0x21C].copy_from_slice(&plain[0x208..0x21C]);
    Ok(tag)
}
