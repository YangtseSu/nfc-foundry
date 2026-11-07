package workflow

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"sort"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

// ClassicNDEFRequest selects a card for a read-only NDEF operation.
type ClassicNDEFRequest struct {
	Card nfc.CardInfo
	Keys []SectorKeys
}

// ClassicNDEFWriteRequest writes one validated NDEF message.
type ClassicNDEFWriteRequest struct {
	Card    nfc.CardInfo
	Device  nfc.DeviceInfo
	Keys    []SectorKeys
	Message []byte
	DryRun  bool
}

// ClassicNDEFFormatRequest initializes a contiguous NDEF application.
type ClassicNDEFFormatRequest struct {
	Card    nfc.CardInfo
	Device  nfc.DeviceInfo
	Keys    []SectorKeys
	Sectors []int
	DryRun  bool
}

// ClassicNDEFState is the validated mapping and message read from a card.
type ClassicNDEFState struct {
	Card        nfc.CardInfo
	Layout      mifare.Layout
	MAD         mifare.MAD
	Application bool
	Sectors     []int
	Blocks      []int
	Capacity    int
	Area        []byte
	NDEF        *type2.NDEF
	ReadOnly    bool
	Keys        []VerifiedSectorKeys

	keyMap      map[int]SectorKeys
	verified    map[int]VerifiedSectorKeys
	sector0     [4][mifare.BlockSize]byte
	sectorReads map[int]classicSectorRead
}

// ClassicNDEFWriteResult reports exactly which blocks changed.
type ClassicNDEFWriteResult struct {
	State      ClassicNDEFState
	BackupPath string
	Planned    []int
	Written    []int
	Verified   []int
	Changes    []type2.Change
	DryRun     bool
	Formatted  bool
}

// ClassicNDEFFormatPlan is a read-only formatting preview.
type ClassicNDEFFormatPlan struct {
	Card       nfc.CardInfo
	Layout     mifare.Layout
	Version    int
	CreateMAD  bool
	Sectors    []int
	Capacity   int
	DataBlocks []int
	Trailer    []int
	MADBlocks  []int
	Changes    []type2.Change
	Warnings   []string
}

// ClassicNDEFBackupStore persists a project dump before a write or format.
type ClassicNDEFBackupStore interface {
	Save(context.Context, DumpResult) (string, error)
}

// ClassicNDEFService implements the Classic NDEF mapping over one reader.
type ClassicNDEFService struct {
	devices ReaderExecutor
	dumps   *DumpService
	backups ClassicNDEFBackupStore
}

func NewClassicNDEFService(devices ReaderExecutor, dumps *DumpService, backups ClassicNDEFBackupStore) *ClassicNDEFService {
	return &ClassicNDEFService{devices: devices, dumps: dumps, backups: backups}
}

// Read returns the mapping state and decoded message without writing.
func (s *ClassicNDEFService) Read(ctx context.Context, request ClassicNDEFRequest) (ClassicNDEFState, error) {
	if s == nil || s.devices == nil {
		return ClassicNDEFState{}, nfc.NewError("read Classic NDEF", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	request.Card = request.Card.Clone()
	var state ClassicNDEFState
	err := s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		var readErr error
		state, readErr = s.readState(operationCtx, reader, request.Card, request.Keys)
		return readErr
	})
	return state, err
}

// PreviewFormat validates a formatting target without modifying the card.
func (s *ClassicNDEFService) PreviewFormat(ctx context.Context, request ClassicNDEFFormatRequest) (ClassicNDEFFormatPlan, error) {
	if s == nil || s.devices == nil {
		return ClassicNDEFFormatPlan{}, nfc.NewError("preview Classic NDEF format", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	request.Card = request.Card.Clone()
	var plan ClassicNDEFFormatPlan
	err := s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		built, buildErr := s.buildFormatPlan(operationCtx, reader, request.Card, request.Keys, request.Sectors)
		if buildErr != nil {
			return buildErr
		}
		plan = built.preview
		return nil
	})
	return plan, err
}

