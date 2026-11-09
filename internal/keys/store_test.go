package keys

import (
	"bytes"
	"path/filepath"
	"strings"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func TestParseDictionaryNormalizesSpacesCaseAndDuplicates(t *testing.T) {
	report, err := ParseDictionary(strings.NewReader(" ffffffffffff \nFF FF FF FF FF FF\na0a1a2a3a4a5 # default\nnot-a-key\n"))
	if err != nil {
		t.Fatal(err)
	}
	if report.Valid != 2 || report.Duplicates != 1 || len(report.Issues) != 1 || report.Issues[0].Line != 4 {
		t.Fatalf("unexpected report: %+v", report)
	}
}

func TestAnalyzeDictionaryReportsEachLine(t *testing.T) {
	analysis, err := AnalyzeDictionary(strings.NewReader("# comment\nffffffffffff\nFF FF FF FF FF FF\nnope\n\n"))
	if err != nil {
		t.Fatal(err)
	}
	if len(analysis.Lines) != 5 {
		t.Fatalf("lines = %d", len(analysis.Lines))
	}
	if !analysis.Lines[0].Comment || !analysis.Lines[0].Ignored {
		t.Fatalf("comment line = %+v", analysis.Lines[0])
	}
	if analysis.Lines[1].Key == nil || analysis.Lines[1].Problem != "" {
		t.Fatalf("valid line = %+v", analysis.Lines[1])
	}
	if !analysis.Lines[2].Duplicate || analysis.Lines[2].Key != nil {
		t.Fatalf("duplicate line = %+v", analysis.Lines[2])
	}
	if analysis.Lines[3].Problem != "key must be 12 hexadecimal digits" {
		t.Fatalf("invalid line = %+v", analysis.Lines[3])
	}
	if !analysis.Lines[4].Ignored || analysis.Report.Ignored != 2 {
		t.Fatalf("blank line/ignored = %+v/%d", analysis.Lines[4], analysis.Report.Ignored)
	}
	if analysis.Report.Valid != 1 || analysis.Report.Duplicates != 1 || len(analysis.Report.Issues) != 1 {
		t.Fatalf("report = %+v", analysis.Report)
	}
}

func TestMergeVerifiedDoesNotOverwriteConflict(t *testing.T) {
	store := NewStore()
	cardID := "01020304/0400/08"
	// Keep these fixtures outside the built-in dictionary: this test exercises
	// the conflict path, where the recovered value must not be added at all.
	original := nfc.Key{0x9A, 0x8B, 0x7C, 0x6D, 0x5E, 0x4F}
	recovered := nfc.Key{0x1F, 0x2E, 0x3D, 0x4C, 0x5B, 0x6A}
	record, _, err := store.Add(original, SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Verify(cardID, 2, nfc.KeyTypeB, record.ID); err != nil {
		t.Fatal(err)
	}
	merged, err := store.MergeVerified(cardID, 2, nfc.KeyTypeB, recovered, SourceAttackEngine)
	if err != nil || merged.Status != VerificationConflict || merged.ExistingKeyID != record.ID {
		t.Fatalf("merge = %+v, err = %v", merged, err)
	}
	matches := store.Verified(cardID)
	if len(matches) != 1 || matches[0].KeyID != record.ID {
		t.Fatalf("verification was overwritten: %+v", matches)
	}
	for _, candidate := range store.Entries() {
		if candidate.Value == recovered {
			t.Fatal("conflicting candidate was added to global catalog")
		}
	}
}

func TestParseKeyRejectsNonTwelveDigitHex(t *testing.T) {
	for _, value := range []string{"", "FFFFFFFFFF", "FFFFFFFFFFFFFF", "GGGGGGGGGGGG"} {
		if _, err := ParseKey(value); err == nil {
			t.Fatalf("ParseKey(%q) succeeded", value)
		}
	}
}

func TestCandidatesPromoteKnownSuccess(t *testing.T) {
	store := NewStore()
	custom := nfc.Key{1, 2, 3, 4, 5, 6}
	record, _, err := store.Add(custom, SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Verify("card", 0, nfc.KeyTypeA, record.ID); err != nil {
		t.Fatal(err)
	}
	if got := store.Candidates("card", 1, nfc.KeyTypeB)[0].ID; got != record.ID {
		t.Fatalf("promoted candidate = %q, want %q", got, record.ID)
	}
}

func TestExportIncludesAllKnownKeys(t *testing.T) {
	store := NewStore()
	custom := nfc.Key{1, 2, 3, 4, 5, 6}
	if _, _, err := store.Add(custom, SourceFileImport); err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	count, err := store.Export(&output)
	if err != nil {
		t.Fatal(err)
	}
	if count != len(store.Entries()) {
		t.Fatalf("export count = %d, want %d", count, len(store.Entries()))
	}
	if !strings.Contains(output.String(), "010203040506\n") || !strings.Contains(output.String(), "FFFFFFFFFFFF\n") {
		t.Fatalf("export omitted a custom or built-in key: %q", output.String())
	}
}

func TestSaveAndLoadPreserveCardScopedVerifications(t *testing.T) {
	store := NewStore()
	custom := nfc.Key{1, 2, 3, 4, 5, 6}
	record, _, err := store.Add(custom, SourceUserInput)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Verify("01020304/0400/08", 3, nfc.KeyTypeB, record.ID); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "keys.json")
	if err := store.Save(path); err != nil {
		t.Fatal(err)
	}
	loaded := NewStore()
	if err := loaded.Load(path); err != nil {
		t.Fatal(err)
	}
	matches := loaded.Verified("01020304/0400/08")
	if len(matches) != 1 || matches[0].KeyID != record.ID || matches[0].Sector != 3 || matches[0].KeyType != nfc.KeyTypeB {
		t.Fatalf("loaded verifications = %+v", matches)
	}
	if other := loaded.Verified("AABBCCDD/0400/08"); len(other) != 0 {
		t.Fatalf("verification leaked to another card: %+v", other)
	}
}

func TestUnverifyRemovesOnlyTheRequestedSlot(t *testing.T) {
	store := NewStore()
	key := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	record, _, err := store.Add(key, SourceBuiltin)
	if err != nil {
		t.Fatal(err)
	}
	cardID := "01020304/0400/08"
	if err := store.Verify(cardID, 5, nfc.KeyTypeA, record.ID); err != nil {
		t.Fatal(err)
	}
	if err := store.Verify(cardID, 5, nfc.KeyTypeB, record.ID); err != nil {
		t.Fatal(err)
	}
	store.Unverify(cardID, 5, nfc.KeyTypeA)
	matches := store.Verified(cardID)
	if len(matches) != 1 || matches[0].KeyType != nfc.KeyTypeB || matches[0].Sector != 5 {
		t.Fatalf("remaining verifications = %+v", matches)
	}
	if _, ok := store.Record(record.ID); !ok {
		t.Fatal("unverify deleted the key record")
	}
}
