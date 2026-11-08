package app

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/device"
	"github.com/YangtseSu/nfc-foundry/internal/keys"
	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/workbench"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

// ClassicKeyOverride selects one exact key for a single-block operation.
type ClassicKeyOverride struct {
	KeyType nfc.KeyType
	Key     nfc.Key
}

// ClassicBlockResult is the shared result of a single-block read or write.
type ClassicBlockResult struct {
	Block   int
	Sector  int
	Kind    string
	Data    [nfc.BlockSize]byte
	KeyType nfc.KeyType
}

type classicKeyCandidate struct {
	keyType nfc.KeyType
	key     nfc.Key
	keyID   string
}

// WaitForCard blocks until the polling loop reports a card or the reader
// leaves a usable state.
func (s *Service) WaitForCard(ctx context.Context) (nfc.CardInfo, error) {
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for {
		snapshot := s.deviceManager.Snapshot()
		if snapshot.Card != nil {
			return snapshot.Card.Clone(), nil
		}
		switch snapshot.Status {
		case device.StatusDisconnected:
			return nfc.CardInfo{}, nfc.NewError("wait for card", nfc.CodeNotOpen, "reader is not connected", nil)
		case device.StatusError:
			code := snapshot.ErrorCode
			if code == nfc.CodeUnknown {
				code = nfc.CodeIO
			}
			detail := snapshot.ErrorMessage
			if detail == "" {
				detail = "reader is not available"
			}
			return nfc.CardInfo{}, nfc.NewError("wait for card", code, detail, nil)
		}
		select {
		case <-ctx.Done():
			code := nfc.CodeCanceled
			if errors.Is(ctx.Err(), context.DeadlineExceeded) {
				code = nfc.CodeTimeout
			}
			return nfc.CardInfo{}, nfc.NewError("wait for card", code, "", ctx.Err())
		case <-ticker.C:
		}
	}
}

// WaitForCardRemoval blocks until polling reports that the card left the
// field. It is used by repeated copy between tags.
func (s *Service) WaitForCardRemoval(ctx context.Context) error {
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for {
		snapshot := s.deviceManager.Snapshot()
		if snapshot.Card == nil {
			return nil
		}
		switch snapshot.Status {
		case device.StatusDisconnected:
			return nfc.NewError("wait for card removal", nfc.CodeNotOpen, "reader is not connected", nil)
		case device.StatusError:
			code := snapshot.ErrorCode
			if code == nfc.CodeUnknown {
				code = nfc.CodeIO
			}
			return nfc.NewError("wait for card removal", code, snapshot.ErrorMessage, nil)
		}
		select {
		case <-ctx.Done():
			code := nfc.CodeCanceled
			if errors.Is(ctx.Err(), context.DeadlineExceeded) {
				code = nfc.CodeTimeout
			}
			return nfc.NewError("wait for card removal", code, "", ctx.Err())
		case <-ticker.C:
		}
	}
}

// ReadClassicBlock reads one block with an explicit key or the verified key
// catalog. It retries the other verified slot only after authentication
// failures, never after a mutating operation.
func (s *Service) ReadClassicBlock(ctx context.Context, block int, override *ClassicKeyOverride) (ClassicBlockResult, error) {
	if err := s.requireClassicRead("read block"); err != nil {
		return ClassicBlockResult{}, err
	}
	card, layout, candidates, err := s.classicBlockContext(block, override)
	if err != nil {
		return ClassicBlockResult{}, err
	}
	var lastErr error
	for _, candidate := range candidates {
		if err := ctx.Err(); err != nil {
			return ClassicBlockResult{}, err
		}
		data, err := s.classicIO.ReadBlock(ctx, workflow.ClassicBlockRequest{
			Card: card, Block: block, KeyType: candidate.keyType, Key: candidate.key,
		})
		if err == nil {
			return classicBlockResult(layout, block, data, candidate.keyType), nil
		}
		lastErr = err
		if !errors.Is(err, nfc.ErrAuthenticationFailed) {
			return ClassicBlockResult{}, err
		}
		s.unverifyRejectedCandidate(card, block, candidate)
	}
	return ClassicBlockResult{}, lastErr
}

