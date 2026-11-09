package workflow

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// DumpFormat identifies an interchange format at the file boundary. The
// internal project format is the only one that can represent partial state.
type DumpFormat string

const (
	DumpFormatProject  DumpFormat = "project"
	DumpFormatRaw      DumpFormat = "raw"
	DumpFormatMCT      DumpFormat = "mct"
	DumpFormatEML      DumpFormat = "eml"
	DumpFormatMCTJSON  DumpFormat = "mct-json"
	DumpFormatProxmark DumpFormat = "proxmark3"
	DumpFormatKeyArray DumpFormat = "keyarray"
	DumpFormatSidecar  DumpFormat = "sidecar"
)

const maxDumpFileSize = 1 << 20

// DumpLoad is the result of content-based dump detection.
type DumpLoad struct {
	Result   DumpResult
	Format   DumpFormat
	Warnings []string
}

func ParseDumpFormat(name string) (DumpFormat, error) {
	format := DumpFormat(strings.ToLower(strings.TrimSpace(name)))
	switch format {
	case DumpFormatProject, DumpFormatRaw, DumpFormatMCT, DumpFormatEML, DumpFormatMCTJSON, DumpFormatProxmark, DumpFormatKeyArray, DumpFormatSidecar:
		return format, nil
	default:
		return "", fmt.Errorf("unknown dump format %q; use project, raw, proxmark3, keyarray, mct, mct-json, eml, or sidecar", name)
	}
}

// DumpFormatForPath infers the format from an unambiguous extension. A .json
// path means the NFC Foundry project; external JSON needs an explicit format.
func DumpFormatForPath(path string) (DumpFormat, error) {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".bin", ".mfd":
		return DumpFormatRaw, nil
	case ".mct":
		return DumpFormatMCT, nil
	case ".eml":
		return DumpFormatEML, nil
	case ".json":
		return DumpFormatProject, nil
	default:
		return "", fmt.Errorf("cannot infer a dump format from %q; choose a format explicitly", path)
	}
}

// LoadDumpFile detects and loads a raw image, an NFC Foundry project, or a
// supported external dump. It never guesses missing bytes and never registers
// imported keys as verified against the current card.
func LoadDumpFile(path string) (DumpLoad, error) {
	info, err := os.Stat(path)
	if err != nil {
		return DumpLoad{}, err
	}
	if info.Size() > maxDumpFileSize {
		return DumpLoad{}, fmt.Errorf("dump file %s is too large (%d bytes)", path, info.Size())
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return DumpLoad{}, err
	}
	if len(data) > maxDumpFileSize {
		return DumpLoad{}, fmt.Errorf("dump file %s is too large (%d bytes)", path, len(data))
	}
	format, err := detectDumpFormat(data)
	if err != nil {
		return DumpLoad{}, fmt.Errorf("%s: %w", filepath.Base(path), err)
	}
	load := DumpLoad{Format: format}
	switch format {
	case DumpFormatProject:
		load.Result, err = parseDumpProject(data)
	case DumpFormatRaw:
		var warnings []string
		load.Result, warnings, err = loadRawDumpBytes(path, data)
		load.Warnings = append(load.Warnings, warnings...)
	case DumpFormatMCT:
		load.Result, err = parseMCTDump(data)
		load.Warnings = append(load.Warnings, "MCT text dump has no card identity; UID, ATQA, and SAK are empty")
	case DumpFormatEML:
		load.Result, err = parseEMLDump(data)
		load.Warnings = append(load.Warnings, "EML text dump has no card identity; UID, ATQA, and SAK are empty")
	case DumpFormatMCTJSON:
		load.Result, load.Warnings, err = parseMCTJSONDump(data)
	case DumpFormatProxmark:
		load.Result, load.Warnings, err = parseProxmarkDump(data)
	default:
		err = fmt.Errorf("unsupported dump format %q", format)
	}
	if err != nil {
		return DumpLoad{}, err
	}
	return load, nil
}

