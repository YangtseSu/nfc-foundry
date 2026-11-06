package app

import (
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/attack"
	runtimebundle "github.com/YangtseSu/nfc-foundry/internal/runtime"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func TestRecoveryEntryPointsRequireAuthorization(t *testing.T) {
	service := &Service{mfocLocator: runtimebundle.NewLocator(t.TempDir())}
	if _, err := service.StartMFoC(MFoCStartRequestDTO{Authorized: false}); !errors.Is(err, workflow.ErrMFoCAuthorization) {
		t.Fatalf("StartMFoC authorization error = %v", err)
	}
	if _, err := service.StartHardnested(HardnestedStartRequestDTO{Authorized: false}); !errors.Is(err, workflow.ErrMFoCAuthorization) {
		t.Fatalf("StartHardnested authorization error = %v", err)
	}
}

func TestRecoveryEntryPointsFailWhenEngineServiceIsMissing(t *testing.T) {
	service := &Service{mfocLocator: runtimebundle.NewLocator(t.TempDir())}
	if _, err := service.StartMFoC(MFoCStartRequestDTO{Authorized: true}); !errors.Is(err, attack.ErrUnavailable) {
		t.Fatalf("StartMFoC service error = %v", err)
	}
	if _, err := service.StartHardnested(HardnestedStartRequestDTO{Authorized: true}); !errors.Is(err, attack.ErrUnavailable) {
		t.Fatalf("StartHardnested service error = %v", err)
	}
}
