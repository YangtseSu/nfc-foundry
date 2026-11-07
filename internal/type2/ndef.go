package type2

import (
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"strings"
	"unicode/utf16"
	"unicode/utf8"
)

const (
	tlvNull          byte = 0x00
	tlvLockControl   byte = 0x01
	tlvMemoryControl byte = 0x02
	tlvNDEF          byte = 0x03
	tlvTerminator    byte = 0xFE
)

// Field is one decoded, human-readable record attribute. Sensitive marks a
// credential that the GUI masks until the user reveals it.
type Field struct {
	Key       string `json:"key"`
	Value     string `json:"value"`
	Sensitive bool   `json:"sensitive,omitempty"`
}

// Record is one validated NDEF record.
type Record struct {
	TNF     byte    `json:"tnf"`
	Type    string  `json:"type"`
	ID      string  `json:"id,omitempty"`
	Payload string  `json:"payload"`
	Text    string  `json:"text,omitempty"`
	URI     string  `json:"uri,omitempty"`
	Kind    string  `json:"kind"`
	Summary string  `json:"summary,omitempty"`
	Fields  []Field `json:"fields,omitempty"`
}

// NDEF is a parsed NDEF TLV.
type NDEF struct {
	Present bool     `json:"present"`
	Length  int      `json:"length"`
	Raw     string   `json:"raw"`
	Records []Record `json:"records"`
	Text    string   `json:"text,omitempty"`
	URI     string   `json:"uri,omitempty"`
	Summary string   `json:"summary,omitempty"`
}

type tlv struct {
	kind   byte
	value  []byte
	offset int
}

func parseTLVs(area []byte) ([]tlv, error) {
	var tlvs []tlv
	for offset := 0; offset < len(area); {
		kind := area[offset]
		if kind == tlvNull {
			offset++
			continue
		}
		if kind == tlvTerminator {
			return tlvs, nil
		}
		offset++
		if offset >= len(area) {
			return nil, fmt.Errorf("%w: TLV 0x%02X has no length", ErrMalformedTLV, kind)
		}
		length := int(area[offset])
		offset++
		if length == 0xFF {
			if offset+2 > len(area) {
				return nil, fmt.Errorf("%w: TLV 0x%02X has a truncated long length", ErrMalformedTLV, kind)
			}
			length = int(binary.BigEndian.Uint16(area[offset : offset+2]))
			offset += 2
		}
		if offset+length > len(area) {
			return nil, fmt.Errorf("%w: TLV 0x%02X value exceeds the NDEF area", ErrMalformedTLV, kind)
		}
		tlvs = append(tlvs, tlv{kind: kind, value: append([]byte(nil), area[offset:offset+length]...), offset: offset})
		offset += length
	}
	return tlvs, nil
}

// ParseNDEF finds and validates the NDEF TLV in an NDEF area.
func ParseNDEF(area []byte) (*NDEF, error) {
	tlvs, err := parseTLVs(area)
	if err != nil {
		return nil, err
	}
	var message []byte
	found := false
	for _, entry := range tlvs {
		if entry.kind != tlvNDEF {
			continue
		}
		if found {
			return nil, fmt.Errorf("%w: more than one NDEF TLV", ErrMalformedTLV)
		}
		found = true
		message = entry.value
	}
	if !found {
		return &NDEF{Present: false}, nil
	}
	if len(message) == 0 {
		return nil, fmt.Errorf("%w: zero-length NDEF TLV", ErrMalformedNDEF)
	}
	records, err := ParseMessage(message)
	if err != nil {
		return nil, err
	}
	result := &NDEF{
		Present: true, Length: len(message),
		Raw: strings.ToUpper(hex.EncodeToString(message)), Records: records,
	}
	for _, record := range records {
		if result.Text == "" && record.Text != "" {
			result.Text = record.Text
		}
		if result.URI == "" && record.URI != "" {
			result.URI = record.URI
		}
		if result.Summary == "" && record.Summary != "" {
			result.Summary = record.Summary
		}
	}
	return result, nil
}

