package isodep

import (
	"context"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/iso14443a"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// Session is one activated ISO-DEP card. It owns raw mode for its lifetime.
type Session struct {
	transport nfc.RawTransceiver
	ATS       ATS
	pni       byte
	fsc       int
	closed    bool
}

// OpenSession selects ISO-DEP with RATS and validates the ATS. The caller must
// have selected the card immediately before opening the session.
func OpenSession(ctx context.Context, transport nfc.RawTransceiver, card nfc.CardInfo) (*Session, error) {
	if transport == nil {
		return nil, nfc.NewError("ISO-DEP activate", nfc.CodeUnsupported, "raw-frame transport is unavailable", nil)
	}
	if card.SAK&0x20 == 0 {
		return nil, nfc.NewError("ISO-DEP activate", nfc.CodeUnsupported, "card does not advertise ISO/IEC 14443-4", nil)
	}
	if err := transport.SetRawMode(ctx, true); err != nil {
		return nil, nfc.NewError("ISO-DEP activate", nfc.CodeUnsupported, "reader could not enter raw mode", err)
	}
	cleanup := true
	defer func() {
		if cleanup {
			_ = transport.SetRawMode(context.WithoutCancel(ctx), false)
		}
	}()
	rats, err := RATS(8, 0)
	if err != nil {
		return nil, nfc.NewError("ISO-DEP activate", nfc.CodeInvalidArgument, err.Error(), err)
	}
	response, err := transport.TransceiveBytes(ctx, rats)
	if err != nil {
		return nil, nfc.NewError("ISO-DEP activate", nfc.CodeIO, "RATS failed", err)
	}
	ats, err := ParseATS(response)
	if err != nil {
		return nil, nfc.NewError("ISO-DEP activate", nfc.CodeInvalidArgument, err.Error(), err)
	}
	cleanup = false
	return &Session{transport: transport, ATS: ats, fsc: ats.FSC}, nil
}

// Close leaves raw mode. The reader performs an RF power cycle so the next
// selection starts from a clean card state.
func (s *Session) Close(ctx context.Context) error {
	if s == nil || s.closed {
		return nil
	}
	s.closed = true
	if s.transport == nil {
		return nil
	}
	if ctx == nil {
		ctx = context.Background()
	}
	return s.transport.SetRawMode(context.WithoutCancel(ctx), false)
}

// Transmit sends one short command APDU and returns its status word and data.
func (s *Session) Transmit(ctx context.Context, apdu []byte) (Response, error) {
	if s == nil || s.transport == nil || s.closed {
		return Response{}, nfc.NewError("ISO-DEP transmit", nfc.CodeNotOpen, "ISO-DEP session is not active", nil)
	}
	if len(apdu) < 4 {
		return Response{}, nfc.NewError("ISO-DEP transmit", nfc.CodeInvalidArgument, "APDU header is incomplete", nil)
	}
	if s.fsc < 4 {
		return Response{}, nfc.NewError("ISO-DEP transmit", nfc.CodeInvalidArgument, "card frame size is too small", nil)
	}
	chunkSize := s.fsc - 3
	var responsePayload []byte
	for offset := 0; offset < len(apdu); {
		if err := ctx.Err(); err != nil {
			return Response{}, nfc.NewError("ISO-DEP transmit", nfc.CodeCanceled, "", err)
		}
		size := min(chunkSize, len(apdu)-offset)
		more := offset+size < len(apdu)
		pcb := byte(0x02) | s.pni
		if more {
			pcb |= 0x10
		}
		frame := append([]byte{pcb}, apdu[offset:offset+size]...)
		reply, err := s.exchange(ctx, frame)
		if err != nil {
			return Response{}, err
		}
		if more {
			if !isRACK(reply[0]) {
				return Response{}, nfc.NewError("ISO-DEP transmit", nfc.CodeIO, fmt.Sprintf("chained command was not acknowledged: PCB %02X", reply[0]), nil)
			}
			s.pni ^= 1
			offset += size
			continue
		}
		payload, err := s.receive(ctx, reply)
		if err != nil {
			return Response{}, err
		}
		responsePayload = payload
		offset += size
	}
	if len(responsePayload) < 2 {
		return Response{}, nfc.NewError("ISO-DEP transmit", nfc.CodeIO, "APDU response has no status word", nil)
	}
	return ParseResponse(responsePayload)
}

func (s *Session) receive(ctx context.Context, reply []byte) ([]byte, error) {
	var payload []byte
	for {
		if len(reply) == 0 {
			return nil, nfc.NewError("ISO-DEP receive", nfc.CodeIO, "empty ISO-DEP block", nil)
		}
		pcb := reply[0]
		switch {
		case pcb&0xC0 == 0x00:
			if pcb&0x01 != s.pni {
				return nil, nfc.NewError("ISO-DEP receive", nfc.CodeIO, fmt.Sprintf("unexpected block number in PCB %02X", pcb), nil)
			}
			payload = append(payload, reply[1:]...)
			if pcb&0x10 == 0 {
				s.pni ^= 1
				return payload, nil
			}
			ack := []byte{0xA2 | s.pni}
			s.pni ^= 1
			next, err := s.exchange(ctx, ack)
			if err != nil {
				return nil, err
			}
			reply = next
		case pcb&0xF0 == 0xF0:
			if len(reply) < 2 {
				return nil, nfc.NewError("ISO-DEP receive", nfc.CodeIO, "WTX block has no multiplier", nil)
			}
			next, err := s.exchange(ctx, []byte{pcb, reply[1]})
			if err != nil {
				return nil, err
			}
			reply = next
		case pcb&0xF0 == 0xA0:
			return nil, nfc.NewError("ISO-DEP receive", nfc.CodeIO, fmt.Sprintf("unexpected R(ACK) PCB %02X", pcb), nil)
		case pcb&0xF0 == 0xB0:
			return nil, nfc.NewError("ISO-DEP receive", nfc.CodeIO, fmt.Sprintf("card returned R(NAK) PCB %02X", pcb), nil)
		default:
			return nil, nfc.NewError("ISO-DEP receive", nfc.CodeIO, fmt.Sprintf("unsupported ISO-DEP PCB %02X", pcb), nil)
		}
	}
}

func (s *Session) exchange(ctx context.Context, frame []byte) ([]byte, error) {
	tx := iso14443a.WithCRCA(frame)
	response, err := s.transport.TransceiveBytes(ctx, tx)
	if err != nil {
		return nil, nfc.NewError("ISO-DEP exchange", nfc.CodeIO, "raw ISO-DEP frame failed", err)
	}
	if len(response) < 3 {
		return nil, nfc.NewError("ISO-DEP exchange", nfc.CodeIO, "ISO-DEP response is too short", nil)
	}
	body := response[:len(response)-2]
	crc := iso14443a.CRCA(body)
	if response[len(response)-2] != crc[0] || response[len(response)-1] != crc[1] {
		return nil, nfc.NewError("ISO-DEP exchange", nfc.CodeIO, "ISO-DEP response CRC does not match", nil)
	}
	return body, nil
}

func isRACK(pcb byte) bool { return pcb&0xF0 == 0xA0 }
