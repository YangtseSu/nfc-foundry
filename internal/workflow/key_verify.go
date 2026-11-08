package workflow

import (
	"context"
	"errors"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// KeyCheck is one explicit key/slot authentication request.
type KeyCheck struct {
	Sector  int
	KeyType nfc.KeyType
	KeyID   string
	Key     nfc.Key
}

// KeyCheckResult reports one authentication without mutating key-store state.
type KeyCheckResult struct {
	Sector   int
	KeyType  nfc.KeyType
	KeyID    string
	Verified bool
	Error    string
}

type KeyVerifyResult struct {
	Card     nfc.CardInfo
	Checks   []KeyCheckResult
	Verified int
}

type KeyVerifyService struct {
	devices ReaderExecutor
}

func NewKeyVerifyService(devices ReaderExecutor) *KeyVerifyService {
	return &KeyVerifyService{devices: devices}
}

// Verify authenticates every requested slot against the selected card. It is a
// read-only verification action: callers decide whether to update the store.
func (s *KeyVerifyService) Verify(ctx context.Context, card nfc.CardInfo, checks []KeyCheck) (KeyVerifyResult, error) {
	card = card.Clone()
	result := KeyVerifyResult{Card: card, Checks: make([]KeyCheckResult, len(checks))}
	if s == nil || s.devices == nil {
		return result, nfc.NewError("verify keys", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	layout, err := layoutForCard(card)
	if err != nil {
		return result, nfc.NewError("verify keys", nfc.CodeUnsupported, "card is not MIFARE Classic 1K/4K", err)
	}
	for index, check := range checks {
		result.Checks[index] = KeyCheckResult{Sector: check.Sector, KeyType: check.KeyType, KeyID: check.KeyID}
		if check.Sector < 0 || !check.KeyType.Valid() {
			return result, nfc.NewError("verify keys", nfc.CodeInvalidArgument, "sector and key type are required", nil)
		}
		sectors, _ := layout.SectorCount()
		if check.Sector >= sectors {
			return result, nfc.NewError("verify keys", nfc.CodeInvalidArgument, fmt.Sprintf("sector %d is outside the card layout", check.Sector), nil)
		}
	}
	err = s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		for index, check := range checks {
			if err := operationCtx.Err(); err != nil {
				return err
			}
			if err := selectExpectedCard(operationCtx, reader, card); err != nil {
				return err
			}
			block, _ := layout.FirstBlock(check.Sector)
			authErr := reader.Authenticate(operationCtx, byte(block), check.KeyType, check.Key)
			if authErr == nil {
				result.Checks[index].Verified = true
				result.Verified++
				continue
			}
			result.Checks[index].Error = authErr.Error()
			if !errors.Is(authErr, nfc.ErrAuthenticationFailed) {
				return authErr
			}
		}
		return nil
	})
	return result, err
}
