// Package isodep implements ISO/IEC 14443-4 Type A activation and block
// framing over the raw NFC transport.
package isodep

import (
	"errors"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
)

// ATS is a validated Answer To Select.
type ATS struct {
	Raw        []byte
	TL         int
	T0         byte
	FSCI       int
	FSC        int
	FWI        int
	TA         *byte
	TB         *byte
	TC         *byte
	Historical []byte
}

var fscTable = [...]int{16, 24, 32, 40, 48, 64, 96, 128, 256}

// ParseATS validates the CRC and length of an ATS frame and parses its
// optional interface bytes.
func ParseATS(frame []byte) (ATS, error) {
	if len(frame) < 3 {
		return ATS{}, errors.New("ATS is shorter than TL and CRC")
	}
	tl := int(frame[0])
	if tl < 1 || len(frame) != tl+2 {
		return ATS{}, fmt.Errorf("ATS length byte %d does not match %d received bytes", tl, len(frame))
	}
	body := frame[:tl]
	crc := iso14443a.CRCA(body)
	if frame[tl] != crc[0] || frame[tl+1] != crc[1] {
		return ATS{}, errors.New("ATS CRC does not match")
	}
	ats := ATS{Raw: append([]byte(nil), frame...), TL: tl, FSCI: 2, FWI: 4}
	if tl == 1 {
		ats.FSC = fscTable[ats.FSCI]
		return ats, nil
	}
	ats.T0 = body[1]
	ats.FSCI = int(ats.T0 & 0x0F)
	if ats.FSCI > 8 {
		return ATS{}, fmt.Errorf("ATS FSCI %d is reserved", ats.FSCI)
	}
	ats.FSC = fscTable[ats.FSCI]
	position := 2
	if ats.T0&0x10 != 0 {
		if position >= len(body) {
			return ATS{}, errors.New("ATS advertises TA(1) but it is missing")
		}
		value := body[position]
		ats.TA = &value
		position++
	}
	if ats.T0&0x20 != 0 {
		if position >= len(body) {
			return ATS{}, errors.New("ATS advertises TB(1) but it is missing")
		}
		value := body[position]
		ats.TB = &value
		ats.FWI = int(value >> 4)
		if ats.FWI > 14 {
			return ATS{}, fmt.Errorf("ATS FWI %d is reserved", ats.FWI)
		}
		position++
	}
	if ats.T0&0x40 != 0 {
		if position >= len(body) {
			return ATS{}, errors.New("ATS advertises TC(1) but it is missing")
		}
		value := body[position]
		ats.TC = &value
		position++
	}
	if position > len(body) {
		return ATS{}, errors.New("ATS interface bytes exceed TL")
	}
	ats.Historical = append([]byte(nil), body[position:]...)
	return ats, nil
}

// RATS returns the Request for Answer To Select frame, including CRC-A.
func RATS(fsdi, cid byte) ([]byte, error) {
	if fsdi > 8 {
		return nil, fmt.Errorf("RATS FSDI %d is reserved", fsdi)
	}
	if cid > 14 {
		return nil, fmt.Errorf("RATS CID %d is reserved", cid)
	}
	return iso14443a.WithCRCA([]byte{0xE0, fsdi<<4 | cid}), nil
}
