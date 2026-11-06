package type2

import (
	"fmt"
	"net/url"
	"strconv"
	"strings"
)

// TemplateField describes one field of a common NDEF template. Kind is "text"
// or "select"; Options contains the stable values for a select field.
type TemplateField struct {
	ID       string
	Kind     string
	Required bool
	Options  []string
}

// Template is a common NDEF message shape.
type Template struct {
	ID     string
	Fields []TemplateField
}

var ndefTemplates = []Template{
	{ID: "text", Fields: []TemplateField{{ID: "text", Required: true}}},
	{ID: "uri", Fields: []TemplateField{{ID: "uri", Required: true}}},
	{ID: "custom", Fields: []TemplateField{{ID: "uri", Required: true}}},
	{ID: "search", Fields: []TemplateField{
		{ID: "search_prefix", Required: true},
		{ID: "query", Required: true},
	}},
	{ID: "payment", Fields: []TemplateField{{ID: "uri", Required: true}}},
	{ID: "social", Fields: []TemplateField{{ID: "uri", Required: true}}},
	{ID: "app", Fields: []TemplateField{{ID: "package", Required: true}}},
	{ID: "email", Fields: []TemplateField{
		{ID: "to", Required: true},
		{ID: "subject"},
		{ID: "body"},
	}},
	{ID: "contact", Fields: []TemplateField{
		{ID: "name", Required: true},
		{ID: "organization"},
		{ID: "job_title"},
		{ID: "phone"},
		{ID: "email"},
		{ID: "address"},
		{ID: "url"},
		{ID: "note"},
	}},
	{ID: "phone", Fields: []TemplateField{{ID: "phone", Required: true}}},
	{ID: "sms", Fields: []TemplateField{
		{ID: "number", Required: true},
		{ID: "body"},
	}},
	{ID: "location", Fields: []TemplateField{
		{ID: "latitude", Required: true},
		{ID: "longitude", Required: true},
		{ID: "label"},
	}},
	{ID: "address", Fields: []TemplateField{{ID: "address", Required: true}}},
	{ID: "bluetooth", Fields: []TemplateField{
		{ID: "address", Required: true},
		{ID: "name"},
	}},
	{ID: "wifi", Fields: []TemplateField{
		{ID: "ssid", Required: true},
		{ID: "security", Kind: "select", Required: true, Options: []string{"wpa2", "wpa2-wpa3", "wpa", "wpa2-wpa", "open"}},
		{ID: "password"},
	}},
	{ID: "dpp", Fields: []TemplateField{{ID: "dpp_uri", Required: true}}},
	{ID: "smartposter", Fields: []TemplateField{
		{ID: "uri", Required: true},
		{ID: "job_title"},
	}},
}

// Templates returns the supported common NDEF templates.
func Templates() []Template {
	result := make([]Template, len(ndefTemplates))
	for index, template := range ndefTemplates {
		fields := make([]TemplateField, len(template.Fields))
		copy(fields, template.Fields)
		for fieldIndex := range fields {
			fields[fieldIndex].Options = append([]string(nil), fields[fieldIndex].Options...)
		}
		result[index] = Template{ID: template.ID, Fields: fields}
	}
	return result
}