// Write applies one NDEF message to an existing mapping. It never changes a
// sector trailer.
func (s *ClassicNDEFService) Write(ctx context.Context, request ClassicNDEFWriteRequest) (ClassicNDEFWriteResult, error) {
	if s == nil || s.devices == nil {
		return ClassicNDEFWriteResult{}, nfc.NewError("write Classic NDEF", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	request.Card = request.Card.Clone()
	request.Message = append([]byte(nil), request.Message...)
	var result ClassicNDEFWriteResult
	result.DryRun = request.DryRun
	err := s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		state, err := s.readState(operationCtx, reader, request.Card, request.Keys)
		if err != nil {
			return err
		}
		result.State = state
		if !state.Application {
			return nfc.NewError("write Classic NDEF", nfc.CodeInvalidArgument, "card has no NDEF application; format it first", nil)
		}
		if state.ReadOnly {
			return nfc.NewError("write Classic NDEF", nfc.CodePermission, "the NDEF sectors are marked read-only", nil)
		}
		desired, err := type2.BuildNDEFArea(state.Capacity, request.Message)
		if err != nil {
			return nfc.NewError("write Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
		}
		writes, err := planClassicNDEFBlocks(state, desired)
		if err != nil {
			return err
		}
		result.Planned = plannedBlockNumbers(writes)
		if len(writes) == 0 {
			return nil
		}
		if request.DryRun {
			result.Changes = classicNDEFChanges(state, writes)
			return nil
		}
		if s.dumps == nil || s.backups == nil {
			return nfc.NewError("write Classic NDEF", nfc.CodeInternal, "backup storage is unavailable; the card was not modified", nil)
		}
		backupPath, err := s.saveBackup(operationCtx, reader, request.Card, request.Device, state)
		if err != nil {
			return err
		}
		result.BackupPath = backupPath
		result.Changes = classicNDEFChanges(state, writes)
		if err := s.executeWrites(operationCtx, reader, request.Card, writes, &result); err != nil {
			return err
		}
		final, err := s.readState(operationCtx, reader, request.Card, request.Keys)
		if err != nil {
			return nfc.NewError("verify Classic NDEF", nfc.CodeVerificationFailed, "the message was written but the final read failed", err)
		}
		if !nfc.SameCard(request.Card, final.Card) {
			return nfc.NewError("verify Classic NDEF", nfc.CodeCardChanged, "the card changed before final verification", nil)
		}
		if !bytes.Equal(final.Area, desired) {
			return nfc.NewError("verify Classic NDEF", nfc.CodeVerificationFailed, "the NDEF area does not match the requested message", nil)
		}
		result.State = final
		return nil
	})
	return result, err
}

// Format initializes a contiguous NDEF application. It is destructive: the
// caller must require explicit confirmation and an Advanced-mode UI.
func (s *ClassicNDEFService) Format(ctx context.Context, request ClassicNDEFFormatRequest) (ClassicNDEFWriteResult, error) {
	if s == nil || s.devices == nil {
		return ClassicNDEFWriteResult{}, nfc.NewError("format Classic NDEF", nfc.CodeNotOpen, "device manager is unavailable", nil)
	}
	if !request.DryRun && (s.dumps == nil || s.backups == nil) {
		return ClassicNDEFWriteResult{}, nfc.NewError("format Classic NDEF", nfc.CodeInternal, "backup storage is unavailable; the card was not modified", nil)
	}
	request.Card = request.Card.Clone()
	var result ClassicNDEFWriteResult
	result.DryRun = request.DryRun
	err := s.devices.WithReader(ctx, func(operationCtx context.Context, reader nfc.Reader) error {
		plan, err := s.buildFormatPlan(operationCtx, reader, request.Card, request.Keys, request.Sectors)
		if err != nil {
			return err
		}
		result.Planned = plannedBlockNumbers(plan.writes)
		result.Changes = plan.preview.Changes
		if len(plan.writes) == 0 {
			if state, readErr := s.readState(operationCtx, reader, request.Card, request.Keys); readErr == nil {
				result.State = state
			}
			result.Formatted = true
			return nil
		}
		if request.DryRun {
			return nil
		}
		dump, err := s.dumps.ReadCardWithReader(operationCtx, reader, DumpRequest{
			Card: request.Card, Device: request.Device, Keys: plan.dumpKeys,
		})
		if err != nil {
			return nfc.NewError("format Classic NDEF", nfc.CodeIO, "could not read the card for the pre-format backup; the card was not modified", err)
		}
		if !nfc.SameCard(request.Card, dump.Card) {
			return nfc.NewError("format Classic NDEF", nfc.CodeCardChanged, "the card changed while reading the pre-format backup; the card was not modified", nil)
		}
		for _, write := range plan.writes {
			if write.block < 0 || write.block >= len(dump.Dump.Blocks) || !dump.Dump.Blocks[write.block].Complete() {
				return nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("the pre-format backup does not contain block %d; the card was not modified", write.block), nil)
			}
		}
		backupPath, err := s.backups.Save(operationCtx, dump)
		if err != nil {
			return nfc.NewError("format Classic NDEF", nfc.CodeIO, "could not save the pre-format backup; the card was not modified", err)
		}
		result.BackupPath = backupPath
		if err := s.executeWrites(operationCtx, reader, request.Card, plan.writes, &result); err != nil {
			return err
		}
		result.Formatted = true
		final, err := s.readState(operationCtx, reader, request.Card, request.Keys)
		if err != nil {
			return nfc.NewError("verify Classic NDEF format", nfc.CodeVerificationFailed, "the mapping was written but the final read failed", err)
		}
		if !nfc.SameCard(request.Card, final.Card) {
			return nfc.NewError("verify Classic NDEF format", nfc.CodeCardChanged, "the card changed before final verification", nil)
		}
		if !classicNDEFMatches(final, plan.preview.Sectors, plan.desiredArea) {
			return nfc.NewError("verify Classic NDEF format", nfc.CodeVerificationFailed, "the final mapping does not match the planned NDEF application", nil)
		}
		result.State = final
		return nil
	})
	return result, err
}

type classicKeyCandidate struct {
	kind nfc.KeyType
	key  nfc.Key
}

type classicSectorRead struct {
	sector  int
	first   int
	blocks  [][mifare.BlockSize]byte
	trailer [mifare.BlockSize]byte
	keys    VerifiedSectorKeys
	used    classicKeyCandidate
}

type plannedClassicWrite struct {
	block   int
	sector  int
	data    [mifare.BlockSize]byte
	kind    string
	keyType nfc.KeyType
	key     nfc.Key
}

type classicFormatPlan struct {
	preview     ClassicNDEFFormatPlan
	desiredArea []byte
	writes      []plannedClassicWrite
	dumpKeys    []SectorKeys
}