// ExportDumpFile writes a dump in the requested interchange format. External
// formats require a complete image; a partial image is only valid as a
// project. The returned path is the metadata sidecar for raw exports.
func ExportDumpFile(path string, format DumpFormat, result DumpResult) (string, error) {
	switch format {
	case DumpFormatProject:
		return "", SaveDumpProject(path, result)
	case DumpFormatRaw:
		return SaveRawDump(path, result)
	case DumpFormatSidecar:
		return SaveDumpSidecar(path, result)
	case DumpFormatMCT, DumpFormatEML, DumpFormatMCTJSON, DumpFormatProxmark, DumpFormatKeyArray:
		if err := validateCompleteDump(result); err != nil {
			return "", err
		}
		var encoded []byte
		var err error
		switch format {
		case DumpFormatMCT:
			encoded, err = encodeMCTDump(result)
		case DumpFormatEML:
			encoded, err = encodeEMLDump(result)
		case DumpFormatMCTJSON:
			encoded, err = encodeMCTJSONDump(result)
		case DumpFormatProxmark:
			encoded, err = encodeProxmarkDump(result)
		case DumpFormatKeyArray:
			encoded, err = EncodeKeyArray(result)
		}
		if err != nil {
			return "", err
		}
		return "", atomicWriteFile(path, encoded)
	default:
		return "", fmt.Errorf("unsupported export format %q", format)
	}
}

// SaveDumpSidecar writes only the metadata file for an existing raw image.
func SaveDumpSidecar(path string, result DumpResult) (string, error) {
	if err := validateCompleteDump(result); err != nil {
		return "", err
	}
	if !strings.HasSuffix(strings.ToLower(path), ".nfcx.json") {
		path += ".nfcx.json"
	}
	encoded, err := encodeDumpMetadata(result)
	if err != nil {
		return "", err
	}
	if err := atomicWriteFile(path, encoded); err != nil {
		return "", err
	}
	return path, nil
}

func validateCompleteDump(result DumpResult) error {
	if !result.Complete() {
		return fmt.Errorf("%w: external dump formats are complete-image only", mifare.ErrIncompleteDump)
	}
	if err := result.Dump.ValidateTrailers(); err != nil {
		return err
	}
	if _, err := result.Dump.ValidateBCC(len(result.Card.UID)); err != nil {
		return err
	}
	return nil
}

func encodeDumpMetadata(result DumpResult) ([]byte, error) {
	encoded, err := json.MarshalIndent(MetadataFromDump(result), "", "  ")
	if err != nil {
		return nil, err
	}
	return append(encoded, '\n'), nil
}

func detectDumpFormat(data []byte) (DumpFormat, error) {
	trimmed := bytes.TrimSpace(data)
	if len(trimmed) == 0 {
		return "", errors.New("dump file is empty")
	}
	if trimmed[0] == '{' {
		return detectJSONDumpFormat(trimmed)
	}
	if isPrintableText(data) {
		if bytes.Contains(trimmed, []byte("+Sector:")) {
			return DumpFormatMCT, nil
		}
		if looksLikeEML(trimmed) {
			return DumpFormatEML, nil
		}
		return "", errors.New("unrecognized text dump; expected MCT +Sector headers or one 32-hex block per line")
	}
	switch len(data) {
	case mifare.Classic1KDumpSize, mifare.Classic4KDumpSize:
		return DumpFormatRaw, nil
	default:
		return "", fmt.Errorf("unrecognized dump size %d; expected %d or %d raw bytes, MCT text, EML text, or JSON", len(data), mifare.Classic1KDumpSize, mifare.Classic4KDumpSize)
	}
}

func detectJSONDumpFormat(data []byte) (DumpFormat, error) {
	if err := rejectDuplicateJSONKeys(data); err != nil {
		return "", err
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(data, &object); err != nil {
		return "", fmt.Errorf("invalid JSON dump: %w", err)
	}
	if versionRaw, ok := object["version"]; ok {
		var version json.Number
		if err := json.Unmarshal(versionRaw, &version); err == nil {
			if _, ok := object["layout"]; ok {
				if blocksRaw, ok := object["blocks"]; ok && firstJSONByte(blocksRaw) == '[' {
					return DumpFormatProject, nil
				}
			}
		}
	}
	fileType, present, err := jsonStringField(object, "FileType")
	if err != nil {
		return "", err
	}
	if present {
		switch strings.ToLower(strings.TrimSpace(fileType)) {
		case "mfcard":
			return DumpFormatMCTJSON, nil
		case "mfc v2", "mfc v3":
			return DumpFormatProxmark, nil
		default:
			return "", fmt.Errorf("unsupported JSON dump FileType %q", fileType)
		}
	}
	if blocksRaw, ok := object["blocks"]; ok && firstJSONByte(blocksRaw) == '{' {
		return DumpFormatMCTJSON, nil
	}
	return "", errors.New("JSON dump has no recognized FileType or blocks map")
}

func firstJSONByte(raw json.RawMessage) byte {
	trimmed := bytes.TrimSpace(raw)
	if len(trimmed) == 0 {
		return 0
	}
	return trimmed[0]
}

func isPrintableText(data []byte) bool {
	for _, value := range data {
		if value == '\n' || value == '\r' || value == '\t' || (value >= 0x20 && value <= 0x7e) {
			continue
		}
		return false
	}
	return true
}

func looksLikeEML(data []byte) bool {
	lines := strings.Split(string(data), "\n")
	count := 0
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		if len(line) != nfc.BlockSize*2 {
			return false
		}
		if _, err := hex.DecodeString(line); err != nil {
			return false
		}
		count++
	}
	return count == mifare.Classic1KBlocks || count == mifare.Classic4KBlocks
}

