// SPDX-FileCopyrightText: 2011 Adam Laurie
// SPDX-FileCopyrightText: 2014 Dario Carluccio
// SPDX-FileCopyrightText: 2026 NFC Foundry contributors
// SPDX-License-Identifier: BSD-2-Clause OR GPL-3.0-or-later

package mifare

import (
	"context"
	"errors"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// The Gen1A command sequence is ported from the libnfc nfc-mfsetuid example
// (BSD-2-Clause). The notice is retained in THIRD_PARTY_NOTICES.md and
// third_party/libnfc/nfc-mfsetuid-BSD-2-Clause.txt.
const (
	gen1aUnlockFirst  byte = 0x40
	gen1aUnlockSecond byte = 0x43
	mifareHalt        byte = 0x50
	mifareWrite       byte = 0xA0
	mifareACK         byte = 0x0A
)

var (
	// ErrGen1ANotSupported means the card did not accept the 7-bit 0x40
	// unlock command or the 0x43 second step. Ordinary cards end here.
	ErrGen1ANotSupported = errors.New("card does not answer the Gen1A unlock sequence")

	// ErrGen1AWriteUncertain means the unlock succeeded but a write frame was
	// not acknowledged. The caller must reselect and verify the card; a
	// matching new UID and block 0 still count as success.
	ErrGen1AWriteUncertain = errors.New("Gen1A block 0 write response was not acknowledged")
)

func isGen1AACK(response []byte) bool {
	return len(response) == 1 && (response[0] == mifareACK || response[0] == mifareACK<<4)
}

// WriteGen1ABlock0 unlocks a 4-byte UID Classic 1K Gen1A card and writes the
// complete manufacturer block in-process. The caller must have selected the
// expected card and must reselect and verify the card before reporting success.
func WriteGen1ABlock0(ctx context.Context, transport nfc.RawTransceiver, card nfc.CardInfo, block0 [nfc.BlockSize]byte) (err error) {
	if transport == nil {
		return nfc.NewError("Gen1A write", nfc.CodeUnsupported, "raw-frame transport is unavailable", nil)
	}
	if len(card.UID) != 4 || nfc.InferCardType(card) != nfc.CardTypeMIFAREClassic1K {
		return nfc.NewError("Gen1A write", nfc.CodeUnsupported, "only 4-byte UID MIFARE Classic 1K cards support the Gen1A path", nil)
	}
	bcc, bccErr := BCCFor4ByteUID(block0[0:4])
	if bccErr != nil || block0[4] != bcc {
		return nfc.NewError("Gen1A write", nfc.CodeInvalidArgument, "planned block 0 has an invalid BCC", bccErr)
	}

	if err := transport.SetRawMode(ctx, true); err != nil {
		return nfc.NewError("Gen1A write", nfc.CodeUnsupported, "reader could not enter raw mode", err)
	}
	defer func() {
		cleanupErr := transport.SetRawMode(context.WithoutCancel(ctx), false)
		if cleanupErr == nil {
			return
		}
		cleanupErr = nfc.NewError("Gen1A write", nfc.CodeIO, "reader could not leave raw mode after the Gen1A sequence", cleanupErr)
		if err == nil {
			err = cleanupErr
		} else {
			err = errors.Join(err, cleanupErr)
		}
	}()

	if err := gen1aHalt(ctx, transport); err != nil {
		return err
	}
	if err := gen1aUnlock(ctx, transport); err != nil {
		return err
	}
	return gen1aWrite(ctx, transport, block0)
}

func gen1aHalt(ctx context.Context, transport nfc.RawTransceiver) error {
	_, err := transport.TransceiveBytes(ctx, iso14443a.WithCRCA([]byte{mifareHalt, 0x00}))
	if err == nil {
		return nil
	}
	if fatalRawError(ctx, err) {
		return err
	}
	// A halted card does not answer. A timeout or missing target here is the
	// expected result; the unlock step is the authoritative capability check.
	return nil
}

func gen1aUnlock(ctx context.Context, transport nfc.RawTransceiver) error {
	response, bits, err := transport.TransceiveBits(ctx, []byte{gen1aUnlockFirst}, 7)
	if err != nil {
		if fatalRawError(ctx, err) {
			return err
		}
		return nfc.NewError("Gen1A unlock", nfc.CodeUnsupported,
			"the card did not answer the 7-bit 0x40 Gen1A unlock command; it is not a Gen1A magic card or its backdoor is locked",
			errors.Join(ErrGen1ANotSupported, err))
	}
	if bits != 4 || !isGen1AACK(response) {
		return nfc.NewError("Gen1A unlock", nfc.CodeUnsupported,
			"the card answered the 7-bit 0x40 Gen1A unlock command with an unexpected frame",
			ErrGen1ANotSupported)
	}
	response, err = transport.TransceiveBytes(ctx, []byte{gen1aUnlockSecond})
	if err != nil {
		if fatalRawError(ctx, err) {
			return err
		}
		return nfc.NewError("Gen1A unlock", nfc.CodeUnsupported,
			"the card accepted the 0x40 step but refused the 0x43 Gen1A unlock command",
			errors.Join(ErrGen1ANotSupported, err))
	}
	if !isGen1AACK(response) {
		return nfc.NewError("Gen1A unlock", nfc.CodeUnsupported,
			"the card accepted the 0x40 step but answered 0x43 with an unexpected frame",
			ErrGen1ANotSupported)
	}
	return nil
}

func gen1aWrite(ctx context.Context, transport nfc.RawTransceiver, block0 [nfc.BlockSize]byte) error {
	response, err := transport.TransceiveBytes(ctx, iso14443a.WithCRCA([]byte{mifareWrite, 0x00}))
	if err != nil {
		if fatalRawError(ctx, err) {
			return err
		}
		return nfc.NewError("Gen1A write", nfc.CodeIO,
			"the card did not acknowledge the Gen1A block 0 write command",
			errors.Join(ErrGen1AWriteUncertain, err))
	}
	if !isGen1AACK(response) {
		return nfc.NewError("Gen1A write", nfc.CodeIO,
			"the card answered the Gen1A block 0 write command with an unexpected frame",
			ErrGen1AWriteUncertain)
	}
	response, err = transport.TransceiveBytes(ctx, iso14443a.WithCRCA(block0[:]))
	if err != nil {
		if fatalRawError(ctx, err) {
			return err
		}
		return nfc.NewError("Gen1A write", nfc.CodeIO,
			"the block 0 data frame was not acknowledged; reselect and verify the card",
			errors.Join(ErrGen1AWriteUncertain, err))
	}
	if !isGen1AACK(response) {
		return nfc.NewError("Gen1A write", nfc.CodeIO,
			"the block 0 data frame was answered with an unexpected frame; reselect and verify the card",
			ErrGen1AWriteUncertain)
	}
	return nil
}

func fatalRawError(ctx context.Context, err error) bool {
	if ctx != nil && ctx.Err() != nil {
		return true
	}
	return errors.Is(err, nfc.ErrCanceled) ||
		errors.Is(err, nfc.ErrDeviceDisconnected) ||
		errors.Is(err, nfc.ErrNotOpen) ||
		errors.Is(err, nfc.ErrBusy) ||
		errors.Is(err, nfc.ErrPermission) ||
		errors.Is(err, nfc.ErrUnsupported)
}
