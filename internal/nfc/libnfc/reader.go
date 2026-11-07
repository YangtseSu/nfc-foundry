// Package libnfc implements the NFC reader contracts through a narrow C shim.
// Native support is compiled only with the "libnfc" build tag.
package libnfc

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"

	"github.com/YangtseSu/nfc-foundry/internal/mifare"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// Backend owns the native implementation used to enumerate and create readers.
type Backend struct {
	native nativeDriver
}

// NewBackend creates a backend for the current build. Without the libnfc build
// tag it remains available but returns nfc.ErrUnsupported from native calls.
func NewBackend() *Backend {
	return &Backend{native: platformNative()}
}

// NewReader creates a closed libnfc Reader.
func NewReader() *Reader {
	return NewBackend().NewReader()
}

// NewReader creates a closed Reader sharing this backend's native driver.
func (b *Backend) NewReader() *Reader {
	return &Reader{native: b.native}
}

// ListDevices returns discovered connstrings. libnfc cannot reliably provide a
// friendly name without opening a device, so Name falls back to ConnString.
func (b *Backend) ListDevices(ctx context.Context) ([]nfc.DeviceInfo, error) {
	if err := contextError(ctx, "list devices"); err != nil {
		return nil, err
	}
	devices, result := b.native.listDevices()
	if err := contextError(ctx, "list devices"); err != nil {
		return nil, err
	}
	if err := result.err("list devices"); err != nil {
		return nil, err
	}
	return append([]nfc.DeviceInfo(nil), devices...), nil
}

// Reader serializes every ordinary call for one native device. abort is the
// sole native operation allowed to run concurrently with an in-flight call.
type Reader struct {
	native   nativeDriver
	opMu     sync.Mutex
	closeMu  sync.Mutex
	abortMu  sync.Mutex
	stateMu  sync.Mutex
	handle   nativeHandle
	closing  bool
	inFlight bool
	device   nfc.DeviceInfo
	card     *nfc.CardInfo
}

// Open initializes libnfc, opens the exact connstring, and initializes the
// device in initiator mode. Empty or NUL-containing connstrings are rejected.
func (r *Reader) Open(ctx context.Context, connString string) error {
	if err := contextError(ctx, "open"); err != nil {
		return err
	}
	if connString == "" || strings.IndexByte(connString, 0) >= 0 {
		return nfc.NewError("open", nfc.CodeInvalidArgument, "connstring must be non-empty and contain no NUL byte", nil)
	}

	r.opMu.Lock()
	defer r.opMu.Unlock()

	r.stateMu.Lock()
	if r.closing {
		r.stateMu.Unlock()
		return nfc.NewError("open", nfc.CodeBusy, "reader is closing", nil)
	}
	if r.handle != nil {
		r.stateMu.Unlock()
		return nfc.NewError("open", nfc.CodeBusy, "reader is already open", nil)
	}
	r.stateMu.Unlock()

	handle, result := r.native.open(connString)
	if err := result.err("open"); err != nil {
		if errors.Is(err, nfc.ErrDeviceDisconnected) {
			if permissionErr := connStringPermissionError(connString); permissionErr != nil {
				return permissionErr
			}
		}
		return wrapOpenHint(connString, err)
	}
	if err := contextError(ctx, "open"); err != nil {
		_ = r.native.close(handle)
		return err
	}

	r.stateMu.Lock()
	if r.closing {
		r.stateMu.Unlock()
		_ = r.native.close(handle)
		return nfc.NewError("open", nfc.CodeCanceled, "reader was closed while opening", nil)
	}
	r.handle = handle
	r.device = nfc.DeviceInfo{Name: connString, ConnString: connString}
	r.card = nil
	if name, nameResult := r.native.deviceName(handle); nameResult.code == nativeOK && name != "" {
		r.device.Name = name
	}
	r.stateMu.Unlock()
	return nil
}

