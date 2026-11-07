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

// Provider is a stable provider preset shared by the GUI and CLI.
type Provider struct {
	ID  string
	URL string
}

// LanguageOptions are the language tags offered by the text editor.
func LanguageOptions() []string {
	return []string{"en", "zh-Hans", "zh-Hant", "ja", "ko", "de", "fr", "es", "it", "pt", "ru", "ar"}
}

// SearchProviders returns the search URL presets.
func SearchProviders() []Provider {
	return []Provider{
		{ID: "duckduckgo", URL: "https://duckduckgo.com/?q="},
		{ID: "google", URL: "https://www.google.com/search?q="},
		{ID: "bing", URL: "https://www.bing.com/search?q="},
		{ID: "brave", URL: "https://search.brave.com/search?q="},
		{ID: "startpage", URL: "https://www.startpage.com/sp/search?query="},
		{ID: "wikipedia", URL: "https://en.wikipedia.org/w/index.php?search="},
	}
}

// SocialProviders returns social profile URL prefixes.
func SocialProviders() []Provider {
	return []Provider{
		{ID: "github", URL: "https://github.com/"},
		{ID: "x", URL: "https://x.com/"},
		{ID: "mastodon", URL: "https://mastodon.social/@"},
		{ID: "linkedin", URL: "https://www.linkedin.com/in/"},
		{ID: "instagram", URL: "https://instagram.com/"},
		{ID: "youtube", URL: "https://youtube.com/@"},
		{ID: "facebook", URL: "https://facebook.com/"},
		{ID: "telegram", URL: "https://t.me/"},
	}
}

// PaymentProviders returns payment link URL prefixes.
func PaymentProviders() []Provider {
	return []Provider{
		{ID: "paypal", URL: "https://paypal.me/"},
		{ID: "venmo", URL: "https://venmo.com/u/"},
		{ID: "cashapp", URL: "https://cash.app/$"},
		{ID: "revolut", URL: "https://revolut.me/"},
		{ID: "kofi", URL: "https://ko-fi.com/"},
		{ID: "buymeacoffee", URL: "https://buymeacoffee.com/"},
	}
}

func providerURL(providers []Provider, id string) string {
	for _, provider := range providers {
		if provider.ID == id {
			return provider.URL
		}
	}
	return ""
}

var ndefTemplates = []Template{
	{ID: "text", Fields: []TemplateField{
		{ID: "text", Required: true},
		{ID: "language", Kind: "select", Options: LanguageOptions()},
	}},
	{ID: "uri", Fields: []TemplateField{{ID: "uri", Required: true}}},
	{ID: "custom", Fields: []TemplateField{{ID: "uri", Required: true}}},
	{ID: "search", Fields: []TemplateField{
		{ID: "search_provider", Kind: "select", Options: providerIDs(SearchProviders())},
		{ID: "search_prefix"},
		{ID: "query", Required: true},
	}},
	{ID: "payment", Fields: []TemplateField{
		{ID: "payment_provider", Kind: "select", Options: providerIDs(PaymentProviders())},
		{ID: "handle"},
		{ID: "uri"},
	}},
	{ID: "social", Fields: []TemplateField{
		{ID: "social_provider", Kind: "select", Options: providerIDs(SocialProviders())},
		{ID: "handle"},
		{ID: "uri"},
	}},
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
	{ID: "bluetooth-le", Fields: []TemplateField{
		{ID: "address", Required: true},
		{ID: "address_type", Kind: "select", Options: []string{"public", "random"}},
		{ID: "name"},
		{ID: "role", Kind: "select", Options: []string{"", "peripheral", "central", "both"}},
	}},
	{ID: "wifi", Fields: []TemplateField{
		{ID: "ssid", Required: true},
		{ID: "security", Kind: "select", Required: true, Options: []string{"wpa2", "wpa2-wpa3", "wpa", "wpa2-wpa", "open"}},
		{ID: "password"},
	}},
	{ID: "dpp", Fields: []TemplateField{{ID: "dpp_uri", Required: true}}},
	{ID: "smartposter", Fields: []TemplateField{
		{ID: "uri", Required: true},
		{ID: "title"},
		{ID: "language", Kind: "select", Options: LanguageOptions()},
		{ID: "action", Kind: "select", Options: []string{"", "do", "save", "edit"}},
		{ID: "resource_type"},
		{ID: "resource_size"},
	}},
}

func providerIDs(providers []Provider) []string {
	ids := make([]string, len(providers))
	for index, provider := range providers {
		ids[index] = provider.ID
	}
	return ids
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
	tnf, recordType, payload, err := templateRecord(template, fields)
	if err != nil {
		return nil, err
	}
	return encodeRecord(tnf, recordType, payload)
}