func rejectDuplicateJSONKeys(data []byte) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := scanJSONValue(decoder, "$"); err != nil {
		return err
	}
	if _, err := decoder.Token(); err != io.EOF {
		if err == nil {
			return errors.New("invalid JSON dump: trailing data")
		}
		return fmt.Errorf("invalid JSON dump: %w", err)
	}
	return nil
}

func scanJSONValue(decoder *json.Decoder, path string) error {
	token, err := decoder.Token()
	if err != nil {
		return fmt.Errorf("invalid JSON dump: %w", err)
	}
	delimiter, ok := token.(json.Delim)
	if !ok {
		return nil
	}
	switch delimiter {
	case '{':
		seen := make(map[string]struct{})
		for decoder.More() {
			keyToken, err := decoder.Token()
			if err != nil {
				return fmt.Errorf("invalid JSON dump: %w", err)
			}
			key, ok := keyToken.(string)
			if !ok {
				return fmt.Errorf("invalid JSON dump: object %s has a non-string key", path)
			}
			if _, duplicate := seen[key]; duplicate {
				return fmt.Errorf("invalid JSON dump: object %s contains duplicate key %q", path, key)
			}
			seen[key] = struct{}{}
			if err := scanJSONValue(decoder, path+"."+key); err != nil {
				return err
			}
		}
		if _, err := decoder.Token(); err != nil {
			return fmt.Errorf("invalid JSON dump: %w", err)
		}
	case '[':
		index := 0
		for decoder.More() {
			if err := scanJSONValue(decoder, fmt.Sprintf("%s[%d]", path, index)); err != nil {
				return err
			}
			index++
		}
		if _, err := decoder.Token(); err != nil {
			return fmt.Errorf("invalid JSON dump: %w", err)
		}
	default:
		return fmt.Errorf("invalid JSON dump: unexpected delimiter %q", delimiter)
	}
	return nil
}

func jsonField(object map[string]json.RawMessage, name string) (json.RawMessage, bool) {
	if raw, ok := object[name]; ok {
		return raw, true
	}
	for key, raw := range object {
		if strings.EqualFold(key, name) {
			return raw, true
		}
	}
	return nil, false
}

func jsonStringField(object map[string]json.RawMessage, name string) (string, bool, error) {
	raw, ok := jsonField(object, name)
	if !ok {
		return "", false, nil
	}
	var value string
	if err := json.Unmarshal(raw, &value); err != nil {
		return "", true, fmt.Errorf("JSON field %s must be a string", name)
	}
	return value, true, nil
}

func parseDumpProject(data []byte) (DumpResult, error) {
	var metadata DumpMetadata
	if err := json.Unmarshal(data, &metadata); err != nil {
		return DumpResult{}, fmt.Errorf("invalid NFC Foundry dump: %w", err)
	}
	return DumpFromMetadata(metadata)
}

func loadRawDumpBytes(path string, raw []byte) (DumpResult, []string, error) {
	image, err := mifare.ParseRaw(raw)
	if err != nil {
		return DumpResult{}, nil, err
	}
	metadataPath := path + ".nfcx.json"
	result, err := LoadDumpProject(metadataPath)
	if os.IsNotExist(err) {
		if err := image.ValidateTrailers(); err != nil {
			return DumpResult{}, nil, err
		}
		return DumpResult{Dump: image}, []string{"raw dump has no NFCX sidecar; card identity and key provenance are unavailable"}, nil
	}
	if err != nil {
		return DumpResult{}, nil, err
	}
	stored, err := result.Dump.Raw()
	if err != nil {
		return DumpResult{}, nil, err
	}
	if !equalBytes(stored, raw) {
		return DumpResult{}, nil, errors.New("raw dump does not match its NFCX metadata sidecar")
	}
	if err := result.Dump.ValidateTrailers(); err != nil {
		return DumpResult{}, nil, err
	}
	if _, err := result.Dump.ValidateBCC(len(result.Card.UID)); err != nil {
		return DumpResult{}, nil, err
	}
	return result, nil, nil
}

