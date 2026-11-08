package desktop

import (
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/egoist/mygo/ui"
)

func isoDepFixture() *fakeBackend {
	backend := classicFixture()
	backend.dashboard.Card = app.CardDTO{
		Present: true, UID: "04 11 22 33 44 55 66", UIDLength: 7,
		ATQA: "00 44", SAK: "20", Type: "ISO/IEC 14443-4",
		TypeCode: "iso14443_4", Inferred: true,
	}
	backend.dashboard.Devices[0].APDU = true
	backend.workbench = app.WorkbenchDTO{}
	return backend
}

func waitForParity(t *testing.T, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		if condition() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("condition did not become true")
}

func TestRecoveryTabStartsNestedAfterConfirmation(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("Recovery"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	waitForParity(t, func() bool {
		tester.Frame()
		return tester.HasText("Start Nested (MFOC)")
	})
	if err := tester.Click("Start Nested (MFOC)"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Run Nested (MFOC)?") {
		t.Fatalf("confirmation missing: %q", tester.Texts())
	}
	if err := tester.Click("Start recovery"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.recoveryStarts) == 1 && backend.recoveryStarts[0] == "nested"
	})
}

func TestAPDUTabSendsDiagnosticCommand(t *testing.T) {
	backend := isoDepFixture()
	model := newShell(backend)
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if !tester.HasText("APDU diagnostic") {
		t.Fatalf("APDU tab missing: %q", tester.Texts())
	}
	model.apduInput = "00A4040000"
	tester.Frame()
	if err := tester.Click("Send APDU"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.apduCalls) == 1 && backend.apduCalls[0] == "00A4040000"
	})
	tester.Frame()
	if !tester.HasText("9000") {
		t.Fatalf("APDU response missing: %q", tester.Texts())
	}
}

func TestType4NDEFWriteNeedsConfirmation(t *testing.T) {
	backend := isoDepFixture()
	model := newShell(backend)
	model.type4 = app.Type4NDEFDTO{
		Loaded: true, CardType: "NFC Forum Type 4",
		CC: app.Type4CCDTO{FileID: "E104", MaxMessage: 256},
	}
	model.ndefMode = "raw"
	model.ndefRaw = "D1010A550161"
	tester := ui.NewTester(model.view, 1360, 860)
	if err := tester.Click("NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click("Write NDEF"); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText("Write 6 Type 4 NDEF bytes?") {
		t.Fatalf("Type 4 confirmation missing: %q", tester.Texts())
	}
	if err := tester.Click("Confirm NDEF write"); err != nil {
		t.Fatal(err)
	}
	waitForParity(t, func() bool {
		backend.mu.Lock()
		defer backend.mu.Unlock()
		return len(backend.type4Writes) == 1 && len(backend.type4Writes[0].Message) == 6
	})
}

func TestDiagnosticsModalShowsSelfCheck(t *testing.T) {
	backend := classicFixture()
	model := newShell(backend)
	model.openDiagnostics()
	tester := ui.NewTester(model.view, 1360, 860)
	tester.Frame()
	if !tester.HasText("Diagnostics") || !tester.HasText("Healthy") {
		t.Fatalf("diagnostics modal missing: %q", tester.Texts())
	}
}
