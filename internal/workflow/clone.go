package workflow

import (
	"bytes"
	"context"
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

type ClonePhase string

const (
	ClonePreflight ClonePhase = "preflight"
	CloneRestore   ClonePhase = "restore"
	CloneBlock0    ClonePhase = "block0"
	CloneReadback  ClonePhase = "readback"
	CloneVerify    ClonePhase = "verify"
	CloneComplete  ClonePhase = "complete"
)

type CloneEvent struct {
	Phase   ClonePhase
	Block   int
	Message string
}

type CloneRequest struct {
	Source     DumpResult
	Card       nfc.CardInfo
	Device     nfc.DeviceInfo
	TargetKeys []SectorKeys
}

type ClonePlan struct {
	Card        nfc.CardInfo
	SourceCard  nfc.CardInfo
	Layout      mifare.Layout
	Restore     RestorePlan
	Block0      *UIDWritePlan
	Block0Write bool
	OldBlock0   [nfc.BlockSize]byte
	NewBlock0   [nfc.BlockSize]byte
}

type CloneResult struct {
	Phase      ClonePhase
	Restore    RestoreResult
	UID        *UIDWriteResult
	Readback   DumpResult
	Diff       DumpDiff
	Verified   bool
	TargetCard nfc.CardInfo
}

type CloneService struct {
	devices ReaderExecutor
	dumps   *DumpService
	uid     *UIDService
}

func NewCloneService(devices ReaderExecutor, dumps *DumpService, uid *UIDService) *CloneService {
	return &CloneService{devices: devices, dumps: dumps, uid: uid}
}

// Preflight proves the clone is possible without mutating the target.
func (s *CloneService) Preflight(ctx context.Context, request CloneRequest) (ClonePlan, error) {
	request.Source = cloneDumpResult(request.Source)
	request.Card = request.Card.Clone()
	if s == nil || s.devices == nil {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	var plan ClonePlan
	err := s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		var err error
		plan, err = s.buildPlan(operationCtx, reader, request)
		return err
	})
	return plan, err
}

// Clone runs preflight, the restore, the optional block 0 write, and a
// read-back diff under one exclusive reader lease.
func (s *CloneService) Clone(ctx context.Context, request CloneRequest, emit func(CloneEvent)) (CloneResult, error) {
	request.Source = cloneDumpResult(request.Source)
	request.Card = request.Card.Clone()
	result := CloneResult{Phase: ClonePreflight, TargetCard: request.Card.Clone()}
	if s == nil || s.devices == nil || s.dumps == nil || s.uid == nil {
		return result, nfc.NewError("clone", nfc.CodeNotOpen, "clone services are unavailable", nil)
	}
	if emit == nil {
		emit = func(CloneEvent) {}
	}
	emit(CloneEvent{Phase: ClonePreflight, Block: -1, Message: "validating the source image and target card"})

	err := s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		plan, err := s.buildPlan(operationCtx, reader, request)
		if err != nil {
			return err
		}
		result.TargetCard = plan.Card.Clone()
		restoreResult := newRestoreResult(RestoreRequest{Card: plan.Card, Dump: request.Source.Dump})
		emit(CloneEvent{Phase: CloneRestore, Block: -1, Message: "writing data blocks and trailers"})
		if err := executeRestorePlan(operationCtx, reader, plan.Restore, &restoreResult, func(event RestoreEvent) {
			emit(CloneEvent{Phase: CloneRestore, Block: event.Block, Message: event.Message})
		}); err != nil {
			restoreResult.Phase = RestoreFailed
			finalizeRestoreResult(&restoreResult)
			result.Restore = restoreResult
			result.Phase = CloneRestore
			return err
		}
		restoreResult.Phase = RestoreComplete
		finalizeRestoreResult(&restoreResult)
		result.Restore = restoreResult

		if plan.Block0Write {
			emit(CloneEvent{Phase: CloneBlock0, Block: 0, Message: "writing the source manufacturer block"})
			expected := plan.OldBlock0
			sourceKeys := SectorKeysFromDump(request.Source.Dump)
			blockPlan, err := buildBlock0WritePlan(operationCtx, reader, plan.Card, sourceKeys[0], &expected)
			if err != nil {
				result.Phase = CloneBlock0
				return err
			}
			blockPlan.NewBlock0 = request.Source.Dump.Blocks[0].Data
			blockPlan.BCC = blockPlan.NewBlock0[4]
			uidResult := UIDWriteResult{OldCard: plan.Card.Clone(), OldBlock0: blockPlan.OldBlock0, NewBlock0: blockPlan.NewBlock0}
			if err := s.uid.writePlanWithReader(operationCtx, reader, blockPlan, &uidResult, func(event UIDWriteEvent) {
				phase := CloneBlock0
				if event.Phase == UIDWriteVerify {
					phase = CloneVerify
				}
				emit(CloneEvent{Phase: phase, Block: 0, Message: event.Message})
			}); err != nil {
				result.UID = &uidResult
				result.Phase = CloneBlock0
				return err
			}
			result.UID = &uidResult
			if uidResult.NewCard.UID != nil {
				result.TargetCard = uidResult.NewCard.Clone()
			}
		}

		emit(CloneEvent{Phase: CloneReadback, Block: -1, Message: "reading the target back for comparison"})
		readback, readErr := s.dumps.ReadCardWithReader(operationCtx, reader, DumpRequest{
			Card: result.TargetCard, Device: request.Device, Keys: SectorKeysFromDump(request.Source.Dump),
		})
		result.Readback = readback
		if readErr != nil {
			result.Phase = CloneReadback
			return readErr
		}
		diff, diffErr := DiffDumps(request.Source, readback)
		if diffErr != nil {
			result.Phase = CloneVerify
			return diffErr
		}
		result.Diff = diff
		emit(CloneEvent{Phase: CloneVerify, Block: -1, Message: "comparing the target read-back with the source image"})
		switch {
		case diff.UnknownBlocks != 0:
			result.Phase = CloneVerify
			return nfc.NewError("clone verification", nfc.CodeVerificationFailed, fmt.Sprintf("the target read-back has %d unknown blocks; the clone is not verified", diff.UnknownBlocks), nil)
		case diff.ChangedBlocks != 0:
			result.Phase = CloneVerify
			return nfc.NewError("clone verification", nfc.CodeVerificationFailed, fmt.Sprintf("the target differs from the source in %d blocks; the clone is not verified", diff.ChangedBlocks), nil)
		case diff.IdentityChanged:
			result.Phase = CloneVerify
			return nfc.NewError("clone verification", nfc.CodeVerificationFailed, "the target card identity does not match the source image", nil)
		}
		result.Verified = true
		result.Phase = CloneComplete
		emit(CloneEvent{Phase: CloneComplete, Block: -1, Message: "clone verified against the source image"})
		return nil
	})
	if err != nil {
		return result, err
	}
	return result, nil
}