func parseRawDumpImage(raw []byte, card nfc.CardInfo) (DumpResult, error) {
	image, err := mifare.ParseRaw(raw)
	if err != nil {
		return DumpResult{}, err
	}
	if err := image.ValidateTrailers(); err != nil {
		return DumpResult{}, err
	}
	if _, err := image.ValidateBCC(len(card.UID)); err != nil {
		return DumpResult{}, err
	}
	return DumpResult{Dump: image, Card: card}, nil
}

func parseMCTDump(data []byte) (DumpResult, error) {
	lines := strings.Split(string(data), "\n")
	sectors := make(map[int][][nfc.BlockSize]byte)
	currentSector := -1
	layout4K := false
	for lineNumber, rawLine := range lines {
		line := strings.TrimSpace(rawLine)
		if line == "" {
			continue
		}
		if sector, isHeader, err := parseMCTSectorHeader(line); err != nil {
			return DumpResult{}, fmt.Errorf("line %d: %w", lineNumber+1, err)
		} else if isHeader {
			if sector >= mifare.Classic1KSectors {
				layout4K = true
			}
			if _, duplicate := sectors[sector]; duplicate {
				return DumpResult{}, fmt.Errorf("line %d: sector %d appears more than once", lineNumber+1, sector)
			}
			sectors[sector] = nil
			currentSector = sector
			continue
		}
		if currentSector < 0 {
			return DumpResult{}, fmt.Errorf("line %d: block data appears before a +Sector header", lineNumber+1)
		}
		if len(line) != nfc.BlockSize*2 {
			return DumpResult{}, fmt.Errorf("line %d: expected 32 hexadecimal digits, found %q", lineNumber+1, line)
		}
		decoded, err := hex.DecodeString(line)
		if err != nil {
			return DumpResult{}, fmt.Errorf("line %d: block data is not hexadecimal", lineNumber+1)
		}
		var block [nfc.BlockSize]byte
		copy(block[:], decoded)
		sectors[currentSector] = append(sectors[currentSector], block)
	}
	layout := mifare.Classic1K
	if layout4K {
		layout = mifare.Classic4K
	}
	raw, err := mctSectorsToRaw(layout, sectors)
	if err != nil {
		return DumpResult{}, err
	}
	return parseRawDumpImage(raw, nfc.CardInfo{})
}

func parseMCTSectorHeader(line string) (int, bool, error) {
	const prefix = "+Sector:"
	if len(line) < len(prefix) || !strings.EqualFold(line[:len(prefix)], prefix) {
		return 0, false, nil
	}
	value := strings.TrimSpace(line[len(prefix):])
	if value == "" {
		return 0, true, errors.New("+Sector header has no sector number")
	}
	sector, err := strconv.Atoi(value)
	if err != nil || sector < 0 || sector >= mifare.Classic4KSectors {
		return 0, true, fmt.Errorf("invalid sector number %q", value)
	}
	return sector, true, nil
}

func mctSectorsToRaw(layout mifare.Layout, sectors map[int][][nfc.BlockSize]byte) ([]byte, error) {
	sectorCount, err := layout.SectorCount()
	if err != nil {
		return nil, err
	}
	size, _ := layout.DumpSize()
	raw := make([]byte, size)
	for sector := range sectorCount {
		blocks, ok := sectors[sector]
		if !ok {
			return nil, fmt.Errorf("MCT dump is missing sector %d", sector)
		}
		expected, _ := layout.BlocksInSector(sector)
		if len(blocks) != expected {
			return nil, fmt.Errorf("MCT sector %d has %d blocks, expected %d", sector, len(blocks), expected)
		}
		first, _ := layout.FirstBlock(sector)
		for index, block := range blocks {
			copy(raw[(first+index)*nfc.BlockSize:], block[:])
		}
	}
	for sector := range sectors {
		if sector >= sectorCount {
			return nil, fmt.Errorf("MCT dump contains sector %d outside the %s layout", sector, layoutName(layout))
		}
	}
	return raw, nil
}

