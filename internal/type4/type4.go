// Package type4 implements the NFC Forum Type 4 NDEF tag application over an
// ISO-DEP APDU exchanger.
package type4

import (
	"bytes"
	"context"
	"encoding/binary"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/isodep"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

const (
	ndefAID      = "\xD2\x76\x00\x00\x85\x01\x01"
	ccFileID     = 0xE103
	defaultMLe   = 0xFF
	defaultMLc   = 0xFF
	selectByAID  = 0x04
	selectByFile = 0x00
)

// Exchanger sends one command APDU and returns its response.
type Exchanger interface {
	Transmit(context.Context, []byte) (isodep.Response, error)
}

// CC is the validated Type 4 capability container.
type CC struct {
	Raw            []byte
	Length         int
	MappingVersion byte
	MLe            int
	MLc            int
	FileID         uint16
	MaxMessage     int
	ReadAccess     byte
	WriteAccess    byte
}

// Document is the CC and NDEF message read from a Type 4 tag.
type Document struct {
	CC      CC
	Message []byte
	NDEF    *type2.NDEF
}

// WriteResult reports the old message and the verified final document.
type WriteResult struct {
	Previous []byte
	Final    Document
}

// StatusError is a non-success ISO 7816-4 status word.
type StatusError struct {
	Operation string
	SW1       byte
	SW2       byte
}

func (e *StatusError) Error() string {
	return fmt.Sprintf("Type 4 %s failed: status %02X %02X (%s)", e.Operation, e.SW1, e.SW2, statusName(e.SW2))
}

func statusName(code byte) string {
	switch code {
	case 0x00:
		return "success"
	case 0x82:
		return "end of file or security status"
	case 0x86:
		return "incorrect parameters"
	case 0x87:
		return "length or reference error"
	case 0x88:
		return "referenced data not found"
	case 0x90:
		return "command not allowed"
	case 0xD6:
		return "file not found"
	default:
		return "card error"
	}
}

// Tag is a Type 4 NDEF application on one ISO-DEP session.
type Tag struct {
	exchange Exchanger
}

func New(exchange Exchanger) *Tag { return &Tag{exchange: exchange} }

// Read selects the NDEF application, validates the CC, and reads the message.
func (t *Tag) Read(ctx context.Context) (Document, error) {
	cc, err := t.readCC(ctx)
	if err != nil {
		return Document{}, err
	}
	if cc.ReadAccess != 0x00 {
		return Document{}, nfc.NewError("Type 4 read", nfc.CodePermission, fmt.Sprintf("NDEF file read access is 0x%02X; authentication is not implemented", cc.ReadAccess), nil)
	}
	message, err := t.readMessage(ctx, cc)
	if err != nil {
		return Document{}, err
	}
	document := Document{CC: cc, Message: message, NDEF: &type2.NDEF{}}
	if len(message) != 0 {
		document.NDEF, err = parseMessage(message)
		if err != nil {
			return Document{}, nfc.NewError("Type 4 NDEF decode", nfc.CodeInvalidArgument, err.Error(), err)
		}
	}
	return document, nil
}

// Write replaces the NDEF message and verifies the final file. The caller is
// responsible for backing up Previous before applying the result.
func (t *Tag) Write(ctx context.Context, message []byte) (WriteResult, error) {
	if _, err := type2.ParseMessage(message); err != nil {
		return WriteResult{}, nfc.NewError("Type 4 NDEF encode", nfc.CodeInvalidArgument, err.Error(), err)
	}
	document, err := t.Read(ctx)
	if err != nil {
		return WriteResult{}, err
	}
	if document.CC.WriteAccess != 0x00 {
		return WriteResult{}, nfc.NewError("Type 4 write", nfc.CodePermission, fmt.Sprintf("NDEF file write access is 0x%02X; authentication is not implemented", document.CC.WriteAccess), nil)
	}
	if len(message) > document.CC.MaxMessage {
		return WriteResult{}, nfc.NewError("Type 4 write", nfc.CodeInvalidArgument, fmt.Sprintf("message needs %d bytes; NDEF file limit is %d", len(message), document.CC.MaxMessage), nil)
	}
	if err := t.writeMessage(ctx, document.CC, message); err != nil {
		return WriteResult{Previous: document.Message}, err
	}
	final, err := t.Read(ctx)
	if err != nil {
		return WriteResult{Previous: document.Message}, err
	}
	if !bytes.Equal(final.Message, message) {
		return WriteResult{Previous: document.Message}, nfc.NewError("Type 4 write", nfc.CodeVerificationFailed, "NDEF file does not match the requested message", nil)
	}
	return WriteResult{Previous: document.Message, Final: final}, nil
}

func (t *Tag) readCC(ctx context.Context) (CC, error) {
	if err := t.selectApplication(ctx); err != nil {
		return CC{}, err
	}
	if err := t.selectFile(ctx, ccFileID); err != nil {
		return CC{}, err
	}
	raw, err := t.readBinary(ctx, 0, 15, 0xFF)
	if err != nil {
		return CC{}, err
	}
	if len(raw) < 7 {
		return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, "capability container is shorter than 7 bytes", nil)
	}
	length := int(binary.BigEndian.Uint16(raw[0:2]))
	if length < 7 {
		return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, fmt.Sprintf("CCLEN %d is too small", length), nil)
	}
	if length > len(raw) {
		raw, err = t.readBinary(ctx, 0, length, 0xFF)
		if err != nil {
			return CC{}, err
		}
	}
	if length > len(raw) {
		return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, "CCLEN exceeds the CC file data", nil)
	}
	cc := CC{
		Raw: append([]byte(nil), raw[:length]...), Length: length,
		MappingVersion: raw[2],
		MLe:            int(binary.BigEndian.Uint16(raw[3:5])),
		MLc:            int(binary.BigEndian.Uint16(raw[5:7])),
	}
	if cc.MLe == 0 {
		cc.MLe = defaultMLe
	}
	if cc.MLc == 0 {
		cc.MLc = defaultMLc
	}
	position := 7
	found := false
	for position < len(cc.Raw) {
		tag := cc.Raw[position]
		position++
		if tag == 0x00 || tag == 0xFE {
			continue
		}
		if position >= len(cc.Raw) {
			return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, "CC TLV has no length", nil)
		}
		size := int(cc.Raw[position])
		position++
		if size == 0xFF {
			if position+2 > len(cc.Raw) {
				return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, "CC TLV long length is truncated", nil)
			}
			size = int(binary.BigEndian.Uint16(cc.Raw[position : position+2]))
			position += 2
		}
		if position+size > len(cc.Raw) {
			return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, "CC TLV exceeds CCLEN", nil)
		}
		value := cc.Raw[position : position+size]
		position += size
		if tag != 0x04 {
			continue
		}
		if size != 6 {
			return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, fmt.Sprintf("NDEF file-control TLV has length %d; want 6", size), nil)
		}
		cc.FileID = binary.BigEndian.Uint16(value[0:2])
		cc.MaxMessage = int(binary.BigEndian.Uint16(value[2:4]))
		cc.ReadAccess = value[4]
		cc.WriteAccess = value[5]
		found = true
	}
	if !found {
		return CC{}, nfc.NewError("Type 4 CC", nfc.CodeInvalidArgument, "CC has no NDEF file-control TLV", nil)
	}
	if cc.MappingVersion>>4 < 1 || cc.MappingVersion>>4 > 2 {
		return CC{}, nfc.NewError("Type 4 CC", nfc.CodeUnsupported, fmt.Sprintf("mapping version 0x%02X is not supported", cc.MappingVersion), nil)
	}
	return cc, nil
}

