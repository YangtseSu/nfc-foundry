package cli

import (
	"context"
	"errors"
	"io/fs"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/attack"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

const (
	exitOK         = 0
	exitOperation  = 1
	exitUsage      = 2
	exitValidation = 3
	exitHardware   = 4
	exitEngine     = 5
	exitCanceled   = 6
	exitInternal   = 7
)

// classifyError maps an application error to the stable CLI exit codes.
func classifyError(err error) (string, int) {
	var usage *usageError
	switch {
	case errors.As(err, &usage):
		return "usage", exitUsage
	case errors.Is(err, context.Canceled),
		errors.Is(err, nfc.ErrCanceled),
		errors.Is(err, attack.ErrCancelled):
		return "canceled", exitCanceled
	case errors.Is(err, app.ErrTrailerConfirm), errors.Is(err, app.ErrUIDConfirm),
		errors.Is(err, workflow.ErrMFoCAuthorization), errors.Is(err, workflow.ErrMFCUKAuthorization):
		return "usage", exitUsage
	case errors.Is(err, app.ErrCardRequired), errors.Is(err, app.ErrOperationRunning):
		return "hardware", exitHardware
	case errors.Is(err, app.ErrDirtyWorkbench), errors.Is(err, app.ErrPreflightRequired),
		errors.Is(err, mifare.ErrIncompleteDump),
		errors.Is(err, workflow.ErrMFoCSeedRequired), errors.Is(err, workflow.ErrMFCUKSeedAlreadyKnown),
		errors.Is(err, workflow.ErrMFCUKNoVerifiedCandidate):
		return "validation", exitValidation
	case errors.Is(err, workflow.ErrMFoCNested), errors.Is(err, workflow.ErrHardnestedCapability),
		errors.Is(err, workflow.ErrMFCUKClassic1K), errors.Is(err, workflow.ErrMFCUKDarkside):
		return "hardware", exitHardware
	}
	var attackErr *attack.Error
	if errors.As(err, &attackErr) {
		return "engine", exitEngine
	}
	switch {
	case errors.Is(err, attack.ErrUnavailable), errors.Is(err, attack.ErrStart),
		errors.Is(err, attack.ErrExit), errors.Is(err, attack.ErrTimeout),
		errors.Is(err, attack.ErrValidation), errors.Is(err, attack.ErrTemporary):
		return "engine", exitEngine
	}
	if code, ok := nfc.ErrorCodeOf(err); ok {
		switch code {
		case nfc.CodePermission:
			return "permission", exitHardware
		case nfc.CodeNoCard, nfc.CodeTimeout, nfc.CodeDeviceDisconnected, nfc.CodeIO,
			nfc.CodeBusy, nfc.CodeNotOpen, nfc.CodeUnsupported:
			return "hardware", exitHardware
		case nfc.CodeInvalidArgument:
			return "validation", exitValidation
		case nfc.CodeCanceled:
			return "canceled", exitCanceled
		case nfc.CodeAuthenticationFailed, nfc.CodeNotAuthenticated, nfc.CodeCardChanged,
			nfc.CodeVerificationFailed:
			return "operation", exitOperation
		case nfc.CodeInternal:
			return "internal", exitInternal
		}
	}
	if errors.Is(err, fs.ErrPermission) {
		return "permission", exitHardware
	}
	return "operation", exitOperation
}