func parseEMLDump(data []byte) (DumpResult, error) {
	lines := strings.Split(string(data), "\n")
	raw := make([]byte, 0, len(data))
	for lineNumber, rawLine := range lines {
		line := strings.TrimSpace(rawLine)
		if line == "" {
			continue
		}
		if len(line) != nfc.BlockSize*2 {
			return DumpResult{}, fmt.Errorf("line %d: expected 32 hexadecimal digits", lineNumber+1)
		}
		decoded, err := hex.DecodeString(line)
		if err != nil {
			return DumpResult{}, fmt.Errorf("line %d: block data is not hexadecimal", lineNumber+1)
		}
		raw = append(raw, decoded...)
	}
	if len(raw) != mifare.Classic1KDumpSize && len(raw) != mifare.Classic4KDumpSize {
		return DumpResult{}, fmt.Errorf("EML dump has %d blocks; expected %d or %d", len(raw)/nfc.BlockSize, mifare.Classic1KBlocks, mifare.Classic4KBlocks)
	}
	return parseRawDumpImage(raw, nfc.CardInfo{})
}

func parseMCTJSONDump(data []byte) (DumpResult, []string, error) {
	var object map[string]json.RawMessage
	if err := json.Unmarshal(data, &object); err != nil {
		return DumpResult{}, nil, fmt.Errorf("invalid MCT JSON dump: %w", err)
	}
	fileType, present, err := jsonStringField(object, "FileType")
	if err != nil {
		return DumpResult{}, nil, err
	}
	if present && !strings.EqualFold(strings.TrimSpace(fileType), "mfcard") {
		return DumpResult{}, nil, fmt.Errorf("MCT JSON FileType is %q, expected mfcard", fileType)
	}
	blocksRaw, ok := object["blocks"]
	if !ok {
		return DumpResult{}, nil, errors.New("MCT JSON dump has no blocks map")
	}
	blockMap, layout, err := decodeJSONBlockMap(blocksRaw)
	if err != nil {
		return DumpResult{}, nil, err
	}
	raw, err := blockMapToRaw(blockMap, layout)
	if err != nil {
		return DumpResult{}, nil, err
	}
	card, warnings, err := decodeMCTJSONCard(object)
	if err != nil {
		return DumpResult{}, nil, err
	}
	result, err := parseRawDumpImage(raw, card)
	if err != nil {
		return DumpResult{}, nil, err
	}
	return result, warnings, nil
}

func decodeMCTJSONCard(object map[string]json.RawMessage) (nfc.CardInfo, []string, error) {
	cardRaw, ok := jsonField(object, "Card")
	if !ok {
		return nfc.CardInfo{}, []string{"MCT JSON has no card identity; UID, ATQA, and SAK are empty"}, nil
	}
	var cardObject map[string]json.RawMessage
	if err := json.Unmarshal(cardRaw, &cardObject); err != nil {
		return nfc.CardInfo{}, nil, errors.New("MCT JSON Card must be an object")
	}
	card, err := decodeJSONCard(cardObject)
	if err != nil {
		return nfc.CardInfo{}, nil, err
	}
	var warnings []string
	if len(card.UID) == 0 {
		warnings = append(warnings, "MCT JSON has no card UID; BCC validation is not possible")
	}
	return card, warnings, nil
}

func parseProxmarkDump(data []byte) (DumpResult, []string, error) {
	var object map[string]json.RawMessage
	if err := json.Unmarshal(data, &object); err != nil {
		return DumpResult{}, nil, fmt.Errorf("invalid Proxmark3 JSON dump: %w", err)
	}
	fileType, present, err := jsonStringField(object, "FileType")
	if err != nil {
		return DumpResult{}, nil, err
	}
	if !present {
		return DumpResult{}, nil, errors.New("Proxmark3 JSON dump has no FileType")
	}
	version := strings.ToLower(strings.TrimSpace(fileType))
	if version != "mfc v2" && version != "mfc v3" {
		return DumpResult{}, nil, fmt.Errorf("Proxmark3 JSON FileType is %q, expected mfc v2 or mfc v3", fileType)
	}
	blocksRaw, ok := object["blocks"]
	if !ok {
		return DumpResult{}, nil, errors.New("Proxmark3 JSON dump has no blocks map")
	}
	blockMap, layout, err := decodeJSONBlockMap(blocksRaw)
	if err != nil {
		return DumpResult{}, nil, err
	}
	raw, err := blockMapToRaw(blockMap, layout)
	if err != nil {
		return DumpResult{}, nil, err
	}
	cardRaw, ok := jsonField(object, "Card")
	if !ok {
		return DumpResult{}, nil, errors.New("Proxmark3 JSON dump has no Card object")
	}
	var cardObject map[string]json.RawMessage
	if err := json.Unmarshal(cardRaw, &cardObject); err != nil {
		return DumpResult{}, nil, errors.New("Proxmark3 JSON Card must be an object")
	}
	card, err := decodeJSONCard(cardObject)
	if err != nil {
		return DumpResult{}, nil, err
	}
	result, err := parseRawDumpImage(raw, card)
	if err != nil {
		return DumpResult{}, nil, err
	}
	sectorKeys, err := decodeProxmarkSectorKeys(object, result.Dump)
	if err != nil {
		return DumpResult{}, nil, err
	}
	result.Keys = sectorKeys
	var warnings []string
	if len(card.UID) == 0 {
		warnings = append(warnings, "Proxmark3 JSON has no card UID; BCC validation is not possible")
	}
	if version == "mfc v3" {
		if ats, ok, _ := jsonStringField(cardObject, "ATS"); ok && strings.TrimSpace(ats) != "" {
			warnings = append(warnings, "Proxmark3 mfc v3 Card.ATS was dropped; the dump model does not store ATS")
		}
		if signature, ok, _ := jsonStringField(cardObject, "SIGNATURE"); ok && strings.TrimSpace(signature) != "" {
			warnings = append(warnings, "Proxmark3 mfc v3 Card.SIGNATURE was dropped; the dump model does not store signatures")
		}
	}
	return result, warnings, nil
}