func (t *Tag) readMessage(ctx context.Context, cc CC) ([]byte, error) {
	if err := t.selectFile(ctx, cc.FileID); err != nil {
		return nil, err
	}
	nlen, err := t.readBinary(ctx, 0, 2, 0xFF)
	if err != nil {
		return nil, err
	}
	if len(nlen) != 2 {
		return nil, nfc.NewError("Type 4 NDEF", nfc.CodeIO, "NLEN is not two bytes", nil)
	}
	length := int(binary.BigEndian.Uint16(nlen))
	if length == 0 {
		return nil, nil
	}
	if length > cc.MaxMessage {
		return nil, nfc.NewError("Type 4 NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("NLEN %d exceeds the CC limit %d", length, cc.MaxMessage), nil)
	}
	limit := cc.MLe
	if limit <= 0 || limit > 0xFF {
		limit = 0xFF
	}
	return t.readBinary(ctx, 2, length, limit)
}

func (t *Tag) writeMessage(ctx context.Context, cc CC, message []byte) error {
	if err := t.selectFile(ctx, cc.FileID); err != nil {
		return err
	}
	if err := t.updateBinary(ctx, 0, []byte{0x00, 0x00}); err != nil {
		return err
	}
	chunk := cc.MLc
	if chunk <= 0 || chunk > 0xFF {
		chunk = 0xFF
	}
	for offset := 0; offset < len(message); offset += chunk {
		size := min(chunk, len(message)-offset)
		if err := t.updateBinary(ctx, 2+offset, message[offset:offset+size]); err != nil {
			return err
		}
	}
	length := []byte{byte(len(message) >> 8), byte(len(message))}
	return t.updateBinary(ctx, 0, length)
}

func (t *Tag) selectApplication(ctx context.Context) error {
	apdu, err := isodep.BuildShort(0x00, 0xA4, selectByAID, 0x00, []byte(ndefAID), 256)
	if err != nil {
		return err
	}
	_, err = t.exchangeOK(ctx, "SelectApplication", apdu)
	return err
}

func (t *Tag) selectFile(ctx context.Context, fileID uint16) error {
	data := []byte{byte(fileID >> 8), byte(fileID)}
	apdu, err := isodep.BuildShort(0x00, 0xA4, selectByFile, 0x0C, data, 0)
	if err != nil {
		return err
	}
	_, err = t.exchangeOK(ctx, "SelectFile", apdu)
	return err
}

func (t *Tag) readBinary(ctx context.Context, offset, length, limit int) ([]byte, error) {
	if offset < 0 || offset > 0xFFFF || length < 0 || length > 0xFFFF {
		return nil, nfc.NewError("Type 4 ReadBinary", nfc.CodeInvalidArgument, "offset or length is outside the APDU range", nil)
	}
	if limit <= 0 || limit > 0xFF {
		limit = 0xFF
	}
	position := 0
	result := make([]byte, 0, length)
	for position < length {
		size := min(limit, length-position)
		for attempt := 0; ; attempt++ {
			apdu, err := isodep.BuildShort(0x00, 0xB0, byte((offset+position)>>8), byte(offset+position), nil, size)
			if err != nil {
				return nil, err
			}
			response, err := t.exchange.Transmit(ctx, apdu)
			if err != nil {
				return nil, err
			}
			if response.SW1 == 0x6C && attempt == 0 {
				size = int(response.SW2)
				if size == 0 {
					size = 256
				}
				size = min(size, limit)
				size = min(size, length-position)
				continue
			}
			chunk, err := t.finishResponse(ctx, "ReadBinary", response)
			if err != nil {
				return nil, err
			}
			if len(chunk) == 0 {
				return nil, nfc.NewError("Type 4 ReadBinary", nfc.CodeIO, "empty ReadBinary response", nil)
			}
			if len(chunk) > size {
				chunk = chunk[:size]
			}
			result = append(result, chunk...)
			position += len(chunk)
			break
		}
	}
	return result, nil
}

func (t *Tag) updateBinary(ctx context.Context, offset int, data []byte) error {
	if offset < 0 || offset > 0xFFFF || len(data) > 0xFF {
		return nfc.NewError("Type 4 UpdateBinary", nfc.CodeInvalidArgument, "offset or data length is outside the short APDU range", nil)
	}
	apdu, err := isodep.BuildShort(0x00, 0xD6, byte(offset>>8), byte(offset), data, 0)
	if err != nil {
		return err
	}
	_, err = t.exchangeOK(ctx, "UpdateBinary", apdu)
	return err
}

func (t *Tag) exchangeOK(ctx context.Context, operation string, apdu []byte) ([]byte, error) {
	response, err := t.exchange.Transmit(ctx, apdu)
	if err != nil {
		return nil, err
	}
	return t.finishResponse(ctx, operation, response)
}

func (t *Tag) finishResponse(ctx context.Context, operation string, response isodep.Response) ([]byte, error) {
	data := append([]byte(nil), response.Data...)
	for response.SW1 == 0x61 {
		next, ok := isodep.GetResponse(response)
		if !ok {
			break
		}
		var err error
		response, err = t.exchange.Transmit(ctx, next)
		if err != nil {
			return nil, err
		}
		data = append(data, response.Data...)
	}
	if response.SW() != 0x9000 {
		return nil, &StatusError{Operation: operation, SW1: response.SW1, SW2: response.SW2}
	}
	return data, nil
}

func parseMessage(message []byte) (*type2.NDEF, error) {
	area := []byte{0x03}
	if len(message) > 0xFE {
		area = append(area, 0xFF, byte(len(message)>>8), byte(len(message)))
	} else {
		area = append(area, byte(len(message)))
	}
	area = append(area, message...)
	area = append(area, 0xFE)
	return type2.ParseNDEF(area)
}