func (s *CloneService) buildPlan(ctx context.Context, reader nfc.Reader, request CloneRequest) (ClonePlan, error) {
	layout, err := layoutForCard(request.Card)
	if err != nil {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeUnsupported, "target card is not MIFARE Classic 1K/4K", err)
	}
	if request.Source.Dump.Layout != layout {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeInvalidArgument, "source image capacity does not match the target card", nil)
	}
	if request.Source.Card.ATQA != request.Card.ATQA || request.Source.Card.SAK != request.Card.SAK {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeUnsupported, "source and target card types differ; ATQA and SAK cannot be cloned", nil)
	}
	if !request.Source.Complete() {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeInvalidArgument, "a partial source image cannot be cloned", mifare.ErrIncompleteDump)
	}
	if _, err := request.Source.Dump.ValidateBCC(len(request.Source.Card.UID)); err != nil {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeInvalidArgument, err.Error(), err)
	}
	if err := request.Source.Dump.ValidateTrailers(); err != nil {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeInvalidArgument, err.Error(), err)
	}
	keyMap, err := validateSectorKeys(layout, request.TargetKeys)
	if err != nil {
		return ClonePlan{}, err
	}
	restore, err := buildRestorePlan(ctx, reader, RestoreRequest{
		Card: request.Card, Dump: request.Source.Dump.Clone(), SourceUIDLength: len(request.Source.Card.UID),
		TargetKeys: request.TargetKeys,
	})
	if err != nil {
		return ClonePlan{}, err
	}
	plan := ClonePlan{Card: request.Card.Clone(), SourceCard: request.Source.Card.Clone(), Layout: layout, Restore: restore}

	targetBlock0, err := readTargetBlock0(ctx, reader, request.Card, keyMap[0])
	if err != nil {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeAuthenticationFailed, "could not read the target manufacturer block", err)
	}
	sourceBlock0 := request.Source.Dump.Blocks[0].Data
	plan.OldBlock0, plan.NewBlock0 = targetBlock0, sourceBlock0
	if bytes.Equal(sourceBlock0[:], targetBlock0[:]) {
		return plan, nil
	}
	if len(request.Source.Card.UID) != 4 {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeUnsupported, "block 0 cloning requires a four-byte source UID", nil)
	}
	if !bytes.Equal(sourceBlock0[0:4], request.Source.Card.UID) {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeVerificationFailed, "source block 0 UID does not match the source card identity", nil)
	}
	sourceBCC, _ := mifare.BCCFor4ByteUID(request.Source.Card.UID)
	if sourceBlock0[4] != sourceBCC {
		return ClonePlan{}, nfc.NewError("clone preflight", nfc.CodeInvalidArgument, "source block 0 BCC is invalid", nil)
	}
	expected := targetBlock0
	blockPlan, err := buildBlock0WritePlan(ctx, reader, request.Card, keyMap[0], &expected)
	if err != nil {
		return ClonePlan{}, err
	}
	blockPlan.NewBlock0, blockPlan.BCC = sourceBlock0, sourceBCC
	plan.Block0, plan.Block0Write = &blockPlan, true
	return plan, nil
}

