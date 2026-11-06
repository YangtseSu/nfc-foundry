package type2

import (
	"context"
	"errors"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

const (
	commandRead       byte = 0x30
	commandFastRead   byte = 0x3A
	commandWrite      byte = 0xA2
	commandPwdAuth    byte = 0x1B
	commandGetVersion byte = 0x60
	ackByte           byte = 0x0A
	maxFastReadPages       = 64
	maxFrame               = 264
)

var (
	// ErrNAK means the tag answered a command with a negative acknowledgement.
	ErrNAK = errors.New("type 2 tag returned a NAK")

	// ErrMalformedCC means page 3 is not a valid NFC Forum capability
	// container.
	ErrMalformedCC = errors.New("type 2 capability container is malformed")

	// ErrMalformedTLV means the TLV structure in user memory is truncated or
	// invalid.
	ErrMalformedTLV = errors.New("type 2 TLV structure is malformed")

	// ErrMalformedNDEF means the NDEF message does not follow the NDEF record
	// layout.
	ErrMalformedNDEF = errors.New("NDEF message is malformed")

	// ErrCapacity means an NDEF message or page operation does not fit the
	// tag's capacity.
	ErrCapacity = errors.New("type 2 operation exceeds tag capacity")

	// ErrProtectedPage means the requested page is not ordinary user memory.
	ErrProtectedPage = errors.New("type 2 page is protected or outside user memory")

	// ErrUnsupportedTLV means a write was refused because user memory contains
	// a lock, memory-control, or proprietary TLV that must not be replaced.
	ErrUnsupportedTLV = errors.New("type 2 user memory contains a protected TLV")
)

// FrameTransport is the narrow raw-frame capability a Type 2 session needs.
// nfc.RawTransceiver satisfies it.
type FrameTransport interface {
	SetRawMode(ctx context.Context, enabled bool) error
	TransceiveBytes(ctx context.Context, tx []byte) ([]byte, error)
}

// Session owns one raw-mode frame stream for a Type 2 command sequence. Raw
// mode disables libnfc CRC handling, so the session frames commands and checks
// response CRC-A itself. A session must be deactivated (or closed) before
// CardInfo reselects a card; reactivate it after selection.
type Session struct {
	transport FrameTransport
	active    bool
}

// NewSession creates an inactive Type 2 session.
func NewSession(transport FrameTransport) *Session {
	return &Session{transport: transport}
}

// OpenSession enters raw mode and returns an active session.
func OpenSession(ctx context.Context, transport FrameTransport) (*Session, error) {
	session := NewSession(transport)
	if err := session.Activate(ctx); err != nil {
		return nil, err
	}
	return session, nil
}

// Activate enters raw mode. Activating an already active session is a no-op.
func (s *Session) Activate(ctx context.Context) error {
	if s == nil || s.transport == nil {
		return nfc.NewError("open Type 2 session", nfc.CodeUnsupported, "raw-frame transport is unavailable", nil)
	}
	if s.active {
		return nil
	}
	if err := s.transport.SetRawMode(ctx, true); err != nil {
		return err
	}
	s.active = true
	return nil
}

// Deactivate leaves raw mode, which power-cycles the RF field and clears the
// selected target. Deactivating an inactive session is a no-op.
func (s *Session) Deactivate(ctx context.Context) error {
	if s == nil || s.transport == nil || !s.active {
		return nil
	}
	err := s.transport.SetRawMode(ctx, false)
	if err == nil {
		s.active = false
	}
	return err
}

// Close leaves raw mode on every exit path. Callers should pass a context that
// is not canceled when the operation is failing.
func (s *Session) Close(ctx context.Context) error { return s.Deactivate(ctx) }

func commandError(op string, detail string, cause error) error {
	return nfc.NewError(op, nfc.CodeIO, detail, cause)
}

func validationError(op string, cause error, detail string) error {
	return nfc.NewError(op, nfc.CodeInvalidArgument, detail, cause)
}

func isACK(response []byte) bool {
	return len(response) == 1 && (response[0] == ackByte || response[0] == ackByte<<4)
}

func isNAK(response []byte) bool {
	return len(response) == 1 && !isACK(response)
}

func nakError(op string, response []byte) error {
	return commandError(op, fmt.Sprintf("tag returned NAK 0x%02X", response[0]), ErrNAK)
}

// transact sends one CRC-A framed command inside an active session and returns
// the raw response (ACK/NAK or data + CRC-A).
func (s *Session) transact(ctx context.Context, tx []byte) ([]byte, error) {
	if s == nil || s.transport == nil {
		return nil, nfc.NewError("type 2 command", nfc.CodeUnsupported, "raw-frame transport is unavailable", nil)
	}
	if !s.active {
		return nil, nfc.NewError("type 2 command", nfc.CodeNotOpen, "Type 2 session is not active", nil)
	}
	if len(tx) == 0 || len(tx)+2 > maxFrame {
		return nil, nfc.NewError("type 2 command", nfc.CodeInvalidArgument, "invalid Type 2 frame length", nil)
	}
	return s.transport.TransceiveBytes(ctx, iso14443a.WithCRCA(tx))
}

// payload validates a data response's CRC-A and returns the payload.
func payload(op string, response []byte) ([]byte, error) {
	if len(response) < 3 {
		return nil, commandError(op, fmt.Sprintf("tag returned a truncated %d-byte data frame", len(response)), nil)
	}
	body := response[:len(response)-2]
	crc := iso14443a.CRCA(body)
	if response[len(response)-2] != crc[0] || response[len(response)-1] != crc[1] {
		return nil, commandError(op, "tag response CRC-A is invalid", nil)
	}
	return body, nil
}

func dataResponse(op string, response []byte, expected int) ([]byte, error) {
	if isACK(response) {
		return nil, commandError(op, "tag answered a data command with ACK", ErrNAK)
	}
	if isNAK(response) {
		return nil, nakError(op, response)
	}
	data, err := payload(op, response)
	if err != nil {
		return nil, err
	}
	if len(data) != expected {
		return nil, commandError(op, fmt.Sprintf("tag returned %d data bytes; expected %d", len(data), expected), nil)
	}
	return data, nil
}

// GetVersion returns the eight-byte product version response.
func (s *Session) GetVersion(ctx context.Context) ([]byte, error) {
	response, err := s.transact(ctx, []byte{commandGetVersion})
	if err != nil {
		return nil, err
	}
	data, err := dataResponse("get version", response, 8)
	if err != nil {
		return nil, err
	}
	return append([]byte(nil), data...), nil
}

// ReadFourPages sends READ (0x30) and returns the sixteen bytes starting at
// page. The caller must ensure that page+3 is within the tag's memory.
func (s *Session) ReadFourPages(ctx context.Context, page int) ([]byte, error) {
	if page < 0 || page > 0xFF {
		return nil, validationError("read pages", nil, "page address is outside 0-255")
	}
	response, err := s.transact(ctx, []byte{commandRead, byte(page)})
	if err != nil {
		return nil, err
	}
	data, err := dataResponse("read pages", response, 16)
	if err != nil {
		return nil, err
	}
	return append([]byte(nil), data...), nil
}

// FastRead reads the inclusive page range with FAST_READ (0x3A).
func (s *Session) FastRead(ctx context.Context, start, end int) ([]byte, error) {
	if start < 0 || end < start || end > 0xFF {
		return nil, validationError("fast read", nil, "invalid page range")
	}
	pages := end - start + 1
	if pages > maxFastReadPages {
		return nil, validationError("fast read", nil, fmt.Sprintf("page range exceeds %d pages", maxFastReadPages))
	}
	response, err := s.transact(ctx, []byte{commandFastRead, byte(start), byte(end)})
	if err != nil {
		return nil, err
	}
	data, err := dataResponse("fast read", response, pages*4)
	if err != nil {
		return nil, err
	}
	return append([]byte(nil), data...), nil
}

// ReadPage reads exactly one page using FAST_READ with a single-page range.
func (s *Session) ReadPage(ctx context.Context, page int) ([4]byte, error) {
	var data [4]byte
	response, err := s.FastRead(ctx, page, page)
	if err != nil {
		return data, err
	}
	copy(data[:], response)
	return data, nil
}

// WritePage sends WRITE (0xA2) and requires an ACK. Verification is separate
// so the workflow can report a known write with a failed readback accurately.
func (s *Session) WritePage(ctx context.Context, page int, data [4]byte) error {
	if page < 0 || page > 0xFF {
		return validationError("write page", nil, "page address is outside 0-255")
	}
	response, err := s.transact(ctx, []byte{commandWrite, byte(page), data[0], data[1], data[2], data[3]})
	if err != nil {
		return err
	}
	if isACK(response) {
		return nil
	}
	if isNAK(response) {
		return nakError("write page", response)
	}
	return commandError("write page", fmt.Sprintf("tag returned an unexpected %d-byte write response", len(response)), nil)
}

// WritePageVerified writes one page and reads it back.
func (s *Session) WritePageVerified(ctx context.Context, page int, data [4]byte) error {
	if err := s.WritePage(ctx, page, data); err != nil {
		return err
	}
	actual, err := s.ReadPage(ctx, page)
	if err != nil {
		return err
	}
	if actual != data {
		return nfc.NewError("verify page", nfc.CodeVerificationFailed,
			fmt.Sprintf("page %d readback % X does not match written % X", page, actual, data), nil)
	}
	return nil
}

// PwdAuth authenticates with a four-byte password and returns the two-byte
// PACK response.
func (s *Session) PwdAuth(ctx context.Context, password [4]byte) ([2]byte, error) {
	response, err := s.transact(ctx, []byte{commandPwdAuth, password[0], password[1], password[2], password[3]})
	if err != nil {
		return [2]byte{}, err
	}
	if isNAK(response) {
		return [2]byte{}, nfc.NewError("password authentication", nfc.CodeAuthenticationFailed,
			fmt.Sprintf("tag rejected PWD_AUTH with NAK 0x%02X", response[0]), ErrNAK)
	}
	data, err := dataResponse("password authentication", response, 2)
	if err != nil {
		return [2]byte{}, err
	}
	return [2]byte{data[0], data[1]}, nil
}
