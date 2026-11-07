package isodep

import "fmt"

// Response is one APDU response. A non-0x9000 status word is a card answer,
// not a transport error.
type Response struct {
	Data []byte
	SW1  byte
	SW2  byte
}

// SW returns the status word as a big-endian value.
func (r Response) SW() uint16 { return uint16(r.SW1)<<8 | uint16(r.SW2) }

// OK reports whether the status word is 0x9000.
func (r Response) OK() bool { return r.SW1 == 0x90 && r.SW2 == 0x00 }

// BuildShort builds a short ISO/IEC 7816-4 command APDU. le is the expected
// response length; 0 means no Le field and 256 is encoded as 0x00. Data must
// fit the one-byte Lc field.
func BuildShort(cla, ins, p1, p2 byte, data []byte, le int) ([]byte, error) {
	if len(data) > 0xFF {
		return nil, fmt.Errorf("APDU command data is %d bytes; short APDU limit is 255", len(data))
	}
	if le < 0 || le > 256 {
		return nil, fmt.Errorf("APDU Le %d is outside 0..256", le)
	}
	apdu := make([]byte, 0, 4+1+len(data)+1)
	apdu = append(apdu, cla, ins, p1, p2)
	if len(data) != 0 {
		apdu = append(apdu, byte(len(data)))
		apdu = append(apdu, data...)
	}
	if le != 0 {
		if le == 256 {
			apdu = append(apdu, 0x00)
		} else {
			apdu = append(apdu, byte(le))
		}
	}
	return apdu, nil
}

// ParseResponse splits response data from the status word.
func ParseResponse(payload []byte) (Response, error) {
	if len(payload) < 2 {
		return Response{}, fmt.Errorf("APDU response is %d bytes; status word is missing", len(payload))
	}
	return Response{
		Data: append([]byte(nil), payload[:len(payload)-2]...),
		SW1:  payload[len(payload)-2],
		SW2:  payload[len(payload)-1],
	}, nil
}

// RetryLe returns the Le value indicated by a 6C XX status word.
func RetryLe(response Response) (int, bool) {
	if response.SW1 != 0x6C {
		return 0, false
	}
	if response.SW2 == 0 {
		return 256, true
	}
	return int(response.SW2), true
}

// GetResponse builds a GET RESPONSE APDU for a 61 XX status word.
func GetResponse(response Response) ([]byte, bool) {
	if response.SW1 != 0x61 {
		return nil, false
	}
	le := int(response.SW2)
	if le == 0 {
		le = 256
	}
	apdu, _ := BuildShort(0x00, 0xC0, 0x00, 0x00, nil, le)
	return apdu, true
}