func readTargetBlock0(ctx context.Context, reader nfc.Reader, card nfc.CardInfo, sector0Keys SectorKeys) ([nfc.BlockSize]byte, error) {
	verified, err := verifyProvidedKeys(ctx, reader, card, 0, mifare.Classic1K, sector0Keys)
	if err != nil {
		return [nfc.BlockSize]byte{}, err
	}
	keyType, key, ok := preferredVerifiedKey(verified)
	if !ok {
		return [nfc.BlockSize]byte{}, nfc.NewError("read target block 0", nfc.CodeAuthenticationFailed, "no verified key is available for sector 0", nil)
	}
	if err := selectExpectedCard(ctx, reader, card); err != nil {
		return [nfc.BlockSize]byte{}, err
	}
	if err := reader.Authenticate(ctx, 0, keyType, key); err != nil {
		return [nfc.BlockSize]byte{}, err
	}
	block, err := reader.ReadBlock(ctx, 0)
	if err != nil {
		return [nfc.BlockSize]byte{}, err
	}
	return block, nil
}

// SectorKeysFromDump derives authentication keys from a complete image's
// trailers. Key A is always stored; Key B is used only when its bytes are
// known from a readable trailer or authenticated metadata.
func SectorKeysFromDump(image mifare.Dump) []SectorKeys {
	sectors, err := image.Layout.SectorCount()
	if err != nil {
		return nil
	}
	keys := make([]SectorKeys, sectors)
	for sector := range sectors {
		keys[sector].Sector = sector
		block, err := image.Layout.TrailerBlock(sector)
		if err != nil || block >= len(image.Blocks) {
			continue
		}
		trailer := image.Blocks[block]
		if trailer.KnownMask&0x003f == 0x003f {
			key := nfc.Key(trailer.Data[0:6])
			keys[sector].KeyA = &key
		}
		if trailer.KnownMask&0xfc00 == 0xfc00 {
			key := nfc.Key(trailer.Data[10:16])
			keys[sector].KeyB = &key
		}
	}
	return keys
}

// BuildFactoryImage derives the transport-configuration image from a complete
// current backup. Block 0 is copied unchanged.
func BuildFactoryImage(backup DumpResult, keepData bool) (DumpResult, error) {
	if !backup.Complete() {
		return DumpResult{}, nfc.NewError("factory format", nfc.CodeInvalidArgument, "a complete target backup is required", mifare.ErrIncompleteDump)
	}
	layout := backup.Dump.Layout
	if !layout.Valid() {
		return DumpResult{}, nfc.NewError("factory format", nfc.CodeUnsupported, "target card is not MIFARE Classic 1K/4K", nil)
	}
	image := backup.Dump.Clone()
	sectors, _ := layout.SectorCount()
	for sector := range sectors {
		first, _ := layout.FirstBlock(sector)
		count, _ := layout.BlocksInSector(sector)
		for block := first; block < first+count; block++ {
			if block == 0 {
				continue
			}
			entry := &image.Blocks[block]
			entry.KnownMask = mifare.AllBytesKnown
			entry.Status = mifare.BlockSynthetic
			entry.Error = ""
			entry.ErrorCode = ""
			if block == first+count-1 {
				entry.Data = [nfc.BlockSize]byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x07, 0x80, 0x69, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff}
				entry.KeyASource = "factory_default"
				entry.KeyBSource = "factory_default"
				continue
			}
			if !keepData {
				entry.Data = [nfc.BlockSize]byte{}
			}
		}
	}
	return DumpResult{Dump: image, Card: backup.Card.Clone(), Device: backup.Device}, nil
}

func cloneDumpResult(result DumpResult) DumpResult {
	clone := result
	clone.Dump = result.Dump.Clone()
	clone.Card = result.Card.Clone()
	clone.Keys = make([]VerifiedSectorKeys, len(result.Keys))
	for index, keys := range result.Keys {
		clone.Keys[index] = VerifiedSectorKeys{Sector: keys.Sector}
		if keys.KeyA != nil {
			key := *keys.KeyA
			clone.Keys[index].KeyA = &key
		}
		if keys.KeyB != nil {
			key := *keys.KeyB
			clone.Keys[index].KeyB = &key
		}
	}
	return clone
}
