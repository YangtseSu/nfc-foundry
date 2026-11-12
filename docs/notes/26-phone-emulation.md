# Phone-emulated MIFARE Classic cards

Date: 2026-11-12

## Scope

Phone emulation is not uniformly UID-only.

- Android Host Card Emulation (HCE) alone emulates ISO/IEC 14443-4, not
  MIFARE Classic.
- OEM wallets can use the NFC controller and embedded secure element to
  emulate a full MIFARE Classic card. Xiaomi wallets, for example, present a
  MIFARE Classic 1K, answer MIFARE authentication, and expose writable data
  sectors. A reader can authenticate, read, and write those sectors when it
  has the card keys.

Do not infer "UID-only" from the card being a phone. The selection identity and
the MIFARE command responses are the evidence.

## Why a full emulation can still fail

The previous read path re-selected the card before every authentication
attempt. Reselection resets Crypto1 and interrupts the command stream. A
physical card tolerates this; a phone emulation may not. The resulting
symptom is a selected card whose authentication times out even though the
emulated card supports MIFARE.

The read path now selects the card once per operation and then sends
authentication and block commands continuously. Identity is checked explicitly
at operation boundaries (before a write sequence and after a completed read),
and the MIFARE authentication frame itself carries the selected UID suffix, so
a different card cannot authenticate with the cached UID.

If an authentication still times out, the read probes with an unauthenticated
READ. A MIFARE card answers with a NAK; a UID-only emulation does not answer.
A card that proves MIFARE support keeps trying the remaining candidate keys
instead of being reported as identity-only.

Selection robustness remains:

- a known UID is re-selected with `InListPassiveTarget` `InitiatorData`
  (cascade tag included) instead of cycling the RF field on every poll;
- a full field cycle and unrestricted selection happen only when the known
  card does not answer;
- PN532 and ACR122U passive activation use ten retries instead of zero, so a
  phone that needs a moment to arm its emulation is still found.

## Read performance

A read is not a key search. The earlier read path tried every one of the 57
common dictionary keys as both Key A and Key B for every unauthenticated
sector: 1,824 authentication commands for a 1K card, roughly a minute on the
PN532 UART. That is long enough for a phone emulation to time out.

The read path now uses, per sector:

1. the key that authenticated the previous sector;
2. the card's stored keys for that sector;
3. the three built-in default keys (factory, MAD Key A, NFC Forum Key A) as
   both key types.

A card with one key authenticates once per sector (16 authentications plus 64
block reads), which completes in a few seconds. The full dictionary remains
available through the explicit key scan. `include_common_keys` controls the
four-key fallback, not the full dictionary.

## Identity-only fallback

Some emulations really do expose only the UID. If the card reselects with the
same identity but never answers MIFARE authentication, a read returns an
identity-only result instead of a timeout:

- the GUI shows the UID, ATQA, and SAK and keeps write actions disabled;
- `nfctl classic dump` prints the warnings and writes a project-format image
  with every block marked unreadable;
- write, restore, NDEF, and backup paths still require readable sectors, so an
  identity-only image cannot be used as a write baseline.

A selected card whose SAK/ATQA does not identify Classic or Type 2 is also
reported as identity-only instead of a generic "unsupported" error.

## Keys

A full emulation still requires the correct sector keys. The built-in default
keys are tried first; if the emulated card kept the original card's keys, add
them to its per-card `.dic` file (or pass `--key` to `nfctl`) before reading. A
key scan also works because authentication now runs without a reselect before
every candidate.

## Writing

A full MIFARE Classic emulation can accept data-sector writes if its keys and
access bits allow them. The normal write gates still apply: authentication,
access-bit checks, backup, explicit confirmation for trailers/UID/format
actions, and read-back verification. Sector 0 and the UID may be managed by
the wallet even when data sectors are writable; the product does not assume
otherwise.

## Testing

Scripted application tests cover a card whose selection succeeds but whose
authentication always times out:

- `uid_only_emulation_reads_identity_without_data`
- `uid_only_emulation_is_an_error_without_the_identity_fallback`
- `phone_emulation_continues_after_a_timed_out_wrong_key`
- `keyscan_continues_after_a_timed_out_wrong_key`
- `read_timeout_marks_one_block_and_keeps_the_rest`
- `dump_reuses_the_last_authenticated_key`
- `dump_does_not_brute_force_the_full_common_dictionary`
- `read_card_dispatches_by_family` covers an unsupported SAK/ATQA returning
  the identity-only result variant.

Driver tests cover selective reselection without a field cycle and the
fallback to a full selection when the known card is gone. The GTK scenario
suite covers both identity-only paths. A phone/reader hardware run for a full
Xiaomi emulation is still required; the scripted tests cannot reproduce the
phone's RF/state-machine behavior.
