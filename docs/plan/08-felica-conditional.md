# Phase 08: FeliCa / Type 3 NDEF scope (conditional)

## Goal

Decide whether FeliCa/Type 3 NDEF belongs in NFC Foundry before writing code.

## Questions to answer

- Type 3 NDEF read only, or read/write?
- Which system codes, service codes, and memory layouts are in scope?
- Which readers are validated for FeliCa on Linux?
- How are IDm/PMm and NDEF errors presented without implying broader FeliCa
  support?

## Entry criteria

- Phase 05 NDEF abstraction is stable.
- A test reader and FeliCa tags are available.
- The scope is written as a spec and accepted.

## Non-goals

- Raw FeliCa service browsing.
- FeliCa card emulation.
- Any advertised support before hardware validation.

If the scope is not accepted, keep FeliCa out of the support matrix.