func decodeJSONCard(object map[string]json.RawMessage) (nfc.CardInfo, error) {
	uidHex, _, err := jsonStringField(object, "UID")
	if err != nil {
		return nfc.CardInfo{}, err
	}
	uid, err := decodeOptionalHex(strings.TrimSpace(uidHex), []int{0, 4, 7, 10}, "card UID")
	if err != nil {
		return nfc.CardInfo{}, err
	}
	atqaHex, _, err := jsonStringField(object, "ATQA")
	if err != nil {
		return nfc.CardInfo{}, err
	}
	atqa, err := decodeOptionalHex(strings.TrimSpace(atqaHex), []int{0, nfc.ATQALength}, "card ATQA")
	if err != nil {
		return nfc.CardInfo{}, err
	}
	sakHex, _, err := jsonStringField(object, "SAK")
	if err != nil {
		return nfc.CardInfo{}, err
	}
	sakBytes, err := decodeOptionalHex(strings.TrimSpace(sakHex), []int{0, 1}, "card SAK")
	if err != nil {
		return nfc.CardInfo{}, err
	}
	card := nfc.CardInfo{UID: uid}
	copy(card.ATQA[:], atqa)
	if len(sakBytes) == 1 {
		card.SAK = sakBytes[0]
	}
	return card, nil
}

func decodeJSONBlockMap(raw json.RawMessage) (map[string]string, mifare.Layout, error) {
	var blocks map[string]string
	if err := json.Unmarshal(raw, &blocks); err != nil {
		return nil, 0, errors.New("dump blocks must be an object keyed by decimal block number")
	}
	var layout mifare.Layout
	switch len(blocks) {
	case mifare.Classic1KBlocks:
		layout = mifare.Classic1K
	case mifare.Classic4KBlocks:
		layout = mifare.Classic4K
	default:
		return nil, 0, fmt.Errorf("dump has %d blocks; expected %d or %d", len(blocks), mifare.Classic1KBlocks, mifare.Classic4KBlocks)
	}
	return blocks, layout, nil
}

func blockMapToRaw(blocks map[string]string, layout mifare.Layout) ([]byte, error) {
	count, _ := layout.BlockCount()
	raw := make([]byte, count*nfc.BlockSize)
	seen := make([]bool, count)
	for key, value := range blocks {
		index, err := strconv.Atoi(strings.TrimSpace(key))
		if err != nil || index < 0 || index >= count {
			return nil, fmt.Errorf("block key %q is outside the %d-block layout", key, count)
		}
		if seen[index] {
			return nil, fmt.Errorf("block %d appears more than once", index)
		}
		data, err := hex.DecodeString(strings.TrimSpace(value))
		if err != nil || len(data) != nfc.BlockSize {
			return nil, fmt.Errorf("block %d must contain 32 hexadecimal digits", index)
		}
		copy(raw[index*nfc.BlockSize:], data)
		seen[index] = true
	}
	for index, present := range seen {
		if !present {
			return nil, fmt.Errorf("block %d is missing", index)
		}
	}
	return raw, nil
}