// ParseMessage validates the NDEF record layout and returns the decoded
// records. It does not accept trailing bytes after the ME record.
func ParseMessage(message []byte) ([]Record, error) {
	if len(message) == 0 {
		return nil, fmt.Errorf("%w: empty message", ErrMalformedNDEF)
	}
	var records []Record
	offset := 0
	first := true
	for {
		if offset >= len(message) {
			return nil, fmt.Errorf("%w: message ends before the ME record", ErrMalformedNDEF)
		}
		header := message[offset]
		offset++
		mb := header&0x80 != 0
		me := header&0x40 != 0
		cf := header&0x20 != 0
		short := header&0x10 != 0
		hasID := header&0x08 != 0
		tnf := header & 0x07
		if first && !mb {
			return nil, fmt.Errorf("%w: first record is missing MB", ErrMalformedNDEF)
		}
		if !first && mb {
			return nil, fmt.Errorf("%w: only the first record may set MB", ErrMalformedNDEF)
		}
		if cf {
			return nil, fmt.Errorf("%w: chunked records are not supported", ErrMalformedNDEF)
		}
		if tnf == 0x07 {
			return nil, fmt.Errorf("%w: reserved TNF 0x07", ErrMalformedNDEF)
		}
		var typeLength int
		if offset >= len(message) {
			return nil, fmt.Errorf("%w: record header is truncated", ErrMalformedNDEF)
		}
		typeLength = int(message[offset])
		offset++
		payloadLength := 0
		if short {
			if offset >= len(message) {
				return nil, fmt.Errorf("%w: short payload length is truncated", ErrMalformedNDEF)
			}
			payloadLength = int(message[offset])
			offset++
		} else {
			if offset+4 > len(message) {
				return nil, fmt.Errorf("%w: payload length is truncated", ErrMalformedNDEF)
			}
			payloadLength = int(binary.BigEndian.Uint32(message[offset : offset+4]))
			offset += 4
		}
		idLength := 0
		if hasID {
			if offset >= len(message) {
				return nil, fmt.Errorf("%w: ID length is truncated", ErrMalformedNDEF)
			}
			idLength = int(message[offset])
			offset++
		}
		if payloadLength < 0 || typeLength+idLength+payloadLength > len(message)-offset {
			return nil, fmt.Errorf("%w: record %d exceeds the message", ErrMalformedNDEF, len(records))
		}
		if tnf == 0x00 && (typeLength != 0 || payloadLength != 0 || idLength != 0) {
			return nil, fmt.Errorf("%w: empty TNF record has content", ErrMalformedNDEF)
		}
		typeBytes := message[offset : offset+typeLength]
		offset += typeLength
		idBytes := message[offset : offset+idLength]
		offset += idLength
		payload := message[offset : offset+payloadLength]
		offset += payloadLength
		record := Record{
			TNF: tnf, Type: string(typeBytes), ID: strings.ToUpper(hex.EncodeToString(idBytes)),
			Payload: strings.ToUpper(hex.EncodeToString(payload)),
		}
		decodeRecord(&record, payload)
		records = append(records, record)
		first = false
		if me {
			if offset != len(message) {
				return nil, fmt.Errorf("%w: %d trailing bytes after the ME record", ErrMalformedNDEF, len(message)-offset)
			}
			break
		}
	}
	if len(records) == 0 {
		return nil, fmt.Errorf("%w: message has no records", ErrMalformedNDEF)
	}
	return records, nil
}

func decodeTextPayload(payload []byte) string {
	if len(payload) == 0 {
		return ""
	}
	status := payload[0]
	languageLength := int(status & 0x3F)
	if 1+languageLength > len(payload) {
		return ""
	}
	text := payload[1+languageLength:]
	if status&0x80 != 0 {
		if len(text)%2 != 0 {
			return ""
		}
		units := make([]uint16, len(text)/2)
		for index := range units {
			units[index] = binary.BigEndian.Uint16(text[index*2 : index*2+2])
		}
		return string(utf16.Decode(units))
	}
	if !utf8.Valid(text) {
		return ""
	}
	return string(text)
}

var uriPrefixes = []string{
	"", "http://www.", "https://www.", "http://", "https://",
	"tel:", "mailto:", "ftp://anonymous:anonymous@", "ftp://ftp.", "ftps://",
	"sftp://", "smb://", "nfs://", "ftp://", "dav://",
	"news:", "telnet://", "imap:", "rtsp://", "urn:",
	"pop:", "sip:", "sips:", "tftp:", "btspp://",
	"btl2cap://", "btgoep://", "tcpobex://", "irdaobex://", "file://",
	"urn:epc:id:", "urn:epc:tag:", "urn:epc:pat:", "urn:epc:raw:", "urn:epc:",
	"urn:nfc:",
}

func decodeURIPayload(payload []byte) string {
	if len(payload) == 0 {
		return ""
	}
	prefix := ""
	if int(payload[0]) < len(uriPrefixes) {
		prefix = uriPrefixes[payload[0]]
	}
	body := payload[1:]
	if !utf8.Valid(body) {
		return ""
	}
	return prefix + string(body)
}

// BuildNDEFArea returns the complete NDEF area for a validated message.
// It includes the NDEF TLV, a terminator, and zero padding to capacity.
func BuildNDEFArea(capacity int, message []byte) ([]byte, error) {
	if capacity <= 0 {
		return nil, fmt.Errorf("%w: NDEF capacity is zero", ErrCapacity)
	}
	if _, err := ParseMessage(message); err != nil {
		return nil, err
	}
	overhead := 2
	long := len(message) > 0xFE
	if long {
		overhead = 4
	}
	if len(message) > 0xFFFF || overhead+len(message)+1 > capacity {
		return nil, fmt.Errorf("%w: message needs %d bytes; capacity is %d", ErrCapacity, overhead+len(message)+1, capacity)
	}
	area := make([]byte, capacity)
	area[0] = tlvNDEF
	position := 1
	if long {
		area[1] = 0xFF
		binary.BigEndian.PutUint16(area[2:4], uint16(len(message)))
		position = 4
	} else {
		area[1] = byte(len(message))
		position = 2
	}
	position += copy(area[position:], message)
	if position < capacity {
		area[position] = tlvTerminator
	}
	return area, nil
}
