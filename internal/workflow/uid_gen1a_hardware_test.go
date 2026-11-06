//go:build libnfc && cgo && libnfc_hardware

package workflow_test

import (
	"bytes"
	"context"
	"encoding/hex"
	"os"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/libnfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func TestHardwareGen1AUIDWrite(t *testing.T) {
	if os.Getenv("NFCX_WORKFLOW_HARDWARE") != "1" {
		t.Skip("set NFCX_WORKFLOW_HARDWARE=1 to enable the explicit Gen1A hardware test")
	}
	if os.Getenv("NFCX_HARDWARE_GEN1A_CONFIRM") != "I_OWN_THIS_CARD" {
		t.Skip("set NFCX_HARDWARE_GEN1A_CONFIRM=I_OWN_THIS_CARD to authorize the Gen1A UID write")
	}
	connString := os.Getenv("LIBNFC_DEVICE")
	if connString == "" {
		t.Fatal("LIBNFC_DEVICE is required")
	}
	value := os.Getenv("NFCX_TEST_UID")
	newUID, decodeErr := hex.DecodeString(value)
	if decodeErr != nil || len(newUID) != 4 {
		t.Fatalf("NFCX_TEST_UID must be exactly 8 hexadecimal digits: %v", decodeErr)
	}
	keyA := hardwareKey(t, "NFCX_TEST_KEY_A", "FFFFFFFFFFFF")

	backupRoot := os.Getenv("NFCX_TEST_UID_BACKUP_DIR")
	if backupRoot == "" {
		t.Fatal("NFCX_TEST_UID_BACKUP_DIR is required so the block 0 backup survives the test")
	}

	reader := libnfc.NewReader()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	if err := reader.Open(ctx, connString); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	card, err := reader.CardInfo(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if nfc.InferCardType(card) != nfc.CardTypeMIFAREClassic1K || len(card.UID) != 4 {
		t.Fatalf("Gen1A test requires a 4-byte UID MIFARE Classic 1K card, got UID %x SAK %#02x", card.UID, card.SAK)
	}
	if bytes.Equal(card.UID, newUID) {
		t.Fatalf("NFCX_TEST_UID %x is already the current UID", newUID)
	}

	service := workflow.NewUIDService(directExecutor{reader: reader}, workflow.FileUIDBackupStore{Root: backupRoot})
	result, writeErr := service.Write(ctx, workflow.UIDWriteRequest{
		TaskID: "hardware-gen1a", Card: card, Device: nfc.DeviceInfo{ConnString: connString},
		NewUID: newUID, Sector0Keys: workflow.SectorKeys{Sector: 0, KeyA: &keyA},
	}, func(event workflow.UIDWriteEvent) {
		t.Logf("%s: %s", event.Phase, event.Message)
	})
	if writeErr != nil {
		t.Fatalf("Gen1A write failed before the original block 0 was restored; backup: %s: %v", result.BackupPath, writeErr)
	}
	if !result.Verified || result.BackupPath == "" {
		t.Fatalf("Gen1A write was not verified; result = %+v", result)
	}
	t.Logf("UID %x -> %x; original block 0 backed up to %s", result.OldCard.UID, result.NewCard.UID, result.BackupPath)

	restoreCtx, restoreCancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer restoreCancel()
	current, err := reader.CardInfo(restoreCtx)
	if err != nil || !nfc.SameCard(result.NewCard, current) {
		t.Fatalf("IMPORTANT: card identity changed before restore; backup is %s: %v", result.BackupPath, err)
	}
	if err := mifare.WriteGen1ABlock0(restoreCtx, reader, current, result.OldBlock0); err != nil {
		t.Fatalf("IMPORTANT: failed to restore the original block 0; backup is %s: %v", result.BackupPath, err)
	}
	deadline := time.Now().Add(8 * time.Second)
	for {
		current, err := reader.CardInfo(restoreCtx)
		if err == nil && bytes.Equal(current.UID, result.OldCard.UID) {
			card = current
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("IMPORTANT: original UID did not return after restore; backup is %s", result.BackupPath)
		}
		time.Sleep(200 * time.Millisecond)
	}
	if err := reader.Authenticate(restoreCtx, 0, nfc.KeyTypeA, keyA); err != nil {
		t.Fatalf("IMPORTANT: original block 0 was written but sector 0 authentication failed: %v", err)
	}
	actual, err := reader.ReadBlock(restoreCtx, 0)
	if err != nil || actual != result.OldBlock0 {
		t.Fatalf("IMPORTANT: original block 0 readback = %x, %v; want %x", actual, err, result.OldBlock0)
	}
	t.Logf("original UID and block 0 restored and verified on the same reader")
}