func decodeProxmarkSectorKeys(object map[string]json.RawMessage, image mifare.Dump) ([]VerifiedSectorKeys, error) {
	raw, ok := jsonField(object, "SectorKeys")
	if !ok {
		return nil, nil
	}
	var stored map[string]json.RawMessage
	if err := json.Unmarshal(raw, &stored); err != nil {
		return nil, errors.New("Proxmark3 SectorKeys must be an object")
	}
	sectors, _ := image.Layout.SectorCount()
	keys := make([]VerifiedSectorKeys, sectors)
	for sector := range keys {
		keys[sector].Sector = sector
	}
	for key, rawSector := range stored {
		sector, err := strconv.Atoi(strings.TrimSpace(key))
		if err != nil || sector < 0 || sector >= sectors {
			return nil, fmt.Errorf("SectorKeys key %q is outside the %d-sector layout", key, sectors)
		}
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(rawSector, &fields); err != nil {
			return nil, fmt.Errorf("SectorKeys sector %d must be an object", sector)
		}
		trailerBlock, _ := image.Layout.TrailerBlock(sector)
		trailer := image.Blocks[trailerBlock].Data
		if keyA, present, err := jsonStringField(fields, "KeyA"); err != nil {
			return nil, err
		} else if present && strings.TrimSpace(keyA) != "" {
			decoded, err := decodeSectorKeyField(keyA, trailer[0:6], sector, "Key A")
			if err != nil {
				return nil, err
			}
			keys[sector].KeyA = &decoded
		}
		if keyB, present, err := jsonStringField(fields, "KeyB"); err != nil {
			return nil, err
		} else if present && strings.TrimSpace(keyB) != "" {
			decoded, err := decodeSectorKeyField(keyB, trailer[10:16], sector, "Key B")
			if err != nil {
				return nil, err
			}
			keys[sector].KeyB = &decoded
		}
		if access, present, err := jsonStringField(fields, "AccessConditions"); err != nil {
			return nil, err
		} else if present && strings.TrimSpace(access) != "" {
			decoded, err := hex.DecodeString(strings.TrimSpace(access))
			if err != nil || len(decoded) != 4 {
				return nil, fmt.Errorf("sector %d AccessConditions must contain 8 hexadecimal digits", sector)
			}
			if !equalBytes(decoded, trailer[6:10]) {
				return nil, fmt.Errorf("sector %d AccessConditions do not match the trailer block", sector)
			}
		}
	}
	return keys, nil
}

func decodeSectorKeyField(value string, expected []byte, sector int, name string) (nfc.Key, error) {
	decoded, err := hex.DecodeString(strings.TrimSpace(value))
	if err != nil || len(decoded) != nfc.KeySize {
		return nfc.Key{}, fmt.Errorf("sector %d %s must contain 12 hexadecimal digits", sector, name)
	}
	if !equalBytes(decoded, expected) {
		return nfc.Key{}, fmt.Errorf("sector %d %s does not match the trailer block", sector, name)
	}
	key, _ := nfc.KeyFromBytes(decoded)
	return key, nil
}

func encodeMCTDump(result DumpResult) ([]byte, error) {
	var builder strings.Builder
	sectors, _ := result.Dump.Layout.SectorCount()
	for sector := range sectors {
		fmt.Fprintf(&builder, "+Sector: %d\n", sector)
		first, _ := result.Dump.Layout.FirstBlock(sector)
		count, _ := result.Dump.Layout.BlocksInSector(sector)
		for block := first; block < first+count; block++ {
			builder.WriteString(strings.ToUpper(hex.EncodeToString(result.Dump.Blocks[block].Data[:])))
			builder.WriteByte('\n')
		}
	}
	return []byte(builder.String()), nil
}

func encodeEMLDump(result DumpResult) ([]byte, error) {
	var builder strings.Builder
	for _, block := range result.Dump.Blocks {
		builder.WriteString(strings.ToUpper(hex.EncodeToString(block.Data[:])))
		builder.WriteByte('\n')
	}
	return []byte(builder.String()), nil
}

type mctJSONExport struct {
	Created  string            `json:"Created"`
	FileType string            `json:"FileType"`
	Blocks   map[string]string `json:"blocks"`
}

func encodeMCTJSONDump(result DumpResult) ([]byte, error) {
	document := mctJSONExport{Created: "NFC Foundry", FileType: "mfcard", Blocks: dumpBlockMap(result.Dump)}
	encoded, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(encoded, '\n'), nil
}

type proxmarkCardExport struct {
	UID  string `json:"UID"`
	ATQA string `json:"ATQA"`
	SAK  string `json:"SAK"`
}

type proxmarkSectorExport struct {
	KeyA                 string            `json:"KeyA"`
	KeyB                 string            `json:"KeyB"`
	AccessConditions     string            `json:"AccessConditions"`
	AccessConditionsText map[string]string `json:"AccessConditionsText,omitempty"`
}

