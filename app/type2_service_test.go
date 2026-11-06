package app

import (
	"context"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

type appEventRecorder struct {
	mu         sync.Mutex
	taskEvents []TaskEventDetailDTO
	taskCh     chan TaskEventDetailDTO
}

func newAppEventRecorder() *appEventRecorder {
	return &appEventRecorder{taskCh: make(chan TaskEventDetailDTO, 16)}
}

func (r *appEventRecorder) Emit(name string, payload any) {
	switch event := payload.(type) {
	case TaskEventDetailDTO:
		r.mu.Lock()
		r.taskEvents = append(r.taskEvents, event)
		r.mu.Unlock()
		if event.Type == TaskEventCompleted || event.Type == "failed" {
			select {
			case r.taskCh <- event:
			default:
			}
		}
	}
}

func (r *appEventRecorder) findTaskEvent(t *testing.T, fragment string) TaskEventDetailDTO {
	t.Helper()
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, event := range r.taskEvents {
		if strings.Contains(event.Message, fragment) {
			return event
		}
	}
	t.Fatalf("task event containing %q not found in %+v", fragment, r.taskEvents)
	return TaskEventDetailDTO{}
}

func (r *appEventRecorder) waitTask(t *testing.T) TaskEventDetailDTO {
	t.Helper()
	select {
	case event := <-r.taskCh:
		return event
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for the Type 2 task")
		return TaskEventDetailDTO{}
	}
}

func newType2AppService(t *testing.T) (*Service, *mock.Type2Tag, *appEventRecorder) {
	t.Helper()
	dir := t.TempDir()
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	reader := mock.NewType2Reader(tag)
	enumerator := mock.NewEnumerator([]nfc.DeviceInfo{{Name: "Mock Type 2 reader", ConnString: "mock:type2"}}, nil)
	recorder := newAppEventRecorder()
	service := NewServiceWithOptions(Options{
		Emitter:    recorder,
		Enumerator: enumerator,
		NewReader:  func() nfc.Reader { return reader },
		DeviceConfig: device.Config{
			PollInterval: 2 * time.Millisecond, PollTimeout: 50 * time.Millisecond,
			OpenTimeout: time.Second, RemovalMisses: 2,
			ReconnectBackoffs: []time.Duration{time.Millisecond}, LockDir: filepath.Join(dir, "locks"),
		},
		KeyStorePath:    filepath.Join(dir, "keys.json"),
		UIDBackupRoot:   filepath.Join(dir, "uid-backups"),
		Type2BackupRoot: filepath.Join(dir, "type2-backups"),
	})
	t.Cleanup(service.Shutdown)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := service.ConnectDevice("mock:type2"); err != nil {
		t.Fatal(err)
	}
	if _, err := service.WaitForCard(ctx); err != nil {
		t.Fatal(err)
	}
	return service, tag, recorder
}

func TestServiceType2PageWriteTask(t *testing.T) {
	service, tag, recorder := newType2AppService(t)
	if _, err := service.StartType2Write(Type2WriteStartRequestDTO{
		Pages: []Type2PageWriteDTO{{Page: 4, Hex: "DEADBEEF"}},
	}); err != nil {
		t.Fatal(err)
	}
	event := recorder.waitTask(t)
	if event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	if detail := recorder.findTaskEvent(t, "verified"); detail.Progress != 100 {
		t.Fatalf("detail event = %+v", detail)
	}
	page, _ := tag.Page(4)
	if page != [4]byte{0xDE, 0xAD, 0xBE, 0xEF} {
		t.Fatalf("page 4 = % X", page)
	}
	snapshot := service.Type2Snapshot()
	if !snapshot.Loaded || snapshot.Pages[4].Hex != "DEADBEEF" || snapshot.Pages[4].Hex == "--------" {
		t.Fatalf("snapshot page 4 = %+v", snapshot.Pages[4])
	}
}

func TestServiceType2NDEFWriteTask(t *testing.T) {
	service, _, recorder := newType2AppService(t)
	if _, err := service.StartType2NDEFWrite(Type2NDEFWriteStartRequestDTO{Mode: "text", Text: "hello"}); err != nil {
		t.Fatal(err)
	}
	event := recorder.waitTask(t)
	if event.Type != TaskEventCompleted {
		t.Fatalf("task event = %+v", event)
	}
	snapshot := service.Type2Snapshot()
	if snapshot.NDEF == nil || snapshot.NDEF.Text != "hello" {
		t.Fatalf("snapshot NDEF = %+v", snapshot.NDEF)
	}
	if detail := recorder.findTaskEvent(t, "backup"); detail.Progress != 100 {
		t.Fatalf("detail event = %+v", detail)
	}
}

func TestServiceType2WriteFailureReportsBackup(t *testing.T) {
	service, tag, recorder := newType2AppService(t)
	tag.FailWritePage = 5
	if _, err := service.StartType2Write(Type2WriteStartRequestDTO{
		Pages: []Type2PageWriteDTO{{Page: 5, Hex: "01020304"}},
	}); err != nil {
		t.Fatal(err)
	}
	event := recorder.waitTask(t)
	if event.Type != "failed" || !strings.Contains(event.Message, "full-image backup") {
		t.Fatalf("task event = %+v", event)
	}
}

func TestEncodeType2NDEFValidation(t *testing.T) {
	if _, err := encodeType2NDEF(Type2NDEFWriteStartRequestDTO{Mode: "text", Text: "  "}); err == nil {
		t.Fatal("empty text was accepted")
	}
	if _, err := encodeType2NDEF(Type2NDEFWriteStartRequestDTO{Mode: "uri", URI: ""}); err == nil {
		t.Fatal("empty URI was accepted")
	}
	if _, err := encodeType2NDEF(Type2NDEFWriteStartRequestDTO{Mode: "other"}); err == nil {
		t.Fatal("unknown mode was accepted")
	}
}
