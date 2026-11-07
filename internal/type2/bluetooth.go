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
	payload, err := bluetoothPayload(address, name)
	if err != nil {
		return nil, err
	}
	return encodeRecord(0x02, "application/vnd.bluetooth.ep.oob", payload)
}

func bluetoothPayload(address, name string) ([]byte, error) {
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
	return payload, nil
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

// EncodeBluetoothLEMessage builds a Bluetooth LE OOB record. addressType is
// "public" or "random"; role is "", "peripheral", "central", or "both".
func EncodeBluetoothLEMessage(address, addressType, name, role string) ([]byte, error) {
	payload, err := bluetoothLEPayload(address, addressType, name, role)
	if err != nil {
		return nil, err
	}
	return encodeRecord(0x02, "application/vnd.bluetooth.le.oob", payload)
}

func bluetoothLEPayload(address, addressType, name, role string) ([]byte, error) {
	wireAddress, err := bluetoothAddressBytes(address)
	if err != nil {
		return nil, err
	}
	addressType = strings.ToLower(strings.TrimSpace(addressType))
	if addressType == "" {
		addressType = "public"
	}
	var typeCode byte
	switch addressType {
	case "public":
		typeCode = 0x00
	case "random":
		typeCode = 0x01
	default:
		return nil, fmt.Errorf("%w: Bluetooth LE address type must be public or random", ErrMalformedNDEF)
	}
	payload := []byte{0x08, 0x1B}
	payload = append(payload, wireAddress...)
	payload = append(payload, typeCode)
	if name != "" {
		if !utf8.ValidString(name) {
			return nil, fmt.Errorf("%w: Bluetooth LE name must be UTF-8", ErrMalformedNDEF)
		}
		if len(name) > 0xFD {
			return nil, fmt.Errorf("%w: Bluetooth LE name is too long", ErrMalformedNDEF)
		}
		payload = append(payload, byte(1+len(name)), 0x09)
		payload = append(payload, name...)
	}
	role = strings.ToLower(strings.TrimSpace(role))
	if role != "" {
		var roleCode byte
		switch role {
		case "peripheral":
			roleCode = 0x00
		case "central":
			roleCode = 0x01
		case "both":
			roleCode = 0x02
		default:
			return nil, fmt.Errorf("%w: Bluetooth LE role must be peripheral, central, or both", ErrMalformedNDEF)
		}
		payload = append(payload, 0x02, 0x1C, roleCode)
	}
	return payload, nil
}

func parseBluetoothLERecord(payload []byte) (string, []Field) {
	address := ""
	addressType := ""
	name := ""
	role := ""
	for offset := 0; offset < len(payload); {
		length := int(payload[offset])
		offset++
		if length == 0 {
			break
		}
		if offset+length > len(payload) || length < 1 {
			return "Bluetooth LE pairing (malformed)", nil
		}
		kind := payload[offset]
		value := payload[offset+1 : offset+length]
		switch kind {
		case 0x1B:
			if len(value) == 7 {
				address = formatBluetoothAddress(value[:6])
				switch value[6] {
				case 0x00:
					addressType = "public"
				case 0x01:
					addressType = "random"
				default:
					addressType = fmt.Sprintf("0x%02X", value[6])
				}
			}
		case 0x09, 0x08:
			if utf8.Valid(value) {
				name = string(value)
			}
		case 0x1C:
			if len(value) == 1 {
				switch value[0] {
				case 0x00:
					role = "peripheral"
				case 0x01:
					role = "central"
				case 0x02:
					role = "both"
				default:
					role = fmt.Sprintf("0x%02X", value[0])
				}
			}
		}
		offset += length
	}
	fields := make([]Field, 0, 4)
	if address != "" {
		fields = append(fields, Field{Key: "bt_address", Value: address})
	}
	if addressType != "" {
		fields = append(fields, Field{Key: "bt_address_type", Value: addressType})
	}
	if name != "" {
		fields = append(fields, Field{Key: "bt_name", Value: name})
	}
	if role != "" {
		fields = append(fields, Field{Key: "bt_role", Value: role})
	}
	summary := name
	if summary == "" {
		summary = address
	}
	if summary == "" {
		summary = "Bluetooth LE pairing"
	}
	return summary, fields
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
