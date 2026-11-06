package type2

import (
	"fmt"
	"net/url"
	"strings"
	"unicode/utf8"
)

const (
	KindText        = "text"
	KindURI         = "uri"
	KindURL         = "url"
	KindEmail       = "email"
	KindPhone       = "phone"
	KindSMS         = "sms"
	KindLocation    = "location"
	KindDPP         = "dpp"
	KindSmartPoster = "smartposter"
	KindVCard       = "vcard"
	KindWiFi        = "wifi"
	KindBluetooth   = "bluetooth"
	KindApp         = "app"
	KindMIME        = "mime"
	KindExternal    = "external"
	KindAbsoluteURI = "absolute-uri"
	KindUnknown     = "unknown"
)

func decodeRecord(record *Record, payload []byte) {
	switch record.TNF {
	case 0x00:
		record.Kind = KindUnknown
		record.Summary = "Empty record"
	case 0x01:
		switch record.Type {
		case "T":
			record.Kind = KindText
			record.Text = decodeTextPayload(payload)
			record.Summary = record.Text
		case "U":
			record.URI = decodeURIPayload(payload)
			decodeURI(record)
		case "Sp":
			decodeSmartPoster(record, payload)
		default:
			record.Kind = KindUnknown
			record.Summary = "Well-known record " + record.Type
		}
	case 0x02:
		decodeMIME(record, payload)
	case 0x03:
		record.Kind = KindAbsoluteURI
		record.URI = record.Type
		record.Summary = record.Type
		if preview := printablePayload(payload); preview != "" {
			record.Fields = append(record.Fields, Field{Key: "payload", Value: preview})
		}
	case 0x04:
		decodeExternal(record, payload)
	default:
		record.Kind = KindUnknown
		record.Summary = fmt.Sprintf("TNF %d record", record.TNF)
	}
	if record.Summary == "" {
		record.Summary = record.Type
	}
}

func decodeSmartPoster(record *Record, payload []byte) {
	record.Kind = KindSmartPoster
	nested, err := ParseMessage(payload)
	if err != nil {
		record.Summary = "Smart Poster (malformed)"
		return
	}
	title := ""
	for _, child := range nested {
		if record.URI == "" && child.URI != "" {
			record.URI = child.URI
		}
		if title == "" && child.Text != "" {
			title = child.Text
		}
	}
	if title != "" {
		record.Text = title
		record.Fields = append(record.Fields, Field{Key: "title", Value: title})
	}
	if record.URI != "" {
		record.Fields = append(record.Fields, Field{Key: "uri", Value: record.URI})
	}
	switch {
	case title != "":
		record.Summary = title
	case record.URI != "":
		record.Summary = record.URI
	default:
		record.Summary = "Smart Poster"
	}
}

func decodeMIME(record *Record, payload []byte) {
	mimeType := strings.ToLower(strings.TrimSpace(strings.Split(record.Type, ";")[0]))
	record.Kind = KindMIME
	record.Summary = mimeType
	switch mimeType {
	case "text/vcard", "text/x-vcard":
		name, fields := parseVCard(payload)
		record.Kind = KindVCard
		record.Fields = fields
		if name != "" {
			record.Summary = name
		} else {
			record.Summary = "Contact"
		}
	case "text/plain":
		if text := printablePayload(payload); text != "" {
			record.Kind = KindText
			record.Text = text
			record.Summary = text
		}
	case "application/json":
		record.Kind = "json"
		record.Summary = "JSON document"
		if text := printablePayload(payload); text != "" {
			record.Text = text
		}
	case "application/vnd.wfa.wsc":
		summary, fields := parseWiFiRecord(payload)
		record.Kind = KindWiFi
		record.Fields = fields
		if summary != "" {
			record.Summary = summary
		} else {
			record.Summary = "Wi-Fi configuration"
		}
	case "application/vnd.bluetooth.ep.oob":
		summary, fields := parseBluetoothRecord(payload)
		record.Kind = KindBluetooth
		record.Fields = fields
		if summary != "" {
			record.Summary = summary
		} else {
			record.Summary = "Bluetooth pairing"
		}
	default:
		if text := printablePayload(payload); text != "" {
			record.Text = text
			record.Summary = text
			record.Fields = append(record.Fields, Field{Key: "text", Value: text})
		}
	}
}

