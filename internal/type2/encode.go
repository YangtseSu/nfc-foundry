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
	payload, err := textPayload(text, language)
	if err != nil {
		return nil, err
	}
	return encodeRecord(0x01, "T", payload)
}

func textPayload(text, language string) ([]byte, error) {
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
	return payload, nil
}

// EncodeURIMessage builds one NDEF URI record with the well-known "U" type.
// The longest known prefix is used when it matches; otherwise the full URI is
// stored with prefix code 0x00.
func EncodeURIMessage(uri string) ([]byte, error) {
	payload, err := uriPayload(uri)
	if err != nil {
		return nil, err
	}
	return encodeRecord(0x01, "U", payload)
}

func uriPayload(uri string) ([]byte, error) {
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
	return payload, nil
}

// SmartPosterOptions carries the optional Smart Poster metadata records.
type SmartPosterOptions struct {
	Language     string
	Action       string
	ResourceType string
	ResourceSize uint32
}

// EncodeSmartPosterMessage builds a Smart Poster record with a nested URI and
// optional title Text record.
func EncodeSmartPosterMessage(uri, title string) ([]byte, error) {
	return EncodeSmartPosterMessageWithOptions(uri, title, SmartPosterOptions{})
}

// EncodeSmartPosterMessageWithOptions builds a Smart Poster with URI, action,
// resource type, resource size, and title records in NFC Forum order.
func EncodeSmartPosterMessageWithOptions(uri, title string, options SmartPosterOptions) ([]byte, error) {
	payload, err := smartPosterPayload(uri, title, options)
	if err != nil {
		return nil, err
	}
	return encodeRecord(0x01, "Sp", payload)
}

func smartPosterPayload(uri, title string, options SmartPosterOptions) ([]byte, error) {
	uriRecordPayload, err := uriPayload(uri)
	if err != nil {
		return nil, err
	}
	language := strings.TrimSpace(options.Language)
	if language == "" {
		language = "en"
	}
	var frames [][]byte
	appendRecord := func(tnf byte, recordType string, payload []byte) error {
		frame, err := encodeRecordFrame(tnf, recordType, payload, len(frames) == 0, false)
		if err != nil {
			return err
		}
		frames = append(frames, frame)
		return nil
	}
	if err := appendRecord(0x01, "U", uriRecordPayload); err != nil {
		return nil, err
	}
	if action := strings.ToLower(strings.TrimSpace(options.Action)); action != "" {
		var code byte
		switch action {
		case "do":
			code = 0
		case "save":
			code = 1
		case "open":
			code = 2
		default:
			return nil, fmt.Errorf("%w: Smart Poster action must be do, save, or open", ErrMalformedNDEF)
		}
		if err := appendRecord(0x01, "act", []byte{code}); err != nil {
			return nil, err
		}
	}
	if resourceType := strings.TrimSpace(options.ResourceType); resourceType != "" {
		if !utf8.ValidString(resourceType) {
			return nil, fmt.Errorf("%w: Smart Poster resource type must be UTF-8", ErrMalformedNDEF)
		}
		if err := appendRecord(0x01, "t", []byte(resourceType)); err != nil {
			return nil, err
		}
	}
	if options.ResourceSize != 0 {
		var size [4]byte
		binary.BigEndian.PutUint32(size[:], options.ResourceSize)
		if err := appendRecord(0x01, "s", size[:]); err != nil {
			return nil, err
		}
	}
	if title != "" {
		titlePayload, err := textPayload(title, language)
		if err != nil {
			return nil, err
		}
		if err := appendRecord(0x01, "T", titlePayload); err != nil {
			return nil, err
		}
	}
	frames[len(frames)-1][0] |= 0x40
	var nested []byte
	for _, frame := range frames {
		nested = append(nested, frame...)
	}
	if _, err := ParseMessage(nested); err != nil {
		return nil, err
	}
	return nested, nil
}

// EncodeExternalMessage builds one external-type NDEF record.
func EncodeExternalMessage(recordType string, payload []byte) ([]byte, error) {
	if recordType == "" {
		return nil, fmt.Errorf("%w: external record type is required", ErrMalformedNDEF)
	}
	return encodeRecord(0x04, recordType, payload)
}

// EncodeMIMEMessage builds one MIME-type NDEF record.
func EncodeMIMEMessage(mimeType string, payload []byte) ([]byte, error) {
	if mimeType == "" {
		return nil, fmt.Errorf("%w: MIME type is required", ErrMalformedNDEF)
	}
	return encodeRecord(0x02, mimeType, payload)
}

func encodeRecord(tnf byte, recordType string, payload []byte) ([]byte, error) {
	record, err := encodeRecordFrame(tnf, recordType, payload, true, true)
	if err != nil {
		return nil, err
	}
	if _, err := ParseMessage(record); err != nil {
		return nil, err
	}
	return record, nil
}

func encodeRecordFrame(tnf byte, recordType string, payload []byte, first, last bool) ([]byte, error) {
	if len(recordType) > 0xFF {
		return nil, fmt.Errorf("%w: record type exceeds 255 bytes", ErrMalformedNDEF)
	}
	if uint64(len(payload)) > 0xFFFFFFFF {
		return nil, fmt.Errorf("%w: record payload is too large", ErrMalformedNDEF)
	}
	short := len(payload) <= 0xFF
	header := tnf & 0x07
	if first {
		header |= 0x80
	}
	if last {
		header |= 0x40
	}
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
	return record, nil
}