func (s *ClassicNDEFService) readState(ctx context.Context, reader nfc.Reader, card nfc.CardInfo, keys []SectorKeys) (ClassicNDEFState, error) {
	layout, err := layoutForCard(card)
	if err != nil {
		return ClassicNDEFState{}, nfc.NewError("read Classic NDEF", nfc.CodeUnsupported, "card is not MIFARE Classic 1K/4K", err)
	}
	keyMap, err := validateSectorKeys(layout, keys)
	if err != nil {
		return ClassicNDEFState{}, err
	}
	state := ClassicNDEFState{
		Card: card.Clone(), Layout: layout, keyMap: keyMap,
		verified: make(map[int]VerifiedSectorKeys), sectorReads: make(map[int]classicSectorRead),
	}
	sector0, err := s.readSector(ctx, reader, card, layout, mifare.MADSector, keyMap)
	if err != nil {
		return ClassicNDEFState{}, err
	}
	state.sector0 = sector0.blocksArray()
	state.addVerified(sector0.keys)
	state.sectorReads[mifare.MADSector] = sector0

	input := mifare.MADInput{Sector0: state.sector0}
	gpb := state.sector0[3][9]
	if layout == mifare.Classic4K && gpb&0x80 != 0 && gpb&0x03 == 0x02 {
		sector16, readErr := s.readSector(ctx, reader, card, layout, mifare.MAD2Sector, keyMap)
		if readErr != nil {
			return ClassicNDEFState{}, readErr
		}
		blocks := sector16.blocksArray()
		state.addVerified(sector16.keys)
		state.sectorReads[mifare.MAD2Sector] = sector16
		input.Sector16 = &blocks
	}
	mad, err := mifare.ParseMAD(layout, input)
	if err != nil {
		return ClassicNDEFState{}, nfc.NewError("read Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	state.MAD = mad
	state.NDEF = &type2.NDEF{}
	if !mad.Present {
		state.Keys = sortedVerifiedKeys(state.verified)
		return state, nil
	}
	sectors, err := mad.NDEFSectors()
	if err != nil {
		if errors.Is(err, mifare.ErrNoNDEFApplication) {
			state.Keys = sortedVerifiedKeys(state.verified)
			return state, nil
		}
		return ClassicNDEFState{}, nfc.NewError("read Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	state.Application = true
	state.Sectors = sectors
	blocks, err := mifare.NDEFAreaBlocks(layout, sectors)
	if err != nil {
		return ClassicNDEFState{}, nfc.NewError("read Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	state.Blocks = blocks
	state.Capacity = len(blocks) * mifare.BlockSize
	area := make([]byte, 0, state.Capacity)
	for _, sector := range sectors {
		read, readErr := s.readSector(ctx, reader, card, layout, sector, keyMap)
		if readErr != nil {
			return ClassicNDEFState{}, readErr
		}
		trailer := read.blocks[len(read.blocks)-1]
		readOnly, validateErr := mifare.ValidateNDEFGPB(trailer)
		if validateErr != nil {
			return ClassicNDEFState{}, nfc.NewError("read Classic NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("sector %d: %v", sector, validateErr), validateErr)
		}
		state.ReadOnly = state.ReadOnly || readOnly
		state.sectorReads[sector] = read
		state.addVerified(read.keys)
		for _, block := range read.blocks[:len(read.blocks)-1] {
			area = append(area, block[:]...)
		}
	}
	state.Area = area
	state.NDEF, err = type2.ParseNDEF(area)
	if err != nil {
		return ClassicNDEFState{}, nfc.NewError("read Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	state.Keys = sortedVerifiedKeys(state.verified)
	return state, nil
}

func (s *ClassicNDEFService) readSector(ctx context.Context, reader nfc.Reader, card nfc.CardInfo, layout mifare.Layout, sector int, keyMap map[int]SectorKeys) (classicSectorRead, error) {
	first, err := layout.FirstBlock(sector)
	if err != nil {
		return classicSectorRead{}, err
	}
	trailer, err := layout.TrailerBlock(sector)
	if err != nil {
		return classicSectorRead{}, err
	}
	candidates := classicCandidates(keyMap[sector], sector)
	var lastErr error
	for _, candidate := range candidates {
		if err := selectExpectedCard(ctx, reader, card); err != nil {
			return classicSectorRead{}, err
		}
		if err := reader.Authenticate(ctx, byte(first), candidate.kind, candidate.key); err != nil {
			lastErr = err
			if errors.Is(err, nfc.ErrAuthenticationFailed) {
				continue
			}
			return classicSectorRead{}, err
		}
		read := classicSectorRead{sector: sector, first: first, used: candidate}
		setVerifiedKey(&read.keys, sector, candidate)
		for block := first; block <= trailer; block++ {
			data, readErr := reader.ReadBlock(ctx, byte(block))
			if readErr != nil {
				return classicSectorRead{}, nfc.NewError("read Classic NDEF", nfc.CodeIO, fmt.Sprintf("sector %d block %d could not be read", sector, block), readErr)
			}
			read.blocks = append(read.blocks, data)
		}
		read.trailer = read.blocks[len(read.blocks)-1]
		// Record the other known candidates that also authenticate. This
		// gives the formatter a write key without another caller round trip.
		for _, other := range candidates {
			if other.kind == candidate.kind && other.key == candidate.key {
				continue
			}
			if err := selectExpectedCard(ctx, reader, card); err != nil {
				return classicSectorRead{}, err
			}
			if authErr := reader.Authenticate(ctx, byte(first), other.kind, other.key); authErr == nil {
				setVerifiedKey(&read.keys, sector, other)
			} else if !errors.Is(authErr, nfc.ErrAuthenticationFailed) {
				return classicSectorRead{}, authErr
			}
		}
		return read, nil
	}
	if lastErr == nil {
		lastErr = nfc.ErrAuthenticationFailed
	}
	return classicSectorRead{}, nfc.NewError("read Classic NDEF", nfc.CodeAuthenticationFailed,
		fmt.Sprintf("no verified or standard key authenticated sector %d; run classic keyscan or pass --keys", sector), lastErr)
}

func (s *ClassicNDEFService) saveBackup(ctx context.Context, reader nfc.Reader, card nfc.CardInfo, device nfc.DeviceInfo, state ClassicNDEFState) (string, error) {
	dump, err := s.dumps.ReadCardWithReader(ctx, reader, DumpRequest{
		Card: card, Device: device, Keys: classicDumpKeys(state),
	})
	if err != nil {
		return "", nfc.NewError("Classic NDEF backup", nfc.CodeIO, "could not read the current card for the pre-write backup; the card was not modified", err)
	}
	if !nfc.SameCard(card, dump.Card) {
		return "", nfc.NewError("Classic NDEF backup", nfc.CodeCardChanged, "the card changed while reading the pre-write backup; the card was not modified", nil)
	}
	for _, block := range state.Blocks {
		if block < 0 || block >= len(dump.Dump.Blocks) || !dump.Dump.Blocks[block].Complete() {
			return "", nfc.NewError("Classic NDEF backup", nfc.CodeInvalidArgument, fmt.Sprintf("the pre-write backup does not contain NDEF block %d; the card was not modified", block), nil)
		}
	}
	path, err := s.backups.Save(ctx, dump)
	if err != nil {
		return "", nfc.NewError("Classic NDEF backup", nfc.CodeIO, "could not save the pre-write backup; the card was not modified", err)
	}
	return path, nil
}

func planClassicNDEFBlocks(state ClassicNDEFState, desired []byte) ([]plannedClassicWrite, error) {
	if len(desired) != state.Capacity {
		return nil, nfc.NewError("plan Classic NDEF", nfc.CodeInvalidArgument, "message area does not match the NDEF capacity", nil)
	}
	current := classicBlockData(state)
	writes := make([]plannedClassicWrite, 0)
	for index, block := range state.Blocks {
		var want [mifare.BlockSize]byte
		copy(want[:], desired[index*mifare.BlockSize:(index+1)*mifare.BlockSize])
		if current[block] == want {
			continue
		}
		sector, _ := state.Layout.SectorForBlock(block)
		keyType, key, err := chooseClassicWriteKey(state, sector, block, false)
		if err != nil {
			return nil, err
		}
		writes = append(writes, plannedClassicWrite{block: block, sector: sector, data: want, kind: "data", keyType: keyType, key: key})
	}
	return writes, nil
}

func classicNDEFChanges(state ClassicNDEFState, writes []plannedClassicWrite) []type2.Change {
	current := classicBlockData(state)
	changes := make([]type2.Change, 0, len(writes))
	for _, write := range writes {
		changes = append(changes, type2.Change{
			Target: fmt.Sprintf("block %d", write.block),
			Old:    fmt.Sprintf("%X", current[write.block]),
			New:    fmt.Sprintf("%X", write.data),
		})
	}
	return changes
}

func (s *ClassicNDEFService) executeWrites(ctx context.Context, reader nfc.Reader, card nfc.CardInfo, writes []plannedClassicWrite, result *ClassicNDEFWriteResult) error {
	trailerWriter, _ := reader.(nfc.ClassicTrailerWriter)
	for _, write := range writes {
		if err := selectExpectedCard(ctx, reader, card); err != nil {
			return err
		}
		if err := reader.Authenticate(ctx, byte(write.block), write.keyType, write.key); err != nil {
			return nfc.NewError("write Classic NDEF", nfc.CodeAuthenticationFailed, fmt.Sprintf("sector %d could not be authenticated for block %d", write.sector, write.block), err)
		}
		switch write.kind {
		case "trailer":
			if trailerWriter == nil {
				return nfc.NewError("write Classic NDEF", nfc.CodeUnsupported, "the reader cannot write sector trailers", nil)
			}
			if err := trailerWriter.WriteSectorTrailer(ctx, byte(write.block), write.data); err != nil {
				return nfc.NewError("write Classic NDEF", nfc.CodeIO, fmt.Sprintf("sector %d trailer write failed", write.sector), err)
			}
		default:
			if err := reader.WriteBlock(ctx, byte(write.block), write.data); err != nil {
				return nfc.NewError("write Classic NDEF", nfc.CodeIO, fmt.Sprintf("block %d write failed", write.block), err)
			}
		}
		result.Written = append(result.Written, write.block)
		if err := selectExpectedCard(ctx, reader, card); err != nil {
			return err
		}
		verifyType, verifyKey := write.keyType, write.key
		if write.kind == "trailer" {
			// A trailer write may invalidate the authentication that
			// authorized it and changes Key A. Verify with the new public
			// Key A that the formatter wrote.
			verifyType = nfc.KeyTypeA
			copy(verifyKey[:], write.data[0:6])
		}
		if err := reader.Authenticate(ctx, byte(write.block), verifyType, verifyKey); err != nil {
			return nfc.NewError("write Classic NDEF", nfc.CodeAuthenticationFailed, fmt.Sprintf("block %d could not be re-authenticated after writing", write.block), err)
		}
		readback, err := reader.ReadBlock(ctx, byte(write.block))
		if err != nil {
			return nfc.NewError("write Classic NDEF", nfc.CodeVerificationFailed, fmt.Sprintf("block %d could not be read back", write.block), err)
		}
		if !classicWriteMatches(write, readback) {
			return nfc.NewError("write Classic NDEF", nfc.CodeVerificationFailed, fmt.Sprintf("block %d did not read back as written", write.block), nil)
		}
		result.Verified = append(result.Verified, write.block)
	}
	return nil
}

func (s *ClassicNDEFService) buildFormatPlan(ctx context.Context, reader nfc.Reader, card nfc.CardInfo, keys []SectorKeys, sectors []int) (classicFormatPlan, error) {
	layout, err := layoutForCard(card)
	if err != nil {
		return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeUnsupported, "card is not MIFARE Classic 1K/4K", err)
	}
	keyMap, err := validateSectorKeys(layout, keys)
	if err != nil {
		return classicFormatPlan{}, err
	}
	sector0, err := s.readSector(ctx, reader, card, layout, mifare.MADSector, keyMap)
	if err != nil {
		return classicFormatPlan{}, err
	}
	input := mifare.MADInput{Sector0: sector0.blocksArray()}
	var sector16Read *classicSectorRead
	if layout == mifare.Classic4K && sector0.trailer[9]&0x80 != 0 && sector0.trailer[9]&0x03 == 0x02 {
		value, readErr := s.readSector(ctx, reader, card, layout, mifare.MAD2Sector, keyMap)
		if readErr != nil {
			return classicFormatPlan{}, readErr
		}
		sector16Read = &value
		blocks := value.blocksArray()
		input.Sector16 = &blocks
	}
	mad, madErr := mifare.ParseMAD(layout, input)
	if madErr != nil && !errors.Is(madErr, mifare.ErrNoMAD) {
		return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, madErr.Error(), madErr)
	}
	create := !mad.Present
	if create {
		if !isTransportTrailer(sector0.trailer) {
			return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "the card has no MAD and its sector 0 is not in the default transport state; refusing to overwrite it", nil)
		}
		if !isDefaultKeyA(sector0.used) {
			return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "a blank card must authenticate sector 0 with the default Key A", nil)
		}
		if !classicDataZero(sector0.blocks[1:3]) {
			return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "sector 0 contains data but has no MAD; refusing to overwrite it", nil)
		}
		mad = mifare.MAD{Present: true, Version: 1, MultiApplication: true, AIDs: make(map[int]uint16)}
	}
	selected, err := normalizeFormatSectors(layout, mad.Version, sectors)
	if err != nil {
		return classicFormatPlan{}, err
	}
	if !create {
		existing, ndefErr := mad.NDEFSectors()
		if ndefErr != nil && !errors.Is(ndefErr, mifare.ErrNoNDEFApplication) {
			return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, ndefErr.Error(), ndefErr)
		}
		selectedSet := make(map[int]bool, len(selected))
		for _, sector := range selected {
			selectedSet[sector] = true
		}
		for _, sector := range existing {
			if !selectedSet[sector] {
				return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument,
					fmt.Sprintf("sector %d is part of the existing NDEF application; select the complete application range", sector), nil)
			}
		}
	}
	plan := classicFormatPlan{
		preview: ClassicNDEFFormatPlan{
			Card: card.Clone(), Layout: layout, Version: mad.Version, CreateMAD: create,
			Sectors: selected, Trailer: []int{}, MADBlocks: []int{},
		},
		dumpKeys: classicDumpKeysFromKeys(layout, keyMap),
	}
	areaBlocks, err := mifare.NDEFAreaBlocks(layout, selected)
	if err != nil {
		return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	plan.preview.DataBlocks = areaBlocks
	plan.preview.Capacity = len(areaBlocks) * mifare.BlockSize
	if plan.preview.Capacity < 3 {
		return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "the selected sectors are too small for an NDEF area", nil)
	}
	desired := make([]byte, plan.preview.Capacity)
	desired[0], desired[1], desired[2] = 0x03, 0x00, 0xFE
	plan.desiredArea = desired
	current := map[int][mifare.BlockSize]byte{}
	for block, value := range sector0.blocks {
		current[sector0First(layout)+block] = value
	}
	currentData := make(map[int][mifare.BlockSize]byte)
	for block, value := range sector0.blocks {
		currentData[sector0First(layout)+block] = value
	}
	sectorReads := map[int]classicSectorRead{mifare.MADSector: sector0}
	if sector16Read != nil {
		sectorReads[mifare.MAD2Sector] = *sector16Read
		first, _ := layout.FirstBlock(mifare.MAD2Sector)
		for block, value := range sector16Read.blocks {
			currentData[first+block] = value
		}
	}
	for _, sector := range selected {
		read, readErr := s.readSector(ctx, reader, card, layout, sector, keyMap)
		if readErr != nil {
			return classicFormatPlan{}, readErr
		}
		sectorReads[sector] = read
		if create {
			if !isTransportTrailer(read.trailer) {
				return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("sector %d is not in the default transport state", sector), nil)
			}
			if !isDefaultKeyA(read.used) {
				return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("a blank card must authenticate sector %d with the default Key A", sector), nil)
			}
			if !classicDataZero(read.blocks[:len(read.blocks)-1]) {
				return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("sector %d contains data; refusing to overwrite a non-blank card", sector), nil)
			}
		} else if aid := mad.AIDs[sector]; aid != 0x0000 && aid != mifare.NDEFAID {
			return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("sector %d belongs to another application (AID %04X)", sector, aid), nil)
		}
		if err := mad.SetAID(sector, mifare.NDEFAID); err != nil {
			return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
		}
		first, _ := layout.FirstBlock(sector)
		for block, value := range read.blocks {
			currentData[first+block] = value
		}
	}
	// NDEF data blocks.
	for index, block := range areaBlocks {
		var want [mifare.BlockSize]byte
		copy(want[:], desired[index*mifare.BlockSize:(index+1)*mifare.BlockSize])
		if currentData[block] == want {
			continue
		}
		sector, _ := layout.SectorForBlock(block)
		keyType, key, err := chooseClassicWriteKeyFromRead(sectorReads[sector], sector, block, false)
		if err != nil {
			return classicFormatPlan{}, err
		}
		plan.writes = append(plan.writes, plannedClassicWrite{block: block, sector: sector, data: want, kind: "data", keyType: keyType, key: key})
	}
	// NDEF sector trailers, after their data blocks.
	for _, sector := range selected {
		read := sectorReads[sector]
		if ndefTrailerMatches(read) {
			continue
		}
		keyB, err := preservedKeyB(read)
		if err != nil {
			return classicFormatPlan{}, err
		}
		want := ndefTrailerBlock(keyB)
		trailer, _ := layout.TrailerBlock(sector)
		keyType, key, err := chooseClassicWriteKeyFromRead(read, sector, trailer, true)
		if err != nil {
			return classicFormatPlan{}, err
		}
		plan.writes = append(plan.writes, plannedClassicWrite{block: trailer, sector: sector, data: want, kind: "trailer", keyType: keyType, key: key})
		plan.preview.Trailer = append(plan.preview.Trailer, trailer)
		plan.preview.Changes = append(plan.preview.Changes, type2.Change{
			Target: fmt.Sprintf("sector %d trailer", sector),
			Old:    fmt.Sprintf("%X", read.trailer),
			New:    fmt.Sprintf("%X", want),
		})
	}
	madBlocks, err := mad.Blocks()
	if err != nil {
		return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
	}
	// MAD data blocks, then the MAD sector trailer last.
	for index := 0; index < 2; index++ {
		block := sector0First(layout) + 1 + index
		want := madBlocks.Sector0[index]
		if current[block] == want {
			continue
		}
		keyType, key, err := chooseClassicWriteKeyFromRead(sector0, mifare.MADSector, block, false)
		if err != nil {
			return classicFormatPlan{}, err
		}
		plan.writes = append(plan.writes, plannedClassicWrite{block: block, sector: mifare.MADSector, data: want, kind: "data", keyType: keyType, key: key})
		plan.preview.MADBlocks = append(plan.preview.MADBlocks, block)
		plan.preview.Changes = append(plan.preview.Changes, type2.Change{
			Target: fmt.Sprintf("MAD block %d", block),
			Old:    fmt.Sprintf("%X", current[block]),
			New:    fmt.Sprintf("%X", want),
		})
	}
	if madBlocks.Sector16 != nil {
		sector16Data, ok := sectorReads[mifare.MAD2Sector]
		if !ok {
			return classicFormatPlan{}, nfc.NewError("format Classic NDEF", nfc.CodeInternal, "MAD2 sector 16 was not read", nil)
		}
		first, _ := layout.FirstBlock(mifare.MAD2Sector)
		for index := 0; index < 3; index++ {
			block := first + index
			want := madBlocks.Sector16[index]
			if currentData[block] == want {
				continue
			}
			keyType, key, err := chooseClassicWriteKeyFromRead(sector16Data, mifare.MAD2Sector, block, false)
			if err != nil {
				return classicFormatPlan{}, err
			}
			plan.writes = append(plan.writes, plannedClassicWrite{block: block, sector: mifare.MAD2Sector, data: want, kind: "data", keyType: keyType, key: key})
			plan.preview.MADBlocks = append(plan.preview.MADBlocks, block)
		}
	}
	if !madTrailerMatches(sector0, mad.Version) {
		keyB, trailerErr := preservedKeyB(sector0)
		if trailerErr != nil {
			return classicFormatPlan{}, trailerErr
		}
		wantTrailer := madTrailerBlock(mad.Version, keyB)
		keyType, key, err := chooseClassicWriteKeyFromRead(sector0, mifare.MADSector, sector0First(layout)+3, true)
		if err != nil {
			return classicFormatPlan{}, err
		}
		plan.writes = append(plan.writes, plannedClassicWrite{
			block: sector0First(layout) + 3, sector: mifare.MADSector, data: wantTrailer, kind: "trailer", keyType: keyType, key: key,
		})
		plan.preview.Trailer = append(plan.preview.Trailer, sector0First(layout)+3)
		plan.preview.Changes = append(plan.preview.Changes, type2.Change{
			Target: "MAD sector trailer",
			Old:    fmt.Sprintf("%X", sector0.trailer),
			New:    fmt.Sprintf("%X", wantTrailer),
		})
	}
	for _, read := range sectorReads {
		for _, key := range []struct {
			kind nfc.KeyType
			key  *nfc.Key
		}{{nfc.KeyTypeA, read.keys.KeyA}, {nfc.KeyTypeB, read.keys.KeyB}} {
			if key.key != nil {
				plan.dumpKeys = mergeDumpKey(plan.dumpKeys, read.sector, key.kind, *key.key)
			}
		}
	}
	if create {
		plan.preview.Warnings = append(plan.preview.Warnings, "the card had no MAD; a MAD1 directory will be created and sectors 16-39 stay unmanaged")
	}
	if len(plan.preview.Trailer) != 0 {
		plan.preview.Warnings = append(plan.preview.Warnings, "sector trailers will be rewritten; the current Key B is preserved and must be readable or verified")
	}
	return plan, nil
}