// DeviceInfo returns a copy of the selected connstring and the friendly name
// libnfc exposes after the device has been opened.
func (r *Reader) DeviceInfo() nfc.DeviceInfo {
	r.stateMu.Lock()
	defer r.stateMu.Unlock()
	return r.device
}

// Close aborts any in-flight command, waits for it to return, and releases the
// device and context. It is safe to call repeatedly.
func (r *Reader) Close() error {
	r.closeMu.Lock()
	defer r.closeMu.Unlock()

	r.stateMu.Lock()
	r.closing = true
	handle := r.handle
	inFlight := r.inFlight
	r.stateMu.Unlock()
	if handle != nil && inFlight {
		r.abort(handle)
	}

	r.opMu.Lock()
	r.stateMu.Lock()
	handle = r.handle
	r.handle = nil
	r.device = nfc.DeviceInfo{}
	r.card = nil
	r.inFlight = false
	r.stateMu.Unlock()

	result := nativeResult{code: nativeOK}
	if handle != nil {
		result = r.native.close(handle)
	}
	r.opMu.Unlock()

	r.stateMu.Lock()
	r.closing = false
	r.stateMu.Unlock()
	return result.err("close")
}

// CardInfo selects one ISO/IEC 14443A target and returns copied UID, ATQA and
// SAK values.
func (r *Reader) CardInfo(ctx context.Context) (nfc.CardInfo, error) {
	if err := r.requireCapability("card info", func(c nfc.ReaderCapabilities) bool { return c.SelectISO14443A }, "ISO/IEC 14443A card selection"); err != nil {
		return nfc.CardInfo{}, err
	}
	handle, finish, err := r.beginOperation(ctx, "card info")
	if err != nil {
		return nfc.CardInfo{}, err
	}
	defer finish()
	card, result := r.native.cardInfo(handle)
	if err := operationError(ctx, "card info", result); err != nil {
		r.clearSelectedCard()
		return nfc.CardInfo{}, err
	}
	if length := len(card.UID); length != 4 && length != 7 && length != nfc.MaxUIDLength {
		r.clearSelectedCard()
		return nfc.CardInfo{}, nfc.NewError("card info", nfc.CodeInternal, "libnfc returned an invalid ISO14443A UID length", nil)
	}
	selected := card.Clone()
	r.stateMu.Lock()
	r.card = &selected
	r.stateMu.Unlock()
	return card.Clone(), nil
}

// Authenticate selects Key A or Key B for the sector containing block.
func (r *Reader) Authenticate(ctx context.Context, block byte, keyType nfc.KeyType, key nfc.Key) error {
	if err := r.requireCapability("authenticate", func(c nfc.ReaderCapabilities) bool { return c.ClassicRead }, "MIFARE Classic read operations"); err != nil {
		return err
	}
	command, valid := authenticationCommand(keyType)
	if !valid {
		return nfc.NewError("authenticate", nfc.CodeInvalidArgument, "unknown key type", nil)
	}
	handle, finish, err := r.beginOperation(ctx, "authenticate")
	if err != nil {
		return err
	}
	defer finish()
	if _, err := r.selectedLayoutForBlock("authenticate", block); err != nil {
		return err
	}
	result := r.native.authenticate(handle, block, command, key)
	err = operationError(ctx, "authenticate", result)
	r.invalidateSelectedCardOn(err)
	return err
}

// ReadBlock reads one authenticated 16-byte MIFARE Classic block.
func (r *Reader) ReadBlock(ctx context.Context, block byte) ([nfc.BlockSize]byte, error) {
	if err := r.requireCapability("read block", func(c nfc.ReaderCapabilities) bool { return c.ClassicRead }, "MIFARE Classic read operations"); err != nil {
		return [nfc.BlockSize]byte{}, err
	}
	handle, finish, err := r.beginOperation(ctx, "read block")
	if err != nil {
		return [nfc.BlockSize]byte{}, err
	}
	defer finish()
	if _, err := r.selectedLayoutForBlock("read block", block); err != nil {
		return [nfc.BlockSize]byte{}, err
	}
	data, result := r.native.readBlock(handle, block)
	if err := operationError(ctx, "read block", result); err != nil {
		r.invalidateSelectedCardOn(err)
		return [nfc.BlockSize]byte{}, err
	}
	return data, nil
}