// RecordInput is one record in a multi-record message.
type RecordInput struct {
	Kind   string
	Fields map[string]string
}

// EncodeInputs builds a validated multi-record NDEF message.
func EncodeInputs(inputs []RecordInput) ([]byte, error) {
	if len(inputs) == 0 {
		return nil, fmt.Errorf("%w: at least one record is required", ErrMalformedNDEF)
	}
	frames := make([][]byte, len(inputs))
	for index, input := range inputs {
		tnf, recordType, payload, err := inputRecord(input)
		if err != nil {
			return nil, err
		}
		frame, err := encodeRecordFrame(tnf, recordType, payload, false, false)
		if err != nil {
			return nil, err
		}
		frames[index] = frame
	}
	frames[0][0] |= 0x80
	frames[len(frames)-1][0] |= 0x40
	var message []byte
	for _, frame := range frames {
		message = append(message, frame...)
	}
	if _, err := ParseMessage(message); err != nil {
		return nil, err
	}
	return message, nil
}

func inputRecord(input RecordInput) (byte, string, []byte, error) {
	switch input.Kind {
	case "external":
		recordType := strings.TrimSpace(input.Fields["type"])
		if recordType == "" {
			return 0, "", nil, fmt.Errorf("%w: external record type is required", ErrMalformedNDEF)
		}
		return 0x04, recordType, []byte(input.Fields["payload"]), nil
	case "mime":
		recordType := strings.TrimSpace(input.Fields["type"])
		if recordType == "" {
			return 0, "", nil, fmt.Errorf("%w: MIME type is required", ErrMalformedNDEF)
		}
		return 0x02, recordType, []byte(input.Fields["payload"]), nil
	default:
		return templateRecord(input.Kind, input.Fields)
	}
}