func (r classicSectorRead) blocksArray() [4][mifare.BlockSize]byte {
	var result [4][mifare.BlockSize]byte
	for index, block := range r.blocks {
		if index < len(result) {
			result[index] = block
		}
	}
	return result
}

func sector0First(layout mifare.Layout) int {
	first, _ := layout.FirstBlock(mifare.MADSector)
	return first
}

func classicBlockData(state ClassicNDEFState) map[int][mifare.BlockSize]byte {
	result := make(map[int][mifare.BlockSize]byte)
	for _, read := range state.sectorReads {
		first, _ := state.Layout.FirstBlock(read.sector)
		for index, block := range read.blocks {
			result[first+index] = block
		}
	}
	return result
}

func classicCandidates(provided SectorKeys, sector int) []classicKeyCandidate {
	candidates := make([]classicKeyCandidate, 0, 6)
	add := func(kind nfc.KeyType, key *nfc.Key) {
		if key == nil {
			return
		}
		for _, existing := range candidates {
			if existing.kind == kind && existing.key == *key {
				return
			}
		}
		candidates = append(candidates, classicKeyCandidate{kind: kind, key: *key})
	}
	add(nfc.KeyTypeA, provided.KeyA)
	add(nfc.KeyTypeB, provided.KeyB)
	public := mifare.NDEFPublicKey
	if sector == mifare.MADSector || sector == mifare.MAD2Sector {
		public = mifare.MADPublicKey
	}
	add(nfc.KeyTypeA, &public)
	transport := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	add(nfc.KeyTypeA, &transport)
	add(nfc.KeyTypeB, &transport)
	return candidates
}

