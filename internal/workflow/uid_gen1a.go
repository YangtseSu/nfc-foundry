package workflow

import (
	"bytes"
	"context"
	"errors"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/attack"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	runtimebundle "github.com/YangtseSu/nfc-foundry/internal/runtime"
)

type UIDFallbackRequest struct {
	TaskID string
	Card   nfc.CardInfo
	Block0 [nfc.BlockSize]byte
}

// UIDFallbackWriter owns the out-of-process Gen1A path. Implementations must
// release the in-process reader before invoking a raw-frame helper.
type UIDFallbackWriter interface {
	Write(context.Context, UIDFallbackRequest, func(UIDWriteEvent)) error
}

type Gen1AUIDWriter struct {
	released attack.ReleasedDeviceController
	locator  *runtimebundle.Locator
}

func NewGen1AUIDWriter(released attack.ReleasedDeviceController, locator *runtimebundle.Locator) *Gen1AUIDWriter {
	return &Gen1AUIDWriter{released: released, locator: locator}
}

func (w *Gen1AUIDWriter) Write(ctx context.Context, request UIDFallbackRequest, emit func(UIDWriteEvent)) error {
	if w == nil || w.released == nil || w.locator == nil {
		return attack.ErrUnavailable
	}
	engine := attack.NewMFSetUIDEngine(w.locator, attack.MFSetUIDInvocation{Card: request.Card, Block0: request.Block0})
	coordinator := attack.NewCoordinator(w.released, engine)
	expected := request.Card.Clone()
	result, err := coordinator.Run(ctx, attack.AttackRequest{
		TaskID: request.TaskID, Timeout: 15 * time.Second, ExpectedCard: &expected,
	}, func(event attack.AttackEvent) {
		if emit == nil {
			return
		}
		if raw := strings.TrimSpace(event.Data); raw != "" {
			emit(UIDWriteEvent{Phase: UIDWriteFallback, Message: "nfc-mfsetuid: " + raw})
		}
		if event.Message == "" {
			return
		}
		message := "running the Gen1A write"
		switch event.State {
		case attack.StateReleasingDevice:
			message = "releasing the reader and switching to the Gen1A write"
		case attack.StateStarting, attack.StateRunning:
			message = "sending the Gen1A unlock and block 0 write sequence"
		case attack.StateReopeningDevice:
			message = "reopening the reader"
		}
		emit(UIDWriteEvent{Phase: UIDWriteFallback, Message: message})
	})
	if err != nil {
		return err
	}
	if result.RecoveryError != nil {
		return errors.Join(errors.New("the reader could not be reopened after the Gen1A write"), result.RecoveryError)
	}
	if emit != nil {
		emit(UIDWriteEvent{Phase: UIDWriteFallback, Message: describeGen1AResult(result.Stdout, result.Stderr)})
	}
	return nil
}

func describeGen1AResult(stdout, stderr []byte) string {
	output := bytes.Join([][]byte{stdout, stderr}, []byte{'\n'})
	switch {
	case bytes.Contains(output, []byte("Unlock command [1/2]: failed")):
		return "Gen1A diagnostic: the card did not answer the 7-bit 0x40 unlock command; the backdoor is absent or locked"
	case bytes.Contains(output, []byte("Unlock command [2/2]: failed")):
		return "Gen1A diagnostic: the card answered step 1 but refused the 0x43 unlock command"
	case bytes.Contains(output, []byte("Card unlocked")):
		return "Gen1A diagnostic: the card accepted the backdoor unlock; the write result is decided by reselection and a full block 0 readback"
	default:
		return "Gen1A diagnostic: the tool did not report an explicit unlock result; the write result is decided by reselection and a full block 0 readback"
	}
}