func templateRecord(template string, fields map[string]string) (byte, string, []byte, error) {
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
			return 0, "", nil, fmt.Errorf("%w: text is required", ErrMalformedNDEF)
		}
		language := value("language")
		if language == "" {
			language = "en"
		}
		payload, err := textPayload(text, language)
		return 0x01, "T", payload, err
	case "uri", "custom":
		uri, err := required("uri")
		if err != nil {
			return 0, "", nil, err
		}
		payload, err := uriPayload(uri)
		return 0x01, "U", payload, err
	case "payment":
		uri, err := presetURI(value, "uri", "payment_provider", "handle", PaymentProviders())
		if err != nil {
			return 0, "", nil, err
		}
		payload, err := uriPayload(uri)
		return 0x01, "U", payload, err
	case "social":
		uri, err := presetURI(value, "uri", "social_provider", "handle", SocialProviders())
		if err != nil {
			return 0, "", nil, err
		}
		payload, err := uriPayload(uri)
		return 0x01, "U", payload, err
	case "search":
		query, err := required("query")
		if err != nil {
			return 0, "", nil, err
		}
		prefix := value("search_prefix")
		if prefix == "" {
			prefix = providerURL(SearchProviders(), value("search_provider"))
		}
		if prefix == "" {
			return 0, "", nil, fmt.Errorf("%w: search prefix or provider is required", ErrMalformedNDEF)
		}
		if !strings.HasPrefix(prefix, "http://") && !strings.HasPrefix(prefix, "https://") {
			return 0, "", nil, fmt.Errorf("%w: search prefix must be an HTTP(S) URL", ErrMalformedNDEF)
		}
		if !strings.Contains(prefix, "?") {
			prefix = strings.TrimRight(prefix, "/") + "/?q="
		}
		payload, err := uriPayload(prefix + url.QueryEscape(query))
		return 0x01, "U", payload, err
	case "app":
		packageName, err := required("package")
		if err != nil {
			return 0, "", nil, err
		}
		return 0x04, "android.com:pkg", []byte(packageName), nil
	case "email":
		to, err := required("to")
		if err != nil {
			return 0, "", nil, err
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
		payload, err := uriPayload(uri)
		return 0x01, "U", payload, err
	case "contact":
		if _, err := required("name"); err != nil {
			return 0, "", nil, err
		}
		payload, err := vcardPayload(fields)
		return 0x02, "text/vcard", payload, err
	case "phone":
		phone, err := required("phone")
		if err != nil {
			return 0, "", nil, err
		}
		payload, err := uriPayload("tel:" + sanitizePhone(phone))
		return 0x01, "U", payload, err
	case "sms":
		number, err := required("number")
		if err != nil {
			return 0, "", nil, err
		}
		uri := "sms:" + sanitizePhone(number)
		if body := value("body"); body != "" {
			uri += "?body=" + url.QueryEscape(body)
		}
		payload, err := uriPayload(uri)
		return 0x01, "U", payload, err
	case "location":
		latitude, err := required("latitude")
		if err != nil {
			return 0, "", nil, err
		}
		longitude, err := required("longitude")
		if err != nil {
			return 0, "", nil, err
		}
		lat, err := strconv.ParseFloat(latitude, 64)
		if err != nil || lat < -90 || lat > 90 {
			return 0, "", nil, fmt.Errorf("%w: latitude must be between -90 and 90", ErrMalformedNDEF)
		}
		lon, err := strconv.ParseFloat(longitude, 64)
		if err != nil || lon < -180 || lon > 180 {
			return 0, "", nil, fmt.Errorf("%w: longitude must be between -180 and 180", ErrMalformedNDEF)
		}
		uri := fmt.Sprintf("geo:%s,%s", strconv.FormatFloat(lat, 'f', -1, 64), strconv.FormatFloat(lon, 'f', -1, 64))
		if label := value("label"); label != "" {
			uri += "?q=" + url.QueryEscape(label)
		}
		payload, err := uriPayload(uri)
		return 0x01, "U", payload, err
	case "address":
		address, err := required("address")
		if err != nil {
			return 0, "", nil, err
		}
		payload, err := uriPayload("geo:0,0?q=" + url.QueryEscape(address))
		return 0x01, "U", payload, err
	case "bluetooth":
		address, err := required("address")
		if err != nil {
			return 0, "", nil, err
		}
		payload, err := bluetoothPayload(address, value("name"))
		return 0x02, "application/vnd.bluetooth.ep.oob", payload, err
	case "bluetooth-le":
		address, err := required("address")
		if err != nil {
			return 0, "", nil, err
		}
		payload, err := bluetoothLEPayload(address, value("address_type"), value("name"), value("role"))
		return 0x02, "application/vnd.bluetooth.le.oob", payload, err
	case "wifi":
		ssid, err := required("ssid")
		if err != nil {
			return 0, "", nil, err
		}
		security := value("security")
		if security == "" {
			security = "wpa2"
		}
		payload, err := wifiPayload(ssid, fields["password"], security)
		return 0x02, "application/vnd.wfa.wsc", payload, err
	case "dpp":
		uri, err := required("dpp_uri")
		if err != nil {
			return 0, "", nil, err
		}
		if !strings.HasPrefix(uri, "DPP:") {
			return 0, "", nil, fmt.Errorf("%w: DPP bootstrapping URI must start with DPP:", ErrMalformedNDEF)
		}
		if !strings.Contains(uri, "K:") {
			return 0, "", nil, fmt.Errorf("%w: DPP bootstrapping URI must contain a public key", ErrMalformedNDEF)
		}
		payload, err := uriPayload(uri)
		return 0x01, "U", payload, err
	case "smartposter":
		uri, err := required("uri")
		if err != nil {
			return 0, "", nil, err
		}
		size := uint64(0)
		if value("resource_size") != "" {
			parsed, err := strconv.ParseUint(value("resource_size"), 10, 32)
			if err != nil {
				return 0, "", nil, fmt.Errorf("%w: Smart Poster resource size must be a 32-bit unsigned integer", ErrMalformedNDEF)
			}
			size = parsed
		}
		action := value("action")
		if action == "edit" {
			action = "open"
		}
		payload, err := smartPosterPayload(uri, value("title"), SmartPosterOptions{
			Language: value("language"), Action: action,
			ResourceType: value("resource_type"), ResourceSize: uint32(size),
		})
		return 0x01, "Sp", payload, err
	default:
		return 0, "", nil, fmt.Errorf("%w: unknown NDEF template %q", ErrMalformedNDEF, template)
	}
}

func presetURI(value func(string) string, uriField, providerField, handleField string, providers []Provider) (string, error) {
	if uri := value(uriField); uri != "" {
		return uri, nil
	}
	prefix := providerURL(providers, value(providerField))
	if prefix == "" {
		return "", fmt.Errorf("%w: URI or provider is required", ErrMalformedNDEF)
	}
	handle := strings.TrimSpace(value(handleField))
	handle = strings.TrimPrefix(handle, "@")
	handle = strings.TrimPrefix(handle, "$")
	if handle == "" {
		return "", fmt.Errorf("%w: provider handle is required", ErrMalformedNDEF)
	}
	return prefix + handle, nil
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
