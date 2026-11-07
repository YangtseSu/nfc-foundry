//go:build libnfc && cgo && libnfc_hardware

package workflow_test

import (
	"context"
	"errors"
	"os"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/desfire"
	"github.com/YangtseSu/nfc-foundry/internal/isodep"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/libnfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/type4"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func hardwareISODEPSession(t *testing.T, ctx context.Context) (*libnfc.Reader, nfc.CardInfo, *isodep.Session) {
	t.Helper()
	connString := os.Getenv("LIBNFC_DEVICE")
	if connString == "" {
		t.Fatal("LIBNFC_DEVICE is required")
	}
	reader := libnfc.NewReader()
	if err := reader.Open(ctx, connString); err != nil {
		t.Fatal(err)
	}
	card, err := reader.CardInfo(ctx)
	if err != nil {
		reader.Close()
		t.Fatal(err)
	}
	if nfc.InferCardType(card) != nfc.CardTypeISO14443 {
		reader.Close()
		t.Skipf("card is %s, not ISO/IEC 14443-4", nfc.InferCardType(card))
	}
	transport, ok := any(reader).(nfc.RawTransceiver)
	if !ok {
		reader.Close()
		t.Fatal("reader has no raw transport")
	}
	session, err := isodep.OpenSession(ctx, transport, card)
	if err != nil {
		reader.Close()
		t.Fatal(err)
	}
	return reader, card, session
}

func TestHardwareISODEP(t *testing.T) {
	if os.Getenv("NFCX_WORKFLOW_HARDWARE") != "1" {
		t.Skip("set NFCX_WORKFLOW_HARDWARE=1 to enable the explicit ISO-DEP hardware test")
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	reader, _, session := hardwareISODEPSession(t, ctx)
	defer reader.Close()
	defer session.Close(context.Background())
	t.Logf("ATS: %X FSC=%d FWI=%d", session.ATS.Raw, session.ATS.FSC, session.ATS.FWI)
	apdu, err := isodep.BuildShort(0x00, 0xA4, 0x04, 0x00, []byte{0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01}, 256)
	if err != nil {
		t.Fatal(err)
	}
	response, err := session.Transmit(ctx, apdu)
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("Select NDEF application: SW=%04X data=%X", response.SW(), response.Data)
}

func TestHardwareDESFireRead(t *testing.T) {
	if os.Getenv("NFCX_WORKFLOW_HARDWARE") != "1" {
		t.Skip("set NFCX_WORKFLOW_HARDWARE=1 to enable the explicit DESFire hardware test")
	}
	if os.Getenv("NFCX_HARDWARE_DESFIRE_CONFIRM") != "I_OWN_THIS_CARD" {
		t.Skip("set NFCX_HARDWARE_DESFIRE_CONFIRM=I_OWN_THIS_CARD for the read-only DESFire test")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	reader, _, session := hardwareISODEPSession(t, ctx)
	defer reader.Close()
	defer session.Close(context.Background())
	card := desfire.New(session)
	version, err := card.Identify(ctx)
	if err != nil {
		var status *desfire.StatusError
		if errors.As(err, &status) && status.SW2 == 0x1C {
			t.Skip("card does not answer the DESFire GetVersion command")
		}
		t.Fatal(err)
	}
	t.Logf("DESFire version: %+v", version)
	applications, err := card.Applications(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, aid := range applications {
		files, err := card.Files(ctx, aid)
		if err != nil {
			t.Fatalf("application %06X: %v", aid, err)
		}
		t.Logf("application %06X: %d file(s)", aid, len(files))
		for _, file := range files {
			t.Logf("  file %02X: %+v", file.ID, file)
			if file.Type != desfire.FileStandard && file.Type != desfire.FileBackup {
				continue
			}
			if file.Communication != 0 {
				t.Logf("  file %02X is protected; skipping read", file.ID)
				continue
			}
			data, err := card.ReadFile(ctx, aid, file.ID)
			if err != nil {
				t.Fatalf("read %06X/%02X: %v", aid, file.ID, err)
			}
			t.Logf("  file %02X data: %X", file.ID, data)
		}
	}
}

func TestHardwareType4NDEF(t *testing.T) {
	if os.Getenv("NFCX_WORKFLOW_HARDWARE") != "1" {
		t.Skip("set NFCX_WORKFLOW_HARDWARE=1 to enable the explicit Type 4 hardware test")
	}
	if os.Getenv("NFCX_HARDWARE_TYPE4_CONFIRM") != "I_OWN_THIS_TAG" {
		t.Skip("set NFCX_HARDWARE_TYPE4_CONFIRM=I_OWN_THIS_TAG for the Type 4 NDEF test")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	reader, card, session := hardwareISODEPSession(t, ctx)
	defer reader.Close()
	defer session.Close(context.Background())
	tag := type4.New(session)
	document, err := tag.Read(ctx)
	if err != nil {
		var status *type4.StatusError
		if errors.As(err, &status) && (status.SW2 == 0x82 || status.SW2 == 0xD6) {
			t.Skip("card has no NFC Forum Type 4 NDEF application")
		}
		t.Fatal(err)
	}
	t.Logf("Type 4 CC: %+v", document.CC)
	if document.NDEF != nil && document.NDEF.Present {
		t.Logf("existing NDEF: %s", document.NDEF.Summary)
	}
	if os.Getenv("NFCX_HARDWARE_TYPE4_WRITE") != "I_OWN_THIS_TAG" {
		return
	}
	backupRoot := os.Getenv("NFCX_TEST_TYPE4_BACKUP_DIR")
	if backupRoot == "" {
		t.Fatal("NFCX_TEST_TYPE4_BACKUP_DIR is required for the Type 4 write test")
	}
	message, err := type2.EncodeTextMessage("NFC Foundry Type 4 hardware test", "en")
	if err != nil {
		t.Fatal(err)
	}
	backupPath, err := workflow.FileType4BackupStore{Root: backupRoot}.Save(ctx, workflow.Type4Backup{
		Card: card, CC: document.CC, Message: document.Message,
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("Type 4 backup: %s", backupPath)
	result, err := tag.Write(ctx, message)
	if err != nil {
		t.Fatalf("write failed after backing up %X: %v", document.Message, err)
	}
	if result.Final.NDEF == nil || !result.Final.NDEF.Present {
		t.Fatalf("final NDEF is empty")
	}
	t.Logf("wrote Type 4 NDEF: %s", result.Final.NDEF.Summary)
}
