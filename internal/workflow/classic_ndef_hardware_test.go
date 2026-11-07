//go:build libnfc && cgo && libnfc_hardware

package workflow_test

import (
	"context"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/libnfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func TestHardwareClassicNDEF(t *testing.T) {
	if os.Getenv("NFCX_WORKFLOW_HARDWARE") != "1" {
		t.Skip("set NFCX_WORKFLOW_HARDWARE=1 to enable the explicit Classic NDEF hardware test")
	}
	if os.Getenv("NFCX_HARDWARE_CLASSIC_NDEF_CONFIRM") != "I_OWN_THIS_CARD" {
		t.Skip("set NFCX_HARDWARE_CLASSIC_NDEF_CONFIRM=I_OWN_THIS_CARD to write the test card")
	}
	connString := os.Getenv("LIBNFC_DEVICE")
	if connString == "" {
		t.Fatal("LIBNFC_DEVICE is required")
	}
	backupRoot := os.Getenv("NFCX_TEST_CLASSIC_NDEF_BACKUP_DIR")
	if backupRoot == "" {
		t.Fatal("NFCX_TEST_CLASSIC_NDEF_BACKUP_DIR is required so the backup survives the test")
	}
	keyA := hardwareKey(t, "NFCX_TEST_KEY_A", "FFFFFFFFFFFF")
	var keyB *nfc.Key
	if value := os.Getenv("NFCX_TEST_KEY_B"); value != "" {
		parsed := hardwareKey(t, "NFCX_TEST_KEY_B", "")
		keyB = &parsed
	}

	reader := libnfc.NewReader()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	if err := reader.Open(ctx, connString); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	card, err := reader.CardInfo(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if nfc.InferCardType(card) != nfc.CardTypeMIFAREClassic1K && nfc.InferCardType(card) != nfc.CardTypeMIFAREClassic4K {
		t.Fatalf("Classic NDEF test requires a Classic 1K/4K card, got SAK %02X", card.SAK)
	}
	layout, _ := func() (mifare.Layout, error) {
		if nfc.InferCardType(card) == nfc.CardTypeMIFAREClassic4K {
			return mifare.Classic4K, nil
		}
		return mifare.Classic1K, nil
	}()
	sectors, _ := layout.SectorCount()
	keys := make([]workflow.SectorKeys, sectors)
	for sector := range keys {
		keys[sector] = workflow.SectorKeys{Sector: sector, KeyA: &keyA, KeyB: keyB}
	}
	service := workflow.NewClassicNDEFService(
		directExecutor{reader: reader},
		workflow.NewDumpService(directExecutor{reader: reader}),
		workflow.FileClassicNDEFBackupStore{Root: backupRoot},
	)
	state, err := service.Read(ctx, workflow.ClassicNDEFRequest{Card: card, Keys: keys})
	if err != nil {
		t.Fatal(err)
	}
	if !state.Application {
		if os.Getenv("NFCX_HARDWARE_CLASSIC_NDEF_FORMAT") != "I_OWN_THIS_CARD" {
			t.Skip("card has no NDEF application; set NFCX_HARDWARE_CLASSIC_NDEF_FORMAT=I_OWN_THIS_CARD and NFCX_HARDWARE_CLASSIC_NDEF_SECTORS=1-2 to format it")
		}
		targets := hardwareSectorRange(t, os.Getenv("NFCX_HARDWARE_CLASSIC_NDEF_SECTORS"))
		result, err := service.Format(ctx, workflow.ClassicNDEFFormatRequest{
			Card: card, Device: nfc.DeviceInfo{ConnString: connString}, Keys: keys, Sectors: targets,
		})
		if err != nil {
			t.Fatalf("format stopped after verified blocks %v: %v", result.Verified, err)
		}
		t.Logf("formatted sectors %v; backup %s", targets, result.BackupPath)
		state, err = service.Read(ctx, workflow.ClassicNDEFRequest{Card: card, Keys: keys})
		if err != nil {
			t.Fatal(err)
		}
		if !state.Application {
			t.Fatal("formatted card still has no NDEF application")
		}
	}
	message, err := type2.EncodeTextMessage("NFC Foundry Classic NDEF "+time.Now().UTC().Format(time.RFC3339), "en")
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.Write(ctx, workflow.ClassicNDEFWriteRequest{
		Card: card, Device: nfc.DeviceInfo{ConnString: connString}, Keys: keys, Message: message,
	})
	if err != nil {
		t.Fatalf("write stopped after verified blocks %v; backup %s: %v", result.Verified, result.BackupPath, err)
	}
	final, err := service.Read(ctx, workflow.ClassicNDEFRequest{Card: card, Keys: keys})
	if err != nil {
		t.Fatal(err)
	}
	if final.NDEF == nil || !final.NDEF.Present || final.NDEF.Length != len(message) {
		t.Fatalf("final NDEF state = %+v", final.NDEF)
	}
	t.Logf("wrote %d NDEF bytes to %X; backup %s", len(message), card.UID, result.BackupPath)
}

func hardwareSectorRange(t *testing.T, value string) []int {
	t.Helper()
	startText, endText, ok := strings.Cut(value, "-")
	if !ok {
		startText, endText = value, value
	}
	start, err := strconv.Atoi(strings.TrimSpace(startText))
	if err != nil {
		t.Fatalf("NFCX_HARDWARE_CLASSIC_NDEF_SECTORS must be N or N-M: %v", err)
	}
	end, err := strconv.Atoi(strings.TrimSpace(endText))
	if err != nil || end < start {
		t.Fatal("NFCX_HARDWARE_CLASSIC_NDEF_SECTORS must be an ascending N-M range")
	}
	result := make([]int, 0, end-start+1)
	for sector := start; sector <= end; sector++ {
		result = append(result, sector)
	}
	return result
}
