package type2

import (
	"fmt"
	"strings"
)

// EncodeVCardMessage builds a vCard 3.0 MIME record. The name field is
// required; the remaining fields are optional.
func EncodeVCardMessage(fields map[string]string) ([]byte, error) {
	payload, err := vcardPayload(fields)
	if err != nil {
		return nil, err
	}
	return encodeRecord(0x02, "text/vcard", payload)
}

func vcardPayload(fields map[string]string) ([]byte, error) {
	name := strings.TrimSpace(fields["name"])
	if name == "" {
		return nil, fmt.Errorf("%w: contact name is required", ErrMalformedNDEF)
	}
	lines := []string{
		"BEGIN:VCARD",
		"VERSION:3.0",
		"FN:" + escapeVCard(name),
		"N:" + escapeVCard(name) + ";;;;",
	}
	appendLine := func(property, value string) {
		value = strings.TrimSpace(value)
		if value != "" {
			lines = append(lines, property+":"+escapeVCard(value))
		}
	}
	appendLine("ORG", fields["organization"])
	appendLine("TITLE", fields["job_title"])
	appendLine("TEL;TYPE=CELL", fields["phone"])
	appendLine("EMAIL", fields["email"])
	if address := strings.TrimSpace(fields["address"]); address != "" {
		lines = append(lines, "ADR;TYPE=WORK:;;"+escapeVCard(address)+";;;;")
	}
	appendLine("URL", fields["url"])
	appendLine("NOTE", fields["note"])
	lines = append(lines, "END:VCARD")
	return []byte(strings.Join(lines, "\r\n") + "\r\n"), nil
}

func parseVCard(payload []byte) (string, []Field) {
	lines := unfoldVCard(string(payload))
	values := make(map[string][]string)
	for _, line := range lines {
		property, value, ok := strings.Cut(line, ":")
		if !ok {
			continue
		}
		name := strings.ToUpper(strings.TrimSpace(strings.Split(property, ";")[0]))
		value = unescapeVCard(value)
		if value == "" {
			continue
		}
		values[name] = append(values[name], value)
	}
	fields := make([]Field, 0, 8)
	name := firstVCardValue(values, "FN")
	if name == "" {
		name = vCardName(values["N"])
	}
	add := func(key, value string) {
		if value != "" {
			fields = append(fields, Field{Key: key, Value: value})
		}
	}
	add("name", name)
	add("organization", strings.Join(values["ORG"], " · "))
	add("job_title", firstVCardValue(values, "TITLE"))
	add("phone", strings.Join(values["TEL"], " · "))
	add("email", strings.Join(values["EMAIL"], " · "))
	if addresses := values["ADR"]; len(addresses) > 0 {
		parts := make([]string, 0, len(addresses))
		for _, address := range addresses {
			parts = append(parts, vCardAddress(address))
		}
		add("address", strings.Join(parts, " · "))
	}
	add("url", strings.Join(values["URL"], " · "))
	add("note", strings.Join(values["NOTE"], " · "))
	return name, fields
}

func firstVCardValue(values map[string][]string, key string) string {
	if items := values[key]; len(items) > 0 {
		return items[0]
	}
	return ""
}

func vCardName(value []string) string {
	if len(value) == 0 {
		return ""
	}
	parts := strings.Split(value[0], ";")
	nonEmpty := parts[:0]
	for _, part := range parts {
		if strings.TrimSpace(part) != "" {
			nonEmpty = append(nonEmpty, strings.TrimSpace(part))
		}
	}
	return strings.Join(nonEmpty, " ")
}

func vCardAddress(value string) string {
	parts := strings.Split(value, ";")
	nonEmpty := parts[:0]
	for _, part := range parts {
		if strings.TrimSpace(part) != "" {
			nonEmpty = append(nonEmpty, strings.TrimSpace(part))
		}
	}
	return strings.Join(nonEmpty, ", ")
}

func unfoldVCard(value string) []string {
	value = strings.ReplaceAll(value, "\r\n", "\n")
	raw := strings.Split(value, "\n")
	lines := make([]string, 0, len(raw))
	for _, line := range raw {
		if (strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t")) && len(lines) > 0 {
			lines[len(lines)-1] += line[1:]
			continue
		}
		lines = append(lines, line)
	}
	return lines
}

func escapeVCard(value string) string {
	replacer := strings.NewReplacer("\\", "\\\\", "\n", "\\n", ";", "\\;", ",", "\\,")
	return replacer.Replace(value)
}

func unescapeVCard(value string) string {
	var result strings.Builder
	escaped := false
	for _, char := range value {
		if escaped {
			switch char {
			case 'n', 'N':
				result.WriteByte('\n')
			default:
				result.WriteRune(char)
			}
			escaped = false
			continue
		}
		if char == '\\' {
			escaped = true
			continue
		}
		result.WriteRune(char)
	}
	return strings.TrimSpace(result.String())
}