// EncodeTemplate builds the NDEF message for a common template.
func EncodeTemplate(template string, fields map[string]string) ([]byte, error) {
	value := func(id string) string { return strings.TrimSpace(fields[id]) }
	required := func(id string) (string, error) {
		result := value(id)
		if result == "" {
			return "", fmt.Errorf("%w: template %s requires field %s", ErrMalformedNDEF, template, id)
		}
		return result, nil
	}
	switch template {
	case "text":
		text := fields["text"]
		if strings.TrimSpace(text) == "" {
			return nil, fmt.Errorf("%w: text is required", ErrMalformedNDEF)
		}
		return EncodeTextMessage(text, "en")
	case "uri", "custom", "payment", "social":
		uri, err := required("uri")
		if err != nil {
			return nil, err
		}
		return EncodeURIMessage(uri)
	case "search":
		prefix, err := required("search_prefix")
		if err != nil {
			return nil, err
		}
		query, err := required("query")
		if err != nil {
			return nil, err
		}
		if !strings.HasPrefix(prefix, "http://") && !strings.HasPrefix(prefix, "https://") {
			return nil, fmt.Errorf("%w: search prefix must be an HTTP(S) URL", ErrMalformedNDEF)
		}
		if !strings.Contains(prefix, "?") {
			prefix = strings.TrimRight(prefix, "/") + "/?q="
		}
		return EncodeURIMessage(prefix + url.QueryEscape(query))
	case "app":
		packageName, err := required("package")
		if err != nil {
			return nil, err
		}
		return EncodeExternalMessage("android.com:pkg", []byte(packageName))
	case "email":
		to, err := required("to")
		if err != nil {
			return nil, err
		}
		uri := "mailto:" + to
		query := url.Values{}
		if subject := value("subject"); subject != "" {
			query.Set("subject", subject)
		}
		if body := value("body"); body != "" {
			query.Set("body", body)
		}
		if encoded := query.Encode(); encoded != "" {
			uri += "?" + encoded
		}
		return EncodeURIMessage(uri)
	case "contact":
		name, err := required("name")
		if err != nil {
			return nil, err
		}
		contact := make(map[string]string, len(fields))
		for key, item := range fields {
			contact[key] = item
		}
		contact["name"] = name
		return EncodeVCardMessage(contact)
	case "phone":
		phone, err := required("phone")
		if err != nil {
			return nil, err
		}
		return EncodeURIMessage("tel:" + sanitizePhone(phone))
	case "sms":
		number, err := required("number")
		if err != nil {
			return nil, err
		}
		uri := "sms:" + sanitizePhone(number)
		if body := value("body"); body != "" {
			uri += "?body=" + url.QueryEscape(body)
		}
		return EncodeURIMessage(uri)
	case "location":
		latitude, err := required("latitude")
		if err != nil {
			return nil, err
		}
		longitude, err := required("longitude")
		if err != nil {
			return nil, err
		}
		lat, err := strconv.ParseFloat(latitude, 64)
		if err != nil || lat < -90 || lat > 90 {
			return nil, fmt.Errorf("%w: latitude must be between -90 and 90", ErrMalformedNDEF)
		}
		lon, err := strconv.ParseFloat(longitude, 64)
		if err != nil || lon < -180 || lon > 180 {
			return nil, fmt.Errorf("%w: longitude must be between -180 and 180", ErrMalformedNDEF)
		}
		uri := fmt.Sprintf("geo:%s,%s", strconv.FormatFloat(lat, 'f', -1, 64), strconv.FormatFloat(lon, 'f', -1, 64))
		if label := value("label"); label != "" {
			uri += "?q=" + url.QueryEscape(label)
		}
		return EncodeURIMessage(uri)
	case "address":
		address, err := required("address")
		if err != nil {
			return nil, err
		}
		return EncodeURIMessage("geo:0,0?q=" + url.QueryEscape(address))
	case "bluetooth":
		address, err := required("address")
		if err != nil {
			return nil, err
		}
		return EncodeBluetoothMessage(address, value("name"))
	case "wifi":
		ssid, err := required("ssid")
		if err != nil {
			return nil, err
		}
		security := value("security")
		if security == "" {
			security = "wpa2"
		}
		return EncodeWiFiMessage(ssid, fields["password"], security)
	case "dpp":
		uri, err := required("dpp_uri")
		if err != nil {
			return nil, err
		}
		if !strings.HasPrefix(uri, "DPP:") {
			return nil, fmt.Errorf("%w: DPP bootstrapping URI must start with DPP:", ErrMalformedNDEF)
		}
		if !strings.Contains(uri, "K:") {
			return nil, fmt.Errorf("%w: DPP bootstrapping URI must contain a public key", ErrMalformedNDEF)
		}
		return EncodeURIMessage(uri)
	case "smartposter":
		uri, err := required("uri")
		if err != nil {
			return nil, err
		}
		return EncodeSmartPosterMessage(uri, value("title"))
	default:
		return nil, fmt.Errorf("%w: unknown NDEF template %q", ErrMalformedNDEF, template)
	}
}

func sanitizePhone(value string) string {
	var result strings.Builder
	for _, char := range value {
		if char >= '0' && char <= '9' || char == '+' || char == '*' || char == '#' || char == ';' {
			result.WriteRune(char)
		}
	}
	return result.String()
}
