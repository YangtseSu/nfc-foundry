package workflow

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

type type2Executor struct{ reader nfc.Reader }

func (e type2Executor) WithReader(ctx context.Context, operation func(context.Context, nfc.Reader) error) error {
	return operation(ctx, e.reader)
}

func type2TestTag(t *testing.T) (*mock.Reader, *mock.Type2Tag) {
	t.Helper()
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	reader := mock.NewType2Reader(tag)
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reader.Close() })
	return reader, tag
}

func type2TextRecord(text string) []byte {
	payload := append([]byte{0x02, 'e', 'n'}, []byte(text)...)
	return append([]byte{0xD1, 0x01, byte(len(payload)), 'T'}, payload...)
}

func TestType2WriteNDEFBacksUpAndVerifies(t *testing.T) {
	reader, tag := type2TestTag(t)
	backupRoot := t.TempDir()
	service := NewType2Service(type2Executor{reader: reader}, FileType2BackupStore{Root: backupRoot})
	card := tag.CardInfo()
	result, err := service.WriteNDEF(context.Background(), Type2NDEFWriteRequest{
		Card: card, Message: type2TextRecord("hello"),
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.BackupPath == "" || len(result.Written) == 0 || len(result.Verified) != len(result.Written) {
		t.Fatalf("result = %+v", result)
	}
	data, err := os.ReadFile(result.BackupPath)
	if err != nil {
		t.Fatal(err)
	}
	var backup type2.Document
	if err := json.Unmarshal(data, &backup); err != nil {
		t.Fatal(err)
	}
	if backup.Product != type2.ProductNTAG213 {
		t.Fatalf("backup product = %q", backup.Product)
	}
	if result.Document.NDEF == nil || result.Document.NDEF.Text != "hello" {
		t.Fatalf("NDEF = %+v (error %q)", result.Document.NDEF, result.Document.NDEFError)
	}
	if filepath.Dir(result.BackupPath) != backupRoot {
		t.Fatalf("backup path = %q", result.BackupPath)
	}
}

func TestType2WriteRejectsProtectedPageWithoutConfirmation(t *testing.T) {
	reader, tag := type2TestTag(t)
	service := NewType2Service(type2Executor{reader: reader}, FileType2BackupStore{Root: t.TempDir()})
	card := tag.CardInfo()
	_, err := service.Write(context.Background(), Type2WriteRequest{
		Card:   card,
		Writes: []type2.PageWrite{{Page: 41, Hex: "04000000", Region: "config"}},
	})
	if !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("protected write error = %v", err)
	}
	page, _ := tag.Page(41)
	if page != [4]byte{0x04, 0x00, 0x00, 0xFF} {
		t.Fatalf("protected page changed: % X", page)
	}
}

func TestType2WriteReportsVerificationFailure(t *testing.T) {
	reader, tag := type2TestTag(t)
	tag.CorruptWritePage = 4
	service := NewType2Service(type2Executor{reader: reader}, FileType2BackupStore{Root: t.TempDir()})
	result, err := service.Write(context.Background(), Type2WriteRequest{
		Card:   tag.CardInfo(),
		Writes: []type2.PageWrite{{Page: 4, Hex: "DEADBEEF", Region: "user"}},
	})
	if !errors.Is(err, nfc.ErrVerificationFailed) {
		t.Fatalf("verification error = %v", err)
	}
	if len(result.Written) != 1 || len(result.Verified) != 0 || result.BackupPath == "" {
		t.Fatalf("result = %+v", result)
	}
}

func TestType2WriteAbortsOnCardChange(t *testing.T) {
	reader, tag := type2TestTag(t)
	calls := 0
	reader = mock.NewReaderWithFuncs(mock.ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) {
			calls++
			card := tag.CardInfo()
			if calls > 1 {
				card.UID[6] ^= 0xFF
			}
			return card, nil
		},
		SetRawMode:      tag.SetRawMode,
		TransceiveBytes: tag.TransceiveBytes,
	})
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	service := NewType2Service(type2Executor{reader: reader}, FileType2BackupStore{Root: t.TempDir()})
	_, err := service.Write(context.Background(), Type2WriteRequest{
		Card:   tag.CardInfo(),
		Writes: []type2.PageWrite{{Page: 4, Hex: "01020304", Region: "user"}},
	})
	if !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("card-change error = %v", err)
	}
}

func TestType2RestoreWritesProtectedPages(t *testing.T) {
	reader, tag := type2TestTag(t)
	service := NewType2Service(type2Executor{reader: reader}, FileType2BackupStore{Root: t.TempDir()})
	card := tag.CardInfo()
	document, err := service.Read(context.Background(), Type2ReadRequest{Card: card})
	if err != nil {
		t.Fatal(err)
	}
	document.Pages[4] = "AABBCCDD"
	document.Pages[41] = "04000000"
	result, err := service.Restore(context.Background(), Type2RestoreRequest{
		Card: card, Document: document, IncludeProtected: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.BackupPath == "" {
		t.Fatal("restore did not create a backup")
	}
	page4, _ := tag.Page(4)
	page41, _ := tag.Page(41)
	if page4 != [4]byte{0xAA, 0xBB, 0xCC, 0xDD} || page41 != [4]byte{0x04, 0x00, 0x00, 0x00} {
		t.Fatalf("restored pages = % X, % X", page4, page41)
	}
	found := false
	for _, page := range result.Written {
		if page == 41 {
			found = true
		}
	}
	if !found {
		t.Fatalf("config page was not written: %+v", result)
	}
}

func TestType2WriteRequiresBackupStore(t *testing.T) {
	reader, tag := type2TestTag(t)
	service := NewType2Service(type2Executor{reader: reader}, nil)
	_, err := service.Write(context.Background(), Type2WriteRequest{
		Card:   tag.CardInfo(),
		Writes: []type2.PageWrite{{Page: 4, Hex: "01020304", Region: "user"}},
	})
	if !errors.Is(err, nfc.ErrUnsupported) {
		t.Fatalf("missing backup error = %v", err)
	}
	page, _ := tag.Page(4)
	if page != [4]byte{} {
		t.Fatalf("tag was modified without a backup: % X", page)
	}
}

func TestType2RejectsNonType2Card(t *testing.T) {
	service := NewType2Service(type2Executor{reader: mock.NewReader()}, nil)
	_, err := service.Read(context.Background(), Type2ReadRequest{
		Card: nfc.CardInfo{UID: []byte{1, 2, 3, 4}, ATQA: [2]byte{0, 4}, SAK: 0x08},
	})
	if !errors.Is(err, nfc.ErrUnsupported) {
		t.Fatalf("non-Type 2 error = %v", err)
	}
}

func TestType2WriteDryRunDoesNotModifyTag(t *testing.T) {
	reader, tag := type2TestTag(t)
	service := NewType2Service(type2Executor{reader: reader}, FileType2BackupStore{Root: t.TempDir()})
	result, err := service.Write(context.Background(), Type2WriteRequest{
		Card:   tag.CardInfo(),
		Writes: []type2.PageWrite{{Page: 4, Hex: "01020304", Region: "user"}},
		DryRun: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Planned) != 1 || result.Planned[0] != 4 || len(result.Written) != 0 || result.BackupPath != "" {
		t.Fatalf("dry-run result = %+v", result)
	}
	page, _ := tag.Page(4)
	if page != [4]byte{} {
		t.Fatalf("dry run modified page 4: % X", page)
	}
}
