//go:build libnfc && cgo && libnfc_hardware

package workflow_test

import (
	"bytes"
	"context"
	"encoding/hex"
	"os"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/libnfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func hardwareType2Password(t *testing.T) []byte {
	t.Helper()
	value := os.Getenv("NFCX_TEST_TYPE2_PWD")
	if value == "" {
		return nil
	}
	decoded, err := hex.DecodeString(value)
	if err != nil || len(decoded) != 4 {
		t.Fatalf("NFCX_TEST_TYPE2_PWD must be exactly 8 hexadecimal digits: %v", err)
	}
	return decoded
}

func hardwareType2TextRecord(text string) []byte {
	payload := append([]byte{0x02, 'e', 'n'}, []byte(text)...)
	return append([]byte{0xD1, 0x01, byte(len(payload)), 'T'}, payload...)
}

func TestHardwareType2ReadWrite(t *testing.T) {
	if os.Getenv("NFCX_WORKFLOW_HARDWARE") != "1" {
		t.Skip("set NFCX_WORKFLOW_HARDWARE=1 to enable the explicit Type 2 hardware test")
	}
	connString := os.Getenv("LIBNFC_DEVICE")
	if connString == "" {
		t.Fatal("LIBNFC_DEVICE is required")
	}
	writeConfirmed := os.Getenv("NFCX_HARDWARE_TYPE2_CONFIRM") == "I_OWN_THIS_TAG"
	backupRoot := os.Getenv("NFCX_TEST_TYPE2_BACKUP_DIR")
	if writeConfirmed && backupRoot == "" {
		t.Fatal("NFCX_TEST_TYPE2_BACKUP_DIR is required so the user-memory backup survives the write test")
	}
	password := hardwareType2Password(t)

	reader := libnfc.NewReader()
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()
	if err := reader.Open(ctx, connString); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	card, err := reader.CardInfo(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if nfc.InferCardType(card) != nfc.CardTypeType2 {
		t.Fatalf("Type 2 test requires a tag with ATQA % X and SAK 00, got UID % X ATQA % X SAK %02X",
			[2]byte{0x00, 0x44}, card.UID, card.ATQA, card.SAK)
	}

	service := workflow.NewType2Service(directExecutor{reader: reader}, workflow.FileType2BackupStore{Root: backupRoot})
	document, err := service.Read(ctx, workflow.Type2ReadRequest{Card: card, Password: password})
	if err != nil {
		t.Fatal(err)
	}
	geometry, _ := document.Geometry()
	t.Logf("product=%s pages=%d user=%d bytes cc=%s ndef=%v",
		document.ProductName, len(document.Pages), geometry.UserBytes(), document.CC.Raw, document.NDEF != nil)
	if document.NDEFError != "" {
		t.Logf("NDEF state: %s", document.NDEFError)
	}
	if !writeConfirmed {
		t.Log("read-only pass; set NFCX_HARDWARE_TYPE2_CONFIRM=I_OWN_THIS_TAG to exercise NDEF write and restore")
		return
	}

	message := hardwareType2TextRecord("NFC Foundry Type 2 hardware test")
	written, err := service.WriteNDEF(ctx, workflow.Type2NDEFWriteRequest{Card: card, Message: message, Password: password})
	if err != nil {
		t.Fatalf("NDEF write failed; backup: %s: %v", written.BackupPath, err)
	}
	if written.BackupPath == "" || len(written.Written) == 0 || len(written.Verified) != len(written.Written) {
		t.Fatalf("NDEF write result = %+v", written)
	}
	t.Logf("NDEF written and verified on pages %v; backup %s", written.Written, written.BackupPath)
	if written.Document.NDEF == nil || written.Document.NDEF.Text != "NFC Foundry Type 2 hardware test" {
		t.Fatalf("NDEF readback = %+v", written.Document.NDEF)
	}

	restored, err := service.Restore(ctx, workflow.Type2RestoreRequest{Card: card, Document: document, Password: password})
	if err != nil {
		t.Fatalf("IMPORTANT: restore failed; backup: %s: %v", restored.BackupPath, err)
	}
	if restored.BackupPath == "" {
		t.Fatal("restore did not create a backup")
	}
	original, err := document.UserMemory()
	if err != nil {
		t.Fatal(err)
	}
	final, err := service.Read(ctx, workflow.Type2ReadRequest{Card: card, Password: password})
	if err != nil {
		t.Fatalf("IMPORTANT: final read failed after restore; backup: %s: %v", restored.BackupPath, err)
	}
	restoredMemory, err := final.UserMemory()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(original, restoredMemory) {
		t.Fatalf("IMPORTANT: user memory differs after restore; backup: %s", restored.BackupPath)
	}
	t.Logf("original user memory restored and verified; restore backup %s", restored.BackupPath)
}
