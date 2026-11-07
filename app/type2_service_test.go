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
	if _, err := service.StartType2NDEFWrite(Type2NDEFWriteStartRequestDTO{Template: "text", Fields: map[string]string{"text": "hello"}}); err != nil {
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
	if _, err := encodeType2NDEF(Type2NDEFWriteStartRequestDTO{Template: "text", Fields: map[string]string{"text": "  "}}); err == nil {
		t.Fatal("empty text was accepted")
	}
	if _, err := encodeType2NDEF(Type2NDEFWriteStartRequestDTO{Template: "uri", Fields: map[string]string{"uri": ""}}); err == nil {
		t.Fatal("empty URI was accepted")
	}
	if _, err := encodeType2NDEF(Type2NDEFWriteStartRequestDTO{Template: "other"}); err == nil {
		t.Fatal("unknown mode was accepted")
	}
}

func TestPreviewType2NDEF(t *testing.T) {
	preview, err := (&Service{}).PreviewType2NDEF(Type2NDEFWriteStartRequestDTO{
		Template: "contact",
		Fields:   map[string]string{"name": "Ada Lovelace", "email": "ada@example.com"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if preview.Template != "contact" || preview.Length == 0 {
		t.Fatalf("preview = %+v", preview)
	}
	if _, err := (&Service{}).PreviewType2NDEF(Type2NDEFWriteStartRequestDTO{
		Template: "contact", Fields: map[string]string{},
	}); err == nil {
		t.Fatal("missing contact name was accepted")
	}
}

func TestServiceType2LifecyclePreviewAndStart(t *testing.T) {
	service, tag, recorder := newType2AppService(t)
	tag.SetPage(4, [4]byte{1, 2, 3, 4})
	plan, err := service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequestDTO{Operation: "erase"})
	if err != nil {
		t.Fatal(err)
	}
	if plan.Token == "" || plan.Operation != "erase" || len(plan.Writes) == 0 {
		t.Fatalf("plan = %+v", plan)
	}
	if _, err := service.StartType2Lifecycle(Type2LifecycleRequestDTO{Operation: "erase", Token: "stale"}); err != nil {
		t.Fatal(err)
	}
	event := recorder.waitTask(t)
	if event.Type != "failed" {
		t.Fatalf("stale token event = %+v", event)
	}
	if page, _ := tag.Page(4); page != [4]byte{1, 2, 3, 4} {
		t.Fatalf("tag changed with a stale token: % X", page)
	}

	if _, err := service.StartType2Lifecycle(Type2LifecycleRequestDTO{Operation: "erase", Token: plan.Token}); err != nil {
		t.Fatal(err)
	}
	event = recorder.waitTask(t)
	if event.Type != TaskEventCompleted {
		t.Fatalf("erase event = %+v", event)
	}
	if page, _ := tag.Page(4); page != [4]byte{} {
		t.Fatalf("page 4 = % X", page)
	}
}

func TestServiceType2NDEFRawAndRecordsPreview(t *testing.T) {
	service := &Service{}
	raw, err := service.PreviewType2NDEFRaw(Type2NDEFRawRequestDTO{Raw: "D101085402656E68656C6C6F"})
	if err != nil || raw.Length != 12 {
		t.Fatalf("raw preview = %+v, %v", raw, err)
	}
	records, err := service.PreviewType2NDEFRecords(Type2NDEFRecordsRequestDTO{Records: []Type2NDEFRecordInputDTO{
		{Kind: "text", Fields: map[string]string{"text": "hello"}},
		{Kind: "uri", Fields: map[string]string{"uri": "https://example.com"}},
	}})
	if err != nil || records.Length == 0 {
		t.Fatalf("records preview = %+v, %v", records, err)
	}
	if _, err := service.PreviewType2NDEFRaw(Type2NDEFRawRequestDTO{Raw: "ZZ"}); err == nil {
		t.Fatal("invalid hex was accepted")
	}
}