// WriteBlock writes one ordinary data block and verifies it by immediate
// readback. Manufacturer and sector trailer blocks require separate protected
// workflows and are rejected here.
func (r *Reader) WriteBlock(ctx context.Context, block byte, data [nfc.BlockSize]byte) error {
	if err := r.requireCapability("write block", func(c nfc.ReaderCapabilities) bool { return c.ClassicWrite }, "MIFARE Classic write operations"); err != nil {
		return err
	}
	if block == 0 {
		return nfc.NewError("write block", nfc.CodeInvalidArgument, "ordinary writes to manufacturer block 0 are forbidden", nil)
	}
	handle, finish, err := r.beginOperation(ctx, "write block")
	if err != nil {
		return err
	}
	defer finish()
	layout, err := r.selectedLayoutForBlock("write block", block)
	if err != nil {
		return err
	}
	isTrailer, _ := layout.IsTrailer(int(block))
	if isTrailer {
		return nfc.NewError("write block", nfc.CodeInvalidArgument, "sector trailer writes require an explicit protected workflow", nil)
	}
	result := r.native.writeBlock(handle, block, data)
	// Once the native write reports success, a late context cancellation must
	// not create a Go-side gap before verification. The cancellation watcher
	// can still abort a blocked native read, and Close can still abort directly.
	if err := result.err("write block"); err != nil {
		r.invalidateSelectedCardOn(err)
		return err
	}
	actual, result := r.native.readBlock(handle, block)
	if err := result.err("verify block"); err != nil {
		r.invalidateSelectedCardOn(err)
		return err
	}
	if actual != data {
		return nfc.NewError("verify block", nfc.CodeVerificationFailed, "written block does not match its immediate readback", nil)
	}
	return nil
}

// WriteManufacturerBlock sends one authenticated ordinary MIFARE write to
// block 0. It intentionally performs no same-session readback because changing
// the UID invalidates the selected target identity. The protected UID workflow
// must reselect and verify the card before it reports success.
func (r *Reader) WriteManufacturerBlock(ctx context.Context, data [nfc.BlockSize]byte) error {
	if err := r.requireCapability("write manufacturer block", func(c nfc.ReaderCapabilities) bool { return c.ClassicWrite }, "MIFARE Classic write operations"); err != nil {
		return err
	}
	handle, finish, err := r.beginOperation(ctx, "write manufacturer block")
	if err != nil {
		return err
	}
	defer finish()
	if _, err := r.selectedLayoutForBlock("write manufacturer block", 0); err != nil {
		return err
	}
	result := r.native.writeManufacturerBlock(handle, data)
	err = result.err("write manufacturer block")
	r.invalidateSelectedCardOn(err)
	if err == nil {
		r.clearSelectedCard()
	}
	return err
}

// SetRawMode enters or leaves the raw-frame transport used by bit-level card
// protocols. Leaving raw mode always clears the selected card so the next
// CardInfo performs a fresh selection.
func (r *Reader) SetRawMode(ctx context.Context, enabled bool) error {
	if err := r.requireCapability("set raw mode", func(c nfc.ReaderCapabilities) bool { return c.RawBytes || c.RawBits }, "raw-frame mode"); err != nil {
		return err
	}
	handle, finish, err := r.beginOperation(ctx, "set raw mode")
	if err != nil {
		return err
	}
	defer finish()
	result := r.native.setRawMode(handle, enabled)
	err = operationError(ctx, "set raw mode", result)
	if !enabled || err == nil {
		r.clearSelectedCard()
	}
	return err
}

