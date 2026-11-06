package type2

import (
	"encoding/binary"
	"fmt"
	"strings"
	"unicode/utf8"
)

const (
	wscCredential         = 0x100E
	wscAuthenticationType = 0x1003
	wscEncryptionType     = 0x100F
	wscMACAddress         = 0x1020
	wscNetworkIndex       = 0x1026
	wscNetworkKey         = 0x1027
	wscSSID               = 0x1045
	wscVendorExtension    = 0x1049
	wscAuthOpen           = 0x0001
	wscAuthWPAPersonal    = 0x0002
	wscAuthWPA2Personal   = 0x0020
	wscEncNone            = 0x0001
	wscEncWEP             = 0x0002
	wscEncTKIP            = 0x0004
	wscEncAES             = 0x0008
)

type wscAttribute struct {
	kind  uint16
	value []byte
}

// EncodeWiFiMessage builds a Wi-Fi Simple Configuration credential record.
// security is one of open, wpa, wpa2, or wpa2-wpa.
func EncodeWiFiMessage(ssid, password, security string) ([]byte, error) {
	if ssid == "" || len(ssid) > 32 {
		return nil, fmt.Errorf("%w: Wi-Fi SSID must contain 1-32 bytes", ErrMalformedNDEF)
	}
	if !utf8.ValidString(ssid) {
		return nil, fmt.Errorf("%w: Wi-Fi SSID must be UTF-8", ErrMalformedNDEF)
	}
	security = strings.ToLower(strings.TrimSpace(security))
	if security == "" {
		security = "wpa2"
	}
	var auth, encryption uint16
	switch security {
	case "open":
		auth, encryption = wscAuthOpen, wscEncNone
		if password != "" {
			return nil, fmt.Errorf("%w: an open network must not contain a password", ErrMalformedNDEF)
		}
	case "wpa":
		auth, encryption = wscAuthWPAPersonal, wscEncTKIP
	case "wpa2", "wpa2-wpa3":
		auth, encryption = wscAuthWPA2Personal, wscEncAES
	case "wpa2-wpa":
		auth, encryption = wscAuthWPAPersonal|wscAuthWPA2Personal, wscEncTKIP|wscEncAES
	default:
		return nil, fmt.Errorf("%w: unsupported Wi-Fi security %q", ErrMalformedNDEF, security)
	}
	if security != "open" {
		if len(password) < 8 || len(password) > 63 {
			return nil, fmt.Errorf("%w: WPA/WPA2 password must contain 8-63 characters", ErrMalformedNDEF)
		}
	}
	inner := make([]byte, 0, 128)
	appendTLV := func(kind uint16, value []byte) {
		inner = binary.BigEndian.AppendUint16(inner, kind)
		inner = binary.BigEndian.AppendUint16(inner, uint16(len(value)))
		inner = append(inner, value...)
	}
	appendTLV(wscNetworkIndex, []byte{1})
	appendTLV(wscSSID, []byte(ssid))
	appendTLV(wscAuthenticationType, []byte{byte(auth >> 8), byte(auth)})
	appendTLV(wscEncryptionType, []byte{byte(encryption >> 8), byte(encryption)})
	if security != "open" {
		appendTLV(wscNetworkKey, []byte(password))
	}
	appendTLV(wscMACAddress, []byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF})
	vendor := []byte{0x00, 0x37, 0x2A, 0x00, 0x01, 0x20, 0x02, 0x01, 0x01}
	appendTLV(wscVendorExtension, vendor)

	payload := make([]byte, 0, len(inner)+4)
	payload = binary.BigEndian.AppendUint16(payload, wscCredential)
	payload = binary.BigEndian.AppendUint16(payload, uint16(len(inner)))
	payload = append(payload, inner...)
	return encodeRecord(0x02, "application/vnd.wfa.wsc", payload)
}

func parseWSCAttributes(data []byte) ([]wscAttribute, error) {
	attributes := make([]wscAttribute, 0, 8)
	for offset := 0; offset < len(data); {
		if offset+4 > len(data) {
			return nil, fmt.Errorf("%w: Wi-Fi attribute header is truncated", ErrMalformedNDEF)
		}
		kind := binary.BigEndian.Uint16(data[offset : offset+2])
		length := int(binary.BigEndian.Uint16(data[offset+2 : offset+4]))
		offset += 4
		if length < 0 || offset+length > len(data) {
			return nil, fmt.Errorf("%w: Wi-Fi attribute exceeds payload", ErrMalformedNDEF)
		}
		attributes = append(attributes, wscAttribute{kind: kind, value: append([]byte(nil), data[offset:offset+length]...)})
		offset += length
	}
	return attributes, nil
}

func parseWiFiRecord(payload []byte) (string, []Field) {
	attributes, err := parseWSCAttributes(payload)
	if err != nil {
		return "Wi-Fi configuration (malformed)", nil
	}
	var credential []wscAttribute
	for _, attribute := range attributes {
		switch attribute.kind {
		case wscCredential:
			credential, _ = parseWSCAttributes(attribute.value)
		case wscSSID:
			credential = attributes
		}
	}
	if credential == nil {
		return "Wi-Fi configuration", nil
	}
	values := make(map[uint16][]byte)
	for _, attribute := range credential {
		values[attribute.kind] = attribute.value
	}
	ssid := printablePayload(values[wscSSID])
	fields := make([]Field, 0, 6)
	add := func(key, value string) {
		if value != "" {
			fields = append(fields, Field{Key: key, Value: value})
		}
	}
	add("ssid", ssid)
	if value := values[wscAuthenticationType]; len(value) == 2 {
		add("auth", wscAuthName(binary.BigEndian.Uint16(value)))
	}
	if value := values[wscEncryptionType]; len(value) == 2 {
		add("encryption", wscEncryptionName(binary.BigEndian.Uint16(value)))
	}
	if key := values[wscNetworkKey]; len(key) > 0 {
		add("network_key", "••••••")
	}
	if mac := values[wscMACAddress]; len(mac) == 6 {
		add("mac", formatMAC(mac))
	}
	if ssid == "" {
		ssid = "Wi-Fi configuration"
	}
	return ssid, fields
}

func wscAuthName(value uint16) string {
	names := make([]string, 0, 3)
	if value&wscAuthOpen != 0 {
		names = append(names, "Open")
	}
	if value&wscAuthWPAPersonal != 0 {
		names = append(names, "WPA-Personal")
	}
	if value&0x0004 != 0 {
		names = append(names, "Shared")
	}
	if value&0x0008 != 0 {
		names = append(names, "WPA-Enterprise")
	}
	if value&0x0010 != 0 {
		names = append(names, "WPA2-Enterprise")
	}
	if value&wscAuthWPA2Personal != 0 {
		names = append(names, "WPA2-Personal")
	}
	if len(names) == 0 {
		return fmt.Sprintf("0x%04X", value)
	}
	return strings.Join(names, " + ")
}

func wscEncryptionName(value uint16) string {
	names := make([]string, 0, 4)
	if value&wscEncNone != 0 {
		names = append(names, "None")
	}
	if value&wscEncWEP != 0 {
		names = append(names, "WEP")
	}
	if value&wscEncTKIP != 0 {
		names = append(names, "TKIP")
	}
	if value&wscEncAES != 0 {
		names = append(names, "AES")
	}
	if len(names) == 0 {
		return fmt.Sprintf("0x%04X", value)
	}
	return strings.Join(names, " + ")
}

func formatMAC(value []byte) string {
	parts := make([]string, len(value))
	for index, part := range value {
		parts[index] = fmt.Sprintf("%02X", part)
	}
	return strings.Join(parts, ":")
}
