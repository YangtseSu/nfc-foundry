package cli_test

import (
	"encoding/binary"
	"encoding/hex"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/cli"
	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

func newISODEPHarness(t *testing.T, handler mock.ISODEPHandler) (*testHarness, *mock.Reader) {
	t.Helper()
	dir := t.TempDir()
	keyPath := filepath.Join(dir, "keys.json")
	lockDir := filepath.Join(dir, "locks")
	card := nfc.CardInfo{UID: []byte{0x04, 0xA1, 0xB2, 0xC3}, ATQA: [2]byte{0x00, 0x02}, SAK: 0x20}
	enumerator := mock.NewEnumerator([]nfc.DeviceInfo{{Name: "Mock ISO-DEP reader", ConnString: testConnString}}, nil)
	reader := mock.NewISODEPReader(card, []byte{0x01}, handler)
	harness := &testHarness{t: t, keys: keyPath, dir: dir}
	harness.deps = cli.Dependencies{NewService: func(emitter app.EventEmitter) *app.Service {
		return app.NewServiceWithOptions(app.Options{
			Emitter:    emitter,
			Enumerator: enumerator,
			NewReader:  func() nfc.Reader { return reader },
			DeviceConfig: device.Config{
				PollInterval: 2 * time.Millisecond, PollTimeout: 50 * time.Millisecond,
				OpenTimeout: time.Second, RemovalMisses: 2,
				ReconnectBackoffs: []time.Duration{time.Millisecond}, LockDir: lockDir,
			},
			KeyStorePath:      keyPath,
			Type2BackupRoot:   filepath.Join(dir, "type2-backups"),
			ClassicBackupRoot: filepath.Join(dir, "classic-backups"),
			Type4BackupRoot:   filepath.Join(dir, "type4-backups"),
		})
	}}
	return harness, reader
}

func Type4ResponseHandler(t *testing.T, message []byte) mock.ISODEPHandler {
	t.Helper()
	cc := make([]byte, 15)
	binary.BigEndian.PutUint16(cc[0:2], 15)
	cc[2] = 0x20
	binary.BigEndian.PutUint16(cc[3:5], 0x3B)
	binary.BigEndian.PutUint16(cc[5:7], 0x3B)
	cc[7], cc[8] = 0x04, 0x06
	binary.BigEndian.PutUint16(cc[9:11], 0xE104)
	binary.BigEndian.PutUint16(cc[11:13], 100)
	cc[13], cc[14] = 0x00, 0x00
	file := make([]byte, 2+len(message))
	binary.BigEndian.PutUint16(file[0:2], uint16(len(message)))
	copy(file[2:], message)
	selectedFile := uint16(0)
	return func(apdu []byte) []byte {
		if len(apdu) < 4 {
			return []byte{0x67, 0x00}
		}
		switch {
		case apdu[0] == 0x00 && apdu[1] == 0xA4 && apdu[2] == 0x04:
			return []byte{0x90, 0x00}
		case apdu[0] == 0x00 && apdu[1] == 0xA4 && apdu[2] == 0x00:
			selectedFile = uint16(apdu[5])<<8 | uint16(apdu[6])
			return []byte{0x90, 0x00}
		case apdu[0] == 0x00 && apdu[1] == 0xB0:
			offset := int(apdu[2])<<8 | int(apdu[3])
			length := int(apdu[4])
			if length == 0 {
				length = 256
			}
			source := file
			if selectedFile == 0xE103 {
				source = cc
			}
			if offset >= len(source) {
				return []byte{0x62, 0x82}
			}
			end := min(offset+length, len(source))
			return append(append([]byte(nil), source[offset:end]...), 0x90, 0x00)
		case apdu[0] == 0x00 && apdu[1] == 0xD6:
			offset := int(apdu[2])<<8 | int(apdu[3])
			length := int(apdu[4])
			data := apdu[5 : 5+length]
			if offset+len(data) > len(file) {
				file = append(file, make([]byte, offset+len(data)-len(file))...)
			}
			copy(file[offset:], data)
			return []byte{0x90, 0x00}
		default:
			t.Fatalf("unexpected Type 4 APDU % X", apdu)
			return []byte{0x6D, 0x00}
		}
	}
}

func TestCLIAPDURawAndDESFireRead(t *testing.T) {
	handler := func(apdu []byte) []byte {
		switch apdu[1] {
		case 0x60:
			return []byte{0x04, 0x01, 0x01, 0x12, 0x34, 0x18, 0x05, 0x91, 0x00}
		case 0x6A:
			return []byte{0x01, 0x00, 0x00, 0x91, 0x00}
		case 0x5A:
			return []byte{0x91, 0x00}
		case 0x6F:
			return []byte{0x01, 0x91, 0x00}
		case 0xF5:
			return []byte{0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x91, 0x00}
		case 0xBD:
			return []byte{0xDE, 0xAD, 0xBE, 0xEF, 0x91, 0x00}
		default:
			if len(apdu) == 5 && apdu[0] == 0x00 && apdu[1] == 0xB0 {
				return []byte{0xCA, 0xFE, 0x90, 0x00}
			}
			t.Fatalf("unexpected APDU % X", apdu)
			return []byte{0x6D, 0x00}
		}
	}
	harness, _ := newISODEPHarness(t, handler)

	code, envelope := harness.run("apdu", "--raw", "00B000000F", "--json")
	if code != 0 {
		t.Fatalf("apdu exit %d: %s", code, harness.stderr.String())
	}
	var apdu app.APDUResponseDTO
	decodeResult(t, envelope, &apdu)
	if apdu.SW != "9000" || apdu.Data != "CAFE" || apdu.ATS.FSC != 32 {
		t.Fatalf("apdu = %+v", apdu)
	}

	code, envelope = harness.run("desfire", "info", "--json")
	if code != 0 {
		t.Fatalf("desfire info exit %d: %s", code, harness.stderr.String())
	}
	var info app.DESFireVersionDTO
	decodeResult(t, envelope, &info)
	if info.Vendor != 0x04 || info.Version != "0x12.0x34" {
		t.Fatalf("info = %+v", info)
	}

	code, envelope = harness.run("desfire", "apps", "--json")
	if code != 0 {
		t.Fatalf("desfire apps exit %d: %s", code, harness.stderr.String())
	}
	var apps struct {
		Applications []string `json:"applications"`
	}
	decodeResult(t, envelope, &apps)
	if len(apps.Applications) != 1 || apps.Applications[0] != "000001" {
		t.Fatalf("apps = %+v", apps)
	}

	code, envelope = harness.run("desfire", "read", "000001", "1", "--json")
	if code != 0 {
		t.Fatalf("desfire read exit %d: %s", code, harness.stderr.String())
	}
	var read app.DESFireReadDTO
	decodeResult(t, envelope, &read)
	if read.Hex != "DEADBEEF" {
		t.Fatalf("read = %+v", read)
	}
}

func TestCLIType4NDEFReadAndWrite(t *testing.T) {
	message, _ := type2.EncodeTextMessage("hello", "en")
	harness, _ := newISODEPHarness(t, Type4ResponseHandler(t, message))

	code, envelope := harness.run("ndef", "read", "--json")
	if code != 0 {
		t.Fatalf("ndef read exit %d: %s %s", code, harness.stderr.String(), harness.stdout.String())
	}
	var read struct {
		Family string            `json:"family"`
		Type4  *app.Type4CCDTO   `json:"type4cc"`
		NDEF   *app.Type2NDEFDTO `json:"ndef"`
	}
	decodeResult(t, envelope, &read)
	if read.Family != "type4" || read.Type4 == nil || read.Type4.FileID != "E104" || read.NDEF.Text != "hello" {
		t.Fatalf("read = %+v", read)
	}

	updated, _ := type2.EncodeTextMessage("updated", "en")
	code, envelope = harness.run("ndef", "write", "--hex", hex.EncodeToString(updated), "--yes", "--json")
	if code != 0 {
		t.Fatalf("ndef write exit %d: %s", code, harness.stderr.String())
	}
	var write app.Type4NDEFWriteResultDTO
	decodeResult(t, envelope, &write)
	if write.Backup == "" || write.NDEF == nil || write.NDEF.NDEF == nil || write.NDEF.NDEF.Text != "updated" {
		t.Fatalf("write = %+v", write)
	}
	if _, err := os.Stat(write.Backup); err != nil {
		t.Fatalf("backup: %v", err)
	}

	code, envelope = harness.run("ndef", "read", "--json")
	if code != 0 {
		t.Fatalf("re-read exit %d: %s", code, harness.stderr.String())
	}
	decodeResult(t, envelope, &read)
	if read.NDEF.Text != "updated" {
		t.Fatalf("re-read = %+v", read.NDEF)
	}
}