func chooseClassicWriteKey(state ClassicNDEFState, sector, block int, trailer bool) (nfc.KeyType, nfc.Key, error) {
	read, ok := state.sectorReads[sector]
	if !ok {
		return 0, nfc.Key{}, nfc.NewError("plan Classic NDEF", nfc.CodeInternal, fmt.Sprintf("sector %d was not read", sector), nil)
	}
	return chooseClassicWriteKeyFromRead(read, sector, block, trailer)
}

func chooseClassicWriteKeyFromRead(read classicSectorRead, sector, block int, trailer bool) (nfc.KeyType, nfc.Key, error) {
	access, err := mifare.DecodeAccessBits([3]byte{read.trailer[6], read.trailer[7], read.trailer[8]})
	if err != nil {
		return 0, nfc.Key{}, nfc.NewError("plan Classic NDEF", nfc.CodeInvalidArgument, fmt.Sprintf("sector %d has invalid access bits", sector), err)
	}
	var mask mifare.KeyMask
	if trailer {
		mask = access.Groups[3].FullTrailerWriteKeys()
	} else {
		group, groupErr := layoutAccessGroup(read, block)
		if groupErr != nil {
			return 0, nfc.Key{}, groupErr
		}
		mask = access.Groups[group].DataWriteKeys()
	}
	if read.keys.KeyA != nil && mask.AllowsA() {
		return nfc.KeyTypeA, *read.keys.KeyA, nil
	}
	if read.keys.KeyB != nil && mask.AllowsB() {
		return nfc.KeyTypeB, *read.keys.KeyB, nil
	}
	return 0, nfc.Key{}, nfc.NewError("plan Classic NDEF", nfc.CodePermission, fmt.Sprintf("no verified key can write sector %d block %d", sector, block), nil)
}