// WriteClassicBlock writes one ordinary data block and relies on the reader's
// immediate read-back verification. Block 0 and sector trailers are rejected;
// those paths belong to uid set and the restore workflow.
func (s *Service) WriteClassicBlock(ctx context.Context, block int, data [nfc.BlockSize]byte, override *ClassicKeyOverride) (ClassicBlockResult, error) {
	if err := s.requireClassicWrite("write block"); err != nil {
		return ClassicBlockResult{}, err
	}
	card, layout, candidates, err := s.classicBlockContext(block, override)
	if err != nil {
		return ClassicBlockResult{}, err
	}
	if block == 0 {
		return ClassicBlockResult{}, nfc.NewError("write block", nfc.CodeInvalidArgument, "block 0 writes require `nfctl uid set`", nil)
	}
	if trailer, _ := layout.IsTrailer(block); trailer {
		return ClassicBlockResult{}, nfc.NewError("write block", nfc.CodeInvalidArgument, "sector trailer writes require `nfctl classic restore`", nil)
	}
	var lastErr error
	for _, candidate := range candidates {
		if err := ctx.Err(); err != nil {
			return ClassicBlockResult{}, err
		}
		err := s.classicIO.WriteBlock(ctx, workflow.ClassicBlockRequest{
			Card: card, Block: block, KeyType: candidate.keyType, Key: candidate.key,
		}, data)
		if err == nil {
			return classicBlockResult(layout, block, data, candidate.keyType), nil
		}
		lastErr = err
		if !errors.Is(err, nfc.ErrAuthenticationFailed) {
			return ClassicBlockResult{}, err
		}
		s.unverifyRejectedCandidate(card, block, candidate)
	}
	return ClassicBlockResult{}, lastErr
}

func (s *Service) unverifyRejectedCandidate(card nfc.CardInfo, block int, candidate classicKeyCandidate) {
	if candidate.keyID == "" {
		return
	}
	layout, err := appLayoutForCard(card)
	if err != nil {
		return
	}
	sector, err := layout.SectorForBlock(block)
	if err != nil {
		return
	}
	s.keyStore.Unverify(keys.CardID(card), sector, candidate.keyType)
	_ = s.persistKeys()
	s.emitter.Emit(KeyEventName, sectorKeyDTOs(s.keyStore, &card))
}

func (s *Service) classicBlockContext(block int, override *ClassicKeyOverride) (nfc.CardInfo, mifare.Layout, []classicKeyCandidate, error) {
	snapshot := s.deviceManager.Snapshot()
	if snapshot.Card == nil {
		return nfc.CardInfo{}, 0, nil, ErrCardRequired
	}
	card := snapshot.Card.Clone()
	layout, err := appLayoutForCard(card)
	if err != nil {
		return nfc.CardInfo{}, 0, nil, err
	}
	sector, err := layout.SectorForBlock(block)
	if err != nil {
		return nfc.CardInfo{}, 0, nil, nfc.NewError("select block", nfc.CodeInvalidArgument, err.Error(), err)
	}
	if override != nil {
		if !override.KeyType.Valid() {
			return nfc.CardInfo{}, 0, nil, nfc.NewError("select key", nfc.CodeInvalidArgument, "unknown key type", nil)
		}
		return card, layout, []classicKeyCandidate{{keyType: override.KeyType, key: override.Key}}, nil
	}
	candidates := make([]classicKeyCandidate, 0, 2)
	for _, match := range s.keyStore.Verified(keys.CardID(card)) {
		if match.Sector != sector {
			continue
		}
		record, exists := s.keyStore.Record(match.KeyID)
		if !exists {
			continue
		}
		candidates = append(candidates, classicKeyCandidate{
			keyType: match.KeyType, key: record.Value, keyID: match.KeyID,
		})
	}
	if len(candidates) == 0 {
		return nfc.CardInfo{}, 0, nil, nfc.NewError("select key", nfc.CodeAuthenticationFailed, fmt.Sprintf("no verified key is available for sector %d; pass an explicit key or run classic keyscan", sector), nil)
	}
	return card, layout, candidates, nil
}

func classicBlockResult(layout mifare.Layout, block int, data [nfc.BlockSize]byte, keyType nfc.KeyType) ClassicBlockResult {
	sector, _ := layout.SectorForBlock(block)
	return ClassicBlockResult{
		Block: block, Sector: sector, Kind: string(workbench.KindForBlock(layout, block)),
		Data: data, KeyType: keyType,
	}
}