// TransceiveBytes sends one manually framed byte sequence after the caller has
// enabled raw mode. The returned buffer is owned by Go.
func (r *Reader) TransceiveBytes(ctx context.Context, tx []byte) ([]byte, error) {
	if err := r.requireCapability("transceive bytes", func(c nfc.ReaderCapabilities) bool { return c.RawBytes }, "raw byte frames"); err != nil {
		return nil, err
	}
	if len(tx) == 0 {
		return nil, nfc.NewError("transceive bytes", nfc.CodeInvalidArgument, "raw frame must not be empty", nil)
	}
	handle, finish, err := r.beginOperation(ctx, "transceive bytes")
	if err != nil {
		return nil, err
	}
	defer finish()
	response, result := r.native.transceiveBytes(handle, tx)
	if err := operationError(ctx, "transceive bytes", result); err != nil {
		return nil, err
	}
	return response, nil
}

// TransceiveBits sends a bit-level frame after the caller has enabled raw
// mode. It returns the copied response bytes and the number of valid response
// bits.
func (r *Reader) TransceiveBits(ctx context.Context, tx []byte, txBits int) ([]byte, int, error) {
	if err := r.requireCapability("transceive bits", func(c nfc.ReaderCapabilities) bool { return c.RawBits }, "raw bit frames"); err != nil {
		return nil, 0, err
	}
	if len(tx) == 0 || txBits <= 0 || txBits > len(tx)*8 {
		return nil, 0, nfc.NewError("transceive bits", nfc.CodeInvalidArgument, "invalid raw bit frame", nil)
	}
	handle, finish, err := r.beginOperation(ctx, "transceive bits")
	if err != nil {
		return nil, 0, err
	}
	defer finish()
	response, bits, result := r.native.transceiveBits(handle, tx, txBits)
	if err := operationError(ctx, "transceive bits", result); err != nil {
		return nil, 0, err
	}
	return response, bits, nil
}

// WriteSectorTrailer performs only the native 16-byte trailer write. The
// protected restore workflow is responsible for access-bit validation,
// identity checks, authentication with the current key, and verification with
// the replacement keys. Keeping this separate preserves the ordinary write
// API's block-0 and trailer protections.
func (r *Reader) WriteSectorTrailer(ctx context.Context, block byte, data [nfc.BlockSize]byte) error {
	if err := r.requireCapability("write sector trailer", func(c nfc.ReaderCapabilities) bool { return c.ClassicWrite }, "MIFARE Classic write operations"); err != nil {
		return err
	}
	handle, finish, err := r.beginOperation(ctx, "write sector trailer")
	if err != nil {
		return err
	}
	defer finish()
	layout, err := r.selectedLayoutForBlock("write sector trailer", block)
	if err != nil {
		return err
	}
	isTrailer, _ := layout.IsTrailer(int(block))
	if !isTrailer {
		return nfc.NewError("write sector trailer", nfc.CodeInvalidArgument, "block is not a sector trailer", nil)
	}
	result := r.native.writeBlock(handle, block, data)
	err = result.err("write sector trailer")
	r.invalidateSelectedCardOn(err)
	return err
}

var _ nfc.ClassicTrailerWriter = (*Reader)(nil)
var _ nfc.ClassicManufacturerWriter = (*Reader)(nil)
var _ nfc.RawTransceiver = (*Reader)(nil)

// requireCapability rejects operations the selected driver profile does not
// implement before any native command is sent.
func (r *Reader) requireCapability(op string, allowed func(nfc.ReaderCapabilities) bool, feature string) error {
	r.stateMu.Lock()
	connString := r.device.ConnString
	r.stateMu.Unlock()
	if allowed(nfc.CapabilitiesFor(connString)) {
		return nil
	}
	return nfc.NewError(op, nfc.CodeUnsupported, fmt.Sprintf("reader %q does not support %s", connString, feature), nil)
}