type proxmarkExport struct {
	Created    string                          `json:"Created"`
	FileType   string                          `json:"FileType"`
	Card       proxmarkCardExport              `json:"Card"`
	Blocks     map[string]string               `json:"blocks"`
	SectorKeys map[string]proxmarkSectorExport `json:"SectorKeys"`
}

func encodeProxmarkDump(result DumpResult) ([]byte, error) {
	document := proxmarkExport{
		Created: "NFC Foundry", FileType: "mfc v2",
		Card: proxmarkCardExport{
			UID:  strings.ToUpper(hex.EncodeToString(result.Card.UID)),
			ATQA: strings.ToUpper(hex.EncodeToString(result.Card.ATQA[:])),
			SAK:  fmt.Sprintf("%02X", result.Card.SAK),
		},
		Blocks: dumpBlockMap(result.Dump), SectorKeys: make(map[string]proxmarkSectorExport),
	}
	sectors, _ := result.Dump.Layout.SectorCount()
	for sector := range sectors {
		trailerBlock, _ := result.Dump.Layout.TrailerBlock(sector)
		trailer := result.Dump.Blocks[trailerBlock].Data
		document.SectorKeys[strconv.Itoa(sector)] = proxmarkSectorExport{
			KeyA:                 strings.ToUpper(hex.EncodeToString(trailer[0:6])),
			KeyB:                 strings.ToUpper(hex.EncodeToString(trailer[10:16])),
			AccessConditions:     strings.ToUpper(hex.EncodeToString(trailer[6:10])),
			AccessConditionsText: accessConditionsText(result.Dump.Layout, sector, trailer),
		}
	}
	encoded, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(encoded, '\n'), nil
}

func dumpBlockMap(image mifare.Dump) map[string]string {
	blocks := make(map[string]string, len(image.Blocks))
	for index, block := range image.Blocks {
		blocks[strconv.Itoa(index)] = strings.ToUpper(hex.EncodeToString(block.Data[:]))
	}
	return blocks
}

func accessConditionsText(layout mifare.Layout, sector int, trailer [nfc.BlockSize]byte) map[string]string {
	bits, err := mifare.DecodeAccessBits([3]byte{trailer[6], trailer[7], trailer[8]})
	if err != nil {
		return nil
	}
	first, _ := layout.FirstBlock(sector)
	count, _ := layout.BlocksInSector(sector)
	trailerBlock := first + count - 1
	labels := []string{
		fmt.Sprintf("block%d", first),
		fmt.Sprintf("block%d", first+1),
		fmt.Sprintf("block%d", first+2),
		fmt.Sprintf("block%d", trailerBlock),
	}
	if count == 16 {
		labels = []string{
			fmt.Sprintf("blocks%d-%d", first, first+4),
			fmt.Sprintf("blocks%d-%d", first+5, first+9),
			fmt.Sprintf("blocks%d-%d", first+10, first+14),
			fmt.Sprintf("block%d", trailerBlock),
		}
	}
	text := make(map[string]string, 5)
	for group, label := range labels {
		text[label] = accessConditionDescription(bits.Groups[group], group == 3)
	}
	text["UserData"] = fmt.Sprintf("%02X", trailer[9])
	return text
}

func accessConditionDescription(condition mifare.AccessCondition, trailer bool) string {
	code := condition.Code()
	if trailer {
		switch code {
		case 0:
			return "Key A write A; access bits read A, write A; Key B read A, write A"
		case 1:
			return "Key A write A; access bits read A, write never; Key B read A, write A"
		case 2:
			return "Key A write never; access bits read A, write never; Key B read never, write never"
		case 3:
			return "Key A write B; access bits read A|B, write B; Key B write B"
		case 4:
			return "Key A write B; access bits read A|B, write never; Key B read never, write B"
		default:
			return "Key A write never; access bits read A|B, write never; Key B read never, write never"
		}
	}
	switch code {
	case 0:
		return "read A|B, write A|B, increment A|B, decrement A|B"
	case 1:
		return "read A|B, write never, increment never, decrement A|B"
	case 2:
		return "read A|B, write never, increment never, decrement never"
	case 3:
		return "read B, write B, increment never, decrement never"
	case 4:
		return "read A|B, write B, increment never, decrement never"
	case 5:
		return "read B, write never, increment never, decrement never"
	case 6:
		return "read A|B, write B, increment B, decrement B"
	default:
		return "read never, write never, increment never, decrement never"
	}
}