func layoutAccessGroup(read classicSectorRead, block int) (int, error) {
	count := len(read.blocks)
	if count == 0 {
		return 0, nfc.NewError("plan Classic NDEF", nfc.CodeInternal, "sector geometry is missing", nil)
	}
	if count == 16 {
		return (block - read.first) / 5, nil
	}
	return block - read.first, nil
}

func preservedKeyB(read classicSectorRead) (nfc.Key, error) {
	access, err := mifare.DecodeAccessBits([3]byte{read.trailer[6], read.trailer[7], read.trailer[8]})
	if err != nil {
		return nfc.Key{}, nfc.NewError("plan Classic NDEF", nfc.CodeInvalidArgument, "sector trailer has invalid access bits", err)
	}
	if access.Groups[3].KeyBReadable() {
		var key nfc.Key
		copy(key[:], read.trailer[10:16])
		return key, nil
	}
	if read.keys.KeyB != nil {
		return *read.keys.KeyB, nil
	}
	return nfc.Key{}, nfc.NewError("plan Classic NDEF", nfc.CodePermission, fmt.Sprintf("sector %d Key B is secret and not verified; refusing to rewrite its trailer", read.sector), nil)
}

func isTransportTrailer(trailer [mifare.BlockSize]byte) bool {
	return trailer[6] == 0xFF && trailer[7] == 0x07 && trailer[8] == 0x80
}

