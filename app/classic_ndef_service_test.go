package app

import "testing"

func TestClassicCardReadLoadsNDEFState(t *testing.T) {
	_, bindings, _, recorder, _ := newClassicWriteAppService(t, "")
	if _, err := bindings.StartReadCard(ReadCardStartRequestDTO{}); err != nil {
		t.Fatal(err)
	}
	if event := recorder.waitTask(t); event.Type != TaskEventCompleted {
		t.Fatalf("read task event = %+v", event)
	}
	event := recorder.findTaskEvent(t, "read result loaded")
	if event.ClassicNDEF == nil {
		t.Fatal("read task did not include the Classic NDEF state")
	}
	if !event.ClassicNDEF.Loaded || event.ClassicNDEF.Layout != "MIFARE Classic 1K" {
		t.Fatalf("Classic NDEF state = %+v", event.ClassicNDEF)
	}
	if event.ClassicNDEF.MAD.Present {
		t.Fatalf("blank fixture unexpectedly has a MAD: %+v", event.ClassicNDEF.MAD)
	}
}
