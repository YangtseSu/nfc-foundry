// Package desfire implements read-only native DESFire commands over an
// ISO-DEP APDU exchanger.
package desfire

import (
	"context"
	"encoding/binary"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/isodep"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// Exchanger sends one command APDU and returns its response.
type Exchanger interface {
	Transmit(context.Context, []byte) (isodep.Response, error)
}

// StatusError is a DESFire status word returned inside a wrapped APDU.
type StatusError struct {
	Operation string
	SW1       byte
	SW2       byte
}

func (e *StatusError) Error() string {
	return fmt.Sprintf("DESFire %s failed: status %02X %02X (%s)", e.Operation, e.SW1, e.SW2, statusName(e.SW2))
}

func statusName(code byte) string {
	switch code {
	case 0x00:
		return "success"
	case 0x0C:
		return "no changes"
	case 0x1C:
		return "illegal command"
	case 0x1E:
		return "integrity error"
	case 0x40:
		return "no such key"
	case 0x41:
		return "key exists"
	case 0x42:
		return "authentication limit"
	case 0x43:
		return "no such version"
	case 0x45:
		return "memory error"
	case 0x7E:
		return "file not found"
	case 0x9D:
		return "permission denied"
	case 0xA0:
		return "application not found"
	case 0xA1:
		return "application integrity"
	case 0xAE:
		return "authentication required"
	case 0xAF:
		return "additional frame"
	case 0xBE:
		return "boundary error"
	case 0xDE:
		return "duplicate"
	case 0xEE:
		return "EEPROM error"
	case 0xF0:
		return "file exists"
	case 0xFD:
		return "format error"
	default:
		return "card error"
	}
}

// Version is the DESFire GetVersion response.
type Version struct {
	Vendor   byte
	Type     byte
	Subtype  byte
	Major    byte
	Minor    byte
	Storage  byte
	Protocol byte
}

// FileType is a DESFire file type.
type FileType byte

const (
	FileStandard FileType = 0x00
	FileBackup   FileType = 0x01
	FileValue    FileType = 0x02
	FileLinear   FileType = 0x03
	FileCyclic   FileType = 0x04
)

func (t FileType) String() string {
	switch t {
	case FileStandard:
		return "standard"
	case FileBackup:
		return "backup"
	case FileValue:
		return "value"
	case FileLinear:
		return "linear-record"
	case FileCyclic:
		return "cyclic-record"
	default:
		return fmt.Sprintf("unknown(0x%02X)", byte(t))
	}
}

// File describes one DESFire file without authentication or key material.
type File struct {
	ID              byte
	Type            FileType
	Communication   byte
	AccessRights    uint16
	Size            int
	RecordSize      int
	MaxRecords      int
	CurrentRecords  int
	ValueLower      int32
	ValueUpper      int32
	LimitedCredit   int32
	LimitedCreditOn bool
}

// Card is a read-only DESFire session.
type Card struct {
	exchange    Exchanger
	selectedAID uint32
	selected    bool
}

func New(exchange Exchanger) *Card {
	return &Card{exchange: exchange}
}

// Identify reads the native GetVersion response.
func (c *Card) Identify(ctx context.Context) (Version, error) {
	data, err := c.native(ctx, "GetVersion", 0x60, nil)
	if err != nil {
		return Version{}, err
	}
	if len(data) < 7 {
		return Version{}, nfc.NewError("DESFire GetVersion", nfc.CodeIO, fmt.Sprintf("response has %d bytes; want at least 7", len(data)), nil)
	}
	return Version{
		Vendor: data[0], Type: data[1], Subtype: data[2], Major: data[3],
		Minor: data[4], Storage: data[5], Protocol: data[6],
	}, nil
}

// Applications returns the native application IDs, decoded most-significant
// first for display.
func (c *Card) Applications(ctx context.Context) ([]uint32, error) {
	data, err := c.native(ctx, "GetApplicationIDs", 0x6A, nil)
	if err != nil {
		return nil, err
	}
	if len(data)%3 != 0 {
		return nil, nfc.NewError("DESFire GetApplicationIDs", nfc.CodeIO, fmt.Sprintf("response length %d is not a multiple of 3", len(data)), nil)
	}
	result := make([]uint32, 0, len(data)/3)
	for index := 0; index < len(data); index += 3 {
		result = append(result, decodeAID(data[index:index+3]))
	}
	return result, nil
}

// SelectApplication selects a native DESFire application.
func (c *Card) SelectApplication(ctx context.Context, aid uint32) error {
	if aid > 0xFFFFFF {
		return nfc.NewError("DESFire select application", nfc.CodeInvalidArgument, "application ID must fit in 3 bytes", nil)
	}
	if _, err := c.native(ctx, "SelectApplication", 0x5A, encodeAID(aid)); err != nil {
		return err
	}
	c.selectedAID = aid
	c.selected = true
	return nil
}

// Files returns the files in aid, including their settings. The caller must
// have selected aid or allow this method to select it.
func (c *Card) Files(ctx context.Context, aid uint32) ([]File, error) {
	if err := c.ensureApplication(ctx, aid); err != nil {
		return nil, err
	}
	ids, err := c.native(ctx, "GetFileIDs", 0x6F, nil)
	if err != nil {
		return nil, err
	}
	files := make([]File, 0, len(ids))
	for _, id := range ids {
		settings, err := c.native(ctx, "GetFileSettings", 0xF5, []byte{id})
		if err != nil {
			return nil, err
		}
		file, err := parseFileSettings(id, settings)
		if err != nil {
			return nil, err
		}
		files = append(files, file)
	}
	return files, nil
}

// ReadFile reads a standard or backup data file. Encrypted or MAC-protected
// communication modes are refused because no authentication is implemented.
func (c *Card) ReadFile(ctx context.Context, aid uint32, fileID byte) ([]byte, error) {
	if err := c.ensureApplication(ctx, aid); err != nil {
		return nil, err
	}
	settings, err := c.native(ctx, "GetFileSettings", 0xF5, []byte{fileID})
	if err != nil {
		return nil, err
	}
	file, err := parseFileSettings(fileID, settings)
	if err != nil {
		return nil, err
	}
	if file.Type != FileStandard && file.Type != FileBackup {
		return nil, nfc.NewError("DESFire ReadData", nfc.CodeUnsupported, fmt.Sprintf("file %d is %s; only standard and backup files can be read", fileID, file.Type), nil)
	}
	if file.Communication != 0 {
		return nil, nfc.NewError("DESFire ReadData", nfc.CodePermission, "file communication is MAC-protected or enciphered; authentication is not implemented", nil)
	}
	if file.Size < 0 || file.Size > 0xFFFFFF {
		return nil, nfc.NewError("DESFire ReadData", nfc.CodeInvalidArgument, "file size is outside the DESFire limit", nil)
	}
	result := make([]byte, 0, file.Size)
	const chunkSize = 0x80
	for offset := 0; offset < file.Size; offset += chunkSize {
		length := min(chunkSize, file.Size-offset)
		command := make([]byte, 7)
		command[0] = fileID
		putUint24(command[1:4], offset)
		putUint24(command[4:7], length)
		chunk, err := c.native(ctx, "ReadData", 0xBD, command)
		if err != nil {
			return nil, err
		}
		if len(chunk) != length {
			return nil, nfc.NewError("DESFire ReadData", nfc.CodeIO, fmt.Sprintf("read %d bytes at offset %d; expected %d", len(chunk), offset, length), nil)
		}
		result = append(result, chunk...)
	}
	return result, nil
}

func (c *Card) ensureApplication(ctx context.Context, aid uint32) error {
	if c.selected && c.selectedAID == aid {
		return nil
	}
	return c.SelectApplication(ctx, aid)
}

func (c *Card) native(ctx context.Context, operation string, instruction byte, data []byte) ([]byte, error) {
	apdu, err := isodep.BuildShort(0x90, instruction, 0x00, 0x00, data, 256)
	if err != nil {
		return nil, nfc.NewError("DESFire "+operation, nfc.CodeInvalidArgument, err.Error(), err)
	}
	response, err := c.exchange.Transmit(ctx, apdu)
	if err != nil {
		return nil, err
	}
	payload := append([]byte(nil), response.Data...)
	for response.SW() == 0x91AF {
		more, err := c.exchange.Transmit(ctx, []byte{0x90, 0xAF, 0x00, 0x00, 0x00})
		if err != nil {
			return nil, err
		}
		payload = append(payload, more.Data...)
		response = more
	}
	if response.SW() != 0x9100 {
		return nil, &StatusError{Operation: operation, SW1: response.SW1, SW2: response.SW2}
	}
	return payload, nil
}

func parseFileSettings(id byte, data []byte) (File, error) {
	if len(data) < 4 {
		return File{}, nfc.NewError("DESFire GetFileSettings", nfc.CodeIO, "file settings are shorter than the access header", nil)
	}
	file := File{
		ID: id, Type: FileType(data[0]), Communication: data[1],
		AccessRights: binary.LittleEndian.Uint16(data[2:4]),
	}
	switch file.Type {
	case FileStandard, FileBackup:
		if len(data) < 7 {
			return File{}, nfc.NewError("DESFire GetFileSettings", nfc.CodeIO, "standard file settings have no size", nil)
		}
		file.Size = getUint24(data[4:7])
	case FileValue:
		if len(data) < 17 {
			return File{}, nfc.NewError("DESFire GetFileSettings", nfc.CodeIO, "value file settings are truncated", nil)
		}
		file.ValueLower = int32(binary.LittleEndian.Uint32(data[4:8]))
		file.ValueUpper = int32(binary.LittleEndian.Uint32(data[8:12]))
		file.LimitedCredit = int32(binary.LittleEndian.Uint32(data[12:16]))
		file.LimitedCreditOn = data[16]&0x01 != 0
	case FileLinear, FileCyclic:
		if len(data) < 13 {
			return File{}, nfc.NewError("DESFire GetFileSettings", nfc.CodeIO, "record file settings are truncated", nil)
		}
		file.RecordSize = getUint24(data[4:7])
		file.MaxRecords = getUint24(data[7:10])
		file.CurrentRecords = getUint24(data[10:13])
	default:
		return File{}, nfc.NewError("DESFire GetFileSettings", nfc.CodeUnsupported, fmt.Sprintf("unknown file type 0x%02X", data[0]), nil)
	}
	return file, nil
}

func decodeAID(data []byte) uint32 {
	return uint32(data[0]) | uint32(data[1])<<8 | uint32(data[2])<<16
}

func encodeAID(aid uint32) []byte {
	return []byte{byte(aid), byte(aid >> 8), byte(aid >> 16)}
}

func putUint24(dst []byte, value int) {
	dst[0], dst[1], dst[2] = byte(value), byte(value>>8), byte(value>>16)
}

func getUint24(src []byte) int {
	return int(src[0]) | int(src[1])<<8 | int(src[2])<<16
}