func isDefaultKeyA(candidate classicKeyCandidate) bool {
	defaultKey := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	return candidate.kind == nfc.KeyTypeA && candidate.key == defaultKey
}

func ndefTrailerBlock(keyB nfc.Key) [mifare.BlockSize]byte {
	var trailer [mifare.BlockSize]byte
	copy(trailer[0:6], []byte{0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7})
	trailer[6], trailer[7], trailer[8], trailer[9] = 0x7F, 0x07, 0x88, 0x40
	copy(trailer[10:16], keyB[:])
	return trailer
}

func madTrailerBlock(version int, keyB nfc.Key) [mifare.BlockSize]byte {
	var trailer [mifare.BlockSize]byte
	copy(trailer[0:6], []byte{0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5})
	trailer[6], trailer[7], trailer[8], trailer[9] = 0x78, 0x77, 0x88, mifare.MADGPB(version)
	copy(trailer[10:16], keyB[:])
	return trailer
}

func normalizeFormatSectors(layout mifare.Layout, version int, sectors []int) ([]int, error) {
	if len(sectors) == 0 {
		return nil, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "at least one sector is required", nil)
	}
	result := append([]int(nil), sectors...)
	sort.Ints(result)
	first, last := result[0], result[len(result)-1]
	if first < 1 || last > 15 && version == 1 {
		return nil, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "MAD1 formatting accepts sectors 1-15", nil)
	}
	if version == 2 && last > 39 {
		return nil, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "MAD2 formatting accepts sectors 1-15 and 17-39", nil)
	}
	if first <= 15 && last >= 17 {
		return nil, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "format one contiguous side of the MAD sector 16", nil)
	}
	for index, sector := range result {
		if sector == mifare.MAD2Sector {
			return nil, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "sector 16 is a MAD sector and cannot store NDEF", nil)
		}
		if index > 0 && sector != result[index-1]+1 {
			return nil, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, "NDEF sectors must be contiguous", nil)
		}
		if _, err := layout.FirstBlock(sector); err != nil {
			return nil, nfc.NewError("format Classic NDEF", nfc.CodeInvalidArgument, err.Error(), err)
		}
	}
	return result, nil
}

