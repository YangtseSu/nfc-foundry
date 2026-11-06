package type2

import (
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"strings"
	"unicode/utf8"
)

// EncodeBluetoothMessage builds a Bluetooth BR/EDR easy-pairing OOB record.
func EncodeBluetoothMessage(address, name string) ([]byte, error) {
	wireAddress, err := bluetoothAddressBytes(address)
	if err != nil {
		return nil, err
	}
	eir := make([]byte, 0, 32)
	if name != "" {
		if !utf8.ValidString(name) {
			return nil, fmt.Errorf("%w: Bluetooth name must be UTF-8", ErrMalformedNDEF)
		}
		if len(name) > 0xFE {
			return nil, fmt.Errorf("%w: Bluetooth name is too long", ErrMalformedNDEF)
		}
		eir = append(eir, byte(1+len(name)), 0x09)
		eir = append(eir, name...)
	}
	payload := make([]byte, 8+len(eir))
	binary.LittleEndian.PutUint16(payload[0:2], uint16(len(payload)))
	copy(payload[2:8], wireAddress)
	copy(payload[8:], eir)
	return encodeRecord(0x02, "application/vnd.bluetooth.ep.oob", payload)
}

func bluetoothAddressBytes(address string) ([]byte, error) {
	compact := strings.NewReplacer(":", "", "-", "", " ", "").Replace(strings.ToUpper(strings.TrimSpace(address)))
	decoded, err := hex.DecodeString(compact)
	if err != nil || len(decoded) != 6 {
		return nil, fmt.Errorf("%w: Bluetooth address must contain 6 hexadecimal octets", ErrMalformedNDEF)
	}
	reversed := make([]byte, 6)
	for index := range decoded {
		reversed[index] = decoded[len(decoded)-1-index]
	}
	return reversed, nil
}

func parseBluetoothRecord(payload []byte) (string, []Field) {
	if len(payload) < 8 {
		return "Bluetooth pairing (malformed)", nil
	}
	oobLength := int(binary.LittleEndian.Uint16(payload[0:2]))
	if oobLength < 8 || oobLength > len(payload) {
		return "Bluetooth pairing (malformed)", nil
	}
	address := formatBluetoothAddress(payload[2:8])
	name := ""
	class := ""
	offset := 8
	for offset < oobLength {
		length := int(payload[offset])
		offset++
		if length == 0 {
			break
		}
		if offset+length > oobLength || length < 1 {
			break
		}
		kind := payload[offset]
		value := payload[offset+1 : offset+length]
		switch kind {
		case 0x08, 0x09:
			if utf8.Valid(value) {
				name = string(value)
			}
		case 0x0D:
			if len(value) == 3 {
				class = fmt.Sprintf("0x%02X%02X%02X", value[2], value[1], value[0])
			}
		}
		offset += length
	}
	fields := []Field{{Key: "bt_address", Value: address}}
	if name != "" {
		fields = append(fields, Field{Key: "bt_name", Value: name})
	}
	if class != "" {
		fields = append(fields, Field{Key: "bt_class", Value: class})
	}
	summary := name
	if summary == "" {
		summary = address
	}
	return summary, fields
}

func formatBluetoothAddress(wire []byte) string {
	parts := make([]string, len(wire))
	for index := range wire {
		parts[index] = fmt.Sprintf("%02X", wire[len(wire)-1-index])
	}
	return strings.Join(parts, ":")
}