func decodeExternal(record *Record, payload []byte) {
	record.Kind = KindExternal
	record.Summary = record.Type
	if strings.EqualFold(record.Type, "android.com:pkg") {
		packageName := printablePayload(payload)
		record.Kind = KindApp
		if packageName != "" {
			record.Summary = packageName
			record.Fields = append(record.Fields, Field{Key: "package", Value: packageName})
		}
		return
	}
	if text := printablePayload(payload); text != "" {
		record.Fields = append(record.Fields, Field{Key: "payload", Value: text})
	}
}

func printablePayload(payload []byte) string {
	if len(payload) == 0 || !utf8.Valid(payload) {
		return ""
	}
	for _, value := range payload {
		if value < 0x20 && value != '\n' && value != '\r' && value != '\t' {
			return ""
		}
	}
	return strings.TrimSpace(string(payload))
}

func decodeURI(record *Record) {
	uri := record.URI
	record.Kind = KindURI
	record.Summary = uri
	lower := strings.ToLower(uri)
	switch {
	case strings.HasPrefix(lower, "dpp:"):
		decodeDPP(record)
	case strings.HasPrefix(lower, "http://"), strings.HasPrefix(lower, "https://"):
		record.Kind = KindURL
	case strings.HasPrefix(lower, "mailto:"):
		record.Kind = KindEmail
		address, query := splitURIQuery(strings.TrimPrefix(uri, "mailto:"))
		record.Fields = append(record.Fields, Field{Key: "to", Value: address})
		if values, err := url.ParseQuery(query); err == nil {
			if subject := values.Get("subject"); subject != "" {
				record.Fields = append(record.Fields, Field{Key: "subject", Value: subject})
			}
			if body := values.Get("body"); body != "" {
				record.Fields = append(record.Fields, Field{Key: "body", Value: body})
			}
		}
		if address != "" {
			record.Summary = address
		}
	case strings.HasPrefix(lower, "tel:"):
		record.Kind = KindPhone
		number := strings.TrimPrefix(uri, "tel:")
		record.Fields = append(record.Fields, Field{Key: "phone", Value: number})
		record.Summary = number
	case strings.HasPrefix(lower, "sms:"):
		record.Kind = KindSMS
		number, query := splitURIQuery(strings.TrimPrefix(uri, "sms:"))
		record.Fields = append(record.Fields, Field{Key: "number", Value: number})
		if values, err := url.ParseQuery(query); err == nil {
			if body := values.Get("body"); body != "" {
				record.Fields = append(record.Fields, Field{Key: "body", Value: body})
			}
		}
		if number != "" {
			record.Summary = number
		}
	case strings.HasPrefix(lower, "geo:"):
		record.Kind = KindLocation
		coordinates, query := splitURIQuery(strings.TrimPrefix(uri, "geo:"))
		parts := strings.SplitN(coordinates, ",", 2)
		if len(parts) == 2 {
			record.Fields = append(record.Fields,
				Field{Key: "latitude", Value: parts[0]},
				Field{Key: "longitude", Value: parts[1]})
		}
		if values, err := url.ParseQuery(query); err == nil {
			if label := values.Get("q"); label != "" {
				record.Fields = append(record.Fields, Field{Key: "label", Value: label})
				record.Summary = label
			}
		}
	}
}

func splitURIQuery(value string) (string, string) {
	head, tail, found := strings.Cut(value, "?")
	if !found {
		return value, ""
	}
	return head, tail
}

func decodeDPP(record *Record) {
	record.Kind = KindDPP
	record.Summary = "DPP bootstrapping"
	body := strings.TrimPrefix(record.URI, "DPP:")
	mac := ""
	for _, token := range strings.Split(body, ";") {
		if token == "" {
			continue
		}
		key, value, ok := strings.Cut(token, ":")
		if !ok || value == "" {
			continue
		}
		switch key {
		case "M":
			mac = value
			record.Fields = append(record.Fields, Field{Key: "dpp_mac", Value: value})
		case "V":
			record.Fields = append(record.Fields, Field{Key: "dpp_version", Value: value})
		case "C":
			record.Fields = append(record.Fields, Field{Key: "dpp_channels", Value: value})
		case "K":
			record.Fields = append(record.Fields, Field{Key: "dpp_key", Value: fmt.Sprintf("present (%d characters)", len(value))})
		case "I":
			record.Fields = append(record.Fields, Field{Key: "dpp_info", Value: value})
		}
	}
	record.Fields = append(record.Fields, Field{Key: "uri", Value: record.URI})
	if mac != "" {
		record.Summary = mac
	}
}