func (r *Reader) selectedLayoutForBlock(op string, block byte) (mifare.Layout, error) {
	r.stateMu.Lock()
	var card *nfc.CardInfo
	if r.card != nil {
		clone := r.card.Clone()
		card = &clone
	}
	r.stateMu.Unlock()
	if card == nil {
		return 0, nfc.NewError(op, nfc.CodeNoCard, "select a card before a MIFARE Classic operation", nil)
	}

	var layout mifare.Layout
	switch nfc.InferCardType(*card) {
	case nfc.CardTypeMIFAREClassic1K:
		layout = mifare.Classic1K
	case nfc.CardTypeMIFAREClassic4K:
		layout = mifare.Classic4K
	default:
		return 0, nfc.NewError(op, nfc.CodeUnsupported, "selected card is not a supported MIFARE Classic 1K/4K card", nil)
	}
	blockCount, _ := layout.BlockCount()
	if int(block) >= blockCount {
		return 0, nfc.NewError(op, nfc.CodeInvalidArgument, "block is outside the selected card capacity", nil)
	}
	return layout, nil
}

func (r *Reader) clearSelectedCard() {
	r.stateMu.Lock()
	r.card = nil
	r.stateMu.Unlock()
}

func (r *Reader) invalidateSelectedCardOn(err error) {
	if err == nil || errors.Is(err, nfc.ErrAuthenticationFailed) ||
		errors.Is(err, nfc.ErrNotAuthenticated) || errors.Is(err, nfc.ErrInvalidArgument) ||
		errors.Is(err, nfc.ErrUnsupported) || errors.Is(err, nfc.ErrVerificationFailed) {
		return
	}
	r.clearSelectedCard()
}

func (r *Reader) beginOperation(ctx context.Context, op string) (nativeHandle, func(), error) {
	if err := contextError(ctx, op); err != nil {
		return nil, nil, err
	}
	r.opMu.Lock()
	r.stateMu.Lock()
	if r.closing {
		r.stateMu.Unlock()
		r.opMu.Unlock()
		return nil, nil, nfc.NewError(op, nfc.CodeBusy, "reader is closing", nil)
	}
	handle := r.handle
	if handle != nil {
		r.inFlight = true
	}
	r.stateMu.Unlock()
	if handle == nil {
		r.opMu.Unlock()
		return nil, nil, nfc.NewError(op, nfc.CodeNotOpen, "", nil)
	}

	stopWatching := r.watchCancellation(ctx, handle)
	finish := func() {
		stopWatching()
		r.stateMu.Lock()
		r.inFlight = false
		r.stateMu.Unlock()
		r.opMu.Unlock()
	}
	return handle, finish, nil
}

func (r *Reader) watchCancellation(ctx context.Context, handle nativeHandle) func() {
	if ctx.Done() == nil {
		return func() {}
	}
	done := make(chan struct{})
	watcherDone := make(chan struct{})
	go func() {
		defer close(watcherDone)
		select {
		case <-ctx.Done():
			r.abort(handle)
		case <-done:
		}
	}()
	return func() {
		close(done)
		<-watcherDone
	}
}

func (r *Reader) abort(handle nativeHandle) {
	r.abortMu.Lock()
	defer r.abortMu.Unlock()
	_ = r.native.abort(handle)
}

func contextError(ctx context.Context, op string) error {
	if ctx == nil {
		return nfc.NewError(op, nfc.CodeInvalidArgument, "nil context", nil)
	}
	if err := ctx.Err(); err != nil {
		code := nfc.CodeCanceled
		if err == context.DeadlineExceeded {
			code = nfc.CodeTimeout
		}
		return nfc.NewError(op, code, "", err)
	}
	return nil
}

func operationError(ctx context.Context, op string, result nativeResult) error {
	if err := contextError(ctx, op); err != nil {
		return err
	}
	return result.err(op)
}

var (
	_ nfc.DeviceEnumerator = (*Backend)(nil)
	_ nfc.Reader           = (*Reader)(nil)
	_ nfc.DeviceDescriber  = (*Reader)(nil)
)