func plannedBlockNumbers(writes []plannedClassicWrite) []int {
	result := make([]int, 0, len(writes))
	for _, write := range writes {
		result = append(result, write.block)
	}
	return result
}

func classicNDEFMatches(state ClassicNDEFState, sectors []int, area []byte) bool {
	if !state.Application || !bytes.Equal(state.Area, area) || len(state.Sectors) != len(sectors) {
		return false
	}
	for index := range sectors {
		if state.Sectors[index] != sectors[index] {
			return false
		}
	}
	for _, sector := range sectors {
		if state.MAD.AIDs[sector] != mifare.NDEFAID {
			return false
		}
	}
	return true
}

func setVerifiedKey(keys *VerifiedSectorKeys, sector int, candidate classicKeyCandidate) {
	key := candidate.key
	keys.Sector = sector
	if candidate.kind == nfc.KeyTypeA {
		keys.KeyA = &key
	} else {
		keys.KeyB = &key
	}
}

func (s *ClassicNDEFState) addVerified(keys VerifiedSectorKeys) {
	if keys.KeyA != nil {
		key := *keys.KeyA
		s.verified[keys.Sector] = mergeVerified(s.verified[keys.Sector], keys.Sector, nfc.KeyTypeA, key)
	}
	if keys.KeyB != nil {
		key := *keys.KeyB
		s.verified[keys.Sector] = mergeVerified(s.verified[keys.Sector], keys.Sector, nfc.KeyTypeB, key)
	}
}

func mergeVerified(existing VerifiedSectorKeys, sector int, kind nfc.KeyType, key nfc.Key) VerifiedSectorKeys {
	existing.Sector = sector
	if kind == nfc.KeyTypeA {
		value := key
		existing.KeyA = &value
	} else {
		value := key
		existing.KeyB = &value
	}
	return existing
}

func sortedVerifiedKeys(values map[int]VerifiedSectorKeys) []VerifiedSectorKeys {
	sectors := make([]int, 0, len(values))
	for sector := range values {
		sectors = append(sectors, sector)
	}
	sort.Ints(sectors)
	result := make([]VerifiedSectorKeys, 0, len(sectors))
	for _, sector := range sectors {
		result = append(result, values[sector])
	}
	return result
}

func classicDumpKeys(state ClassicNDEFState) []SectorKeys {
	keys := classicDumpKeysFromKeys(state.Layout, state.keyMap)
	for sector, verified := range state.verified {
		if verified.KeyA != nil {
			keys = mergeDumpKey(keys, sector, nfc.KeyTypeA, *verified.KeyA)
		}
		if verified.KeyB != nil {
			keys = mergeDumpKey(keys, sector, nfc.KeyTypeB, *verified.KeyB)
		}
	}
	return keys
}

func classicDumpKeysFromKeys(layout mifare.Layout, keyMap map[int]SectorKeys) []SectorKeys {
	sectors, _ := layout.SectorCount()
	result := make([]SectorKeys, sectors)
	transport := nfc.Key{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}
	for sector := range result {
		result[sector].Sector = sector
		if provided, ok := keyMap[sector]; ok {
			if provided.KeyA != nil {
				key := *provided.KeyA
				result[sector].KeyA = &key
			}
			if provided.KeyB != nil {
				key := *provided.KeyB
				result[sector].KeyB = &key
			}
		}
		if result[sector].KeyA == nil && result[sector].KeyB == nil {
			keyA, keyB := transport, transport
			result[sector].KeyA = &keyA
			result[sector].KeyB = &keyB
		}
	}
	return result
}

func mergeDumpKey(keys []SectorKeys, sector int, kind nfc.KeyType, key nfc.Key) []SectorKeys {
	if sector < 0 || sector >= len(keys) {
		return keys
	}
	value := key
	if kind == nfc.KeyTypeA {
		keys[sector].KeyA = &value
	} else {
		keys[sector].KeyB = &value
	}
	return keys
}

func ndefTrailerMatches(read classicSectorRead) bool {
	desired := ndefTrailerBlock(nfc.Key{})
	if !bytes.Equal(read.trailer[6:10], desired[6:10]) {
		return false
	}
	// Key A is always masked on a trailer read; the only proof is that the
	// standardized public Key A authenticated the sector.
	return read.keys.KeyA != nil && *read.keys.KeyA == mifare.NDEFPublicKey
}

func madTrailerMatches(read classicSectorRead, version int) bool {
	desired := madTrailerBlock(version, nfc.Key{})
	if !bytes.Equal(read.trailer[6:10], desired[6:10]) {
		return false
	}
	return read.keys.KeyA != nil && *read.keys.KeyA == mifare.MADPublicKey
}

func classicWriteMatches(write plannedClassicWrite, readback [mifare.BlockSize]byte) bool {
	if write.kind != "trailer" {
		return readback == write.data
	}
	mask := uint16(0x03C0)
	access, err := mifare.DecodeAccessBits([3]byte{write.data[6], write.data[7], write.data[8]})
	if err == nil && access.Groups[3].KeyBReadable() {
		mask |= 0xFC00
	}
	for index := 0; index < mifare.BlockSize; index++ {
		if mask&(1<<index) == 0 {
			continue
		}
		if readback[index] != write.data[index] {
			return false
		}
	}
	return true
}

func classicDataZero(blocks [][mifare.BlockSize]byte) bool {
	for _, block := range blocks {
		for _, value := range block {
			if value != 0 {
				return false
			}
		}
	}
	return true
}
