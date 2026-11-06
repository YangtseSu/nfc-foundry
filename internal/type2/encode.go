package type2

import (
	"encoding/binary"
	"fmt"
	"strings"
	"unicode/utf8"
)

// EncodeTextMessage builds one NDEF Text record with the well-known "T" type.
// The language tag is the RFC 5646 tag stored in the record (for example
// "en"); the text is encoded as UTF-8.
func EncodeTextMessage(text, language string) ([]byte, error) {
	if !utf8.ValidString(text) {
		return nil, fmt.Errorf("%w: text is not valid UTF-8", ErrMalformedNDEF)
	}
	if !utf8.ValidString(language) || len(language) > 0x3F {
		return nil, fmt.Errorf("%w: language tag must be valid UTF-8 and at most 63 bytes", ErrMalformedNDEF)
	}
	payload := make([]byte, 0, 1+len(language)+len(text))
	payload = append(payload, byte(len(language)))
	payload = append(payload, language...)
	payload = append(payload, text...)
	return encodeRecord(0x01, "T", payload)
}

// EncodeURIMessage builds one NDEF URI record with the well-known "U" type.
// The longest known prefix is used when it matches; otherwise the full URI is
// stored with prefix code 0x00.
func EncodeURIMessage(uri string) ([]byte, error) {
	if uri == "" || !utf8.ValidString(uri) {
		return nil, fmt.Errorf("%w: URI must be non-empty UTF-8", ErrMalformedNDEF)
	}
	prefixCode := byte(0)
	remainder := uri
	bestLength := 0
	for code, prefix := range uriPrefixes {
		if code == 0 || prefix == "" {
			continue
		}
		if strings.HasPrefix(uri, prefix) && len(prefix) > bestLength {
			prefixCode = byte(code)
			remainder = uri[len(prefix):]
			bestLength = len(prefix)
		}
	}
	payload := make([]byte, 0, 1+len(remainder))
	payload = append(payload, prefixCode)
	payload = append(payload, remainder...)
	return encodeRecord(0x01, "U", payload)
}

func encodeRecord(tnf byte, recordType string, payload []byte) ([]byte, error) {
	if len(recordType) > 0xFF {
		return nil, fmt.Errorf("%w: record type exceeds 255 bytes", ErrMalformedNDEF)
	}
	if uint64(len(payload)) > 0xFFFFFFFF {
		return nil, fmt.Errorf("%w: record payload is too large", ErrMalformedNDEF)
	}
	short := len(payload) <= 0xFF
	header := byte(0xC0) | tnf&0x07
	if short {
		header |= 0x10
	}
	record := make([]byte, 0, 2+len(recordType)+len(payload)+4)
	record = append(record, header, byte(len(recordType)))
	if short {
		record = append(record, byte(len(payload)))
	} else {
		var length [4]byte
		binary.BigEndian.PutUint32(length[:], uint32(len(payload)))
		record = append(record, length[:]...)
	}
	record = append(record, recordType...)
	record = append(record, payload...)
	if _, err := ParseMessage(record); err != nil {
		return nil, err
	}
	return record, nil
}
