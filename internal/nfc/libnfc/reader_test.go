package libnfc

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

type fakeHandle struct{}

func (*fakeHandle) isNativeHandle() {}

type fakeNative struct {
	mu sync.Mutex

	devices                 []nfc.DeviceInfo
	listResult              nativeResult
	openResult              nativeResult
	name                    string
	card                    nfc.CardInfo
	cardResult              nativeResult
	authResult              nativeResult
	readData                [nfc.BlockSize]byte
	readResult              nativeResult
	writeResult             nativeResult
	manufacturerWriteResult nativeResult
	rawModeResult           nativeResult
	rawModeState            bool
	rawModeCalls            []bool
	transceiveBytesResult   nativeResult
	transceiveBytesResponse []byte
	transceiveFrames        [][]byte
	transceiveBitsResult    nativeResult
	transceiveBitsResponse  []byte
	transceiveBitsCount     int
	transceiveBitFrames     []rawBitFrame
	authBlock               byte
	authCommand             byte
	authKey                 nfc.Key
	readBlocks              []byte
	writtenBlock            byte
	written                 [nfc.BlockSize]byte
	manufacturerWritten     [nfc.BlockSize]byte
	authCalls               int
	writeCalls              int
	enforceAuth             bool
	authenticated           bool

	opens  int
	closes int
	aborts int
	live   int

	cardStarted chan struct{}
	abortCard   chan struct{}
	startOnce   sync.Once
	abortOnce   sync.Once
}

func (f *fakeNative) listDevices() ([]nfc.DeviceInfo, nativeResult) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]nfc.DeviceInfo(nil), f.devices...), f.listResult
}

func (f *fakeNative) open(string) (nativeHandle, nativeResult) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.opens++
	if f.openResult.code != nativeOK {
		return nil, f.openResult
	}
	f.live++
	return &fakeHandle{}, nativeResult{code: nativeOK}
}

func (f *fakeNative) deviceName(nativeHandle) (string, nativeResult) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.name == "" {
		return "mock reader", nativeResult{code: nativeOK}
	}
	return f.name, nativeResult{code: nativeOK}
}

func (f *fakeNative) close(nativeHandle) nativeResult {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.closes++
	f.live--
	return nativeResult{code: nativeOK}
}

func (f *fakeNative) abort(nativeHandle) nativeResult {
	f.mu.Lock()
	f.aborts++
	abortCard := f.abortCard
	f.mu.Unlock()
	if abortCard != nil {
		f.abortOnce.Do(func() { close(abortCard) })
	}
	return nativeResult{code: nativeOK}
}

func (f *fakeNative) cardInfo(nativeHandle) (nfc.CardInfo, nativeResult) {
	f.mu.Lock()
	if f.enforceAuth {
		f.authenticated = false
	}
	card := f.card.Clone()
	result := f.cardResult
	started := f.cardStarted
	abortCard := f.abortCard
	f.mu.Unlock()
	if started != nil {
		f.startOnce.Do(func() { close(started) })
	}
	if abortCard != nil {
		<-abortCard
		return nfc.CardInfo{}, nativeResult{code: nativeCanceled}
	}
	return card, result
}

func (f *fakeNative) authenticate(_ nativeHandle, block byte, command byte, key nfc.Key) nativeResult {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.authCalls++
	f.authBlock = block
	f.authCommand = command
	f.authKey = key
	if f.authResult.code == nativeOK {
		f.authenticated = true
	}
	return f.authResult
}

func (f *fakeNative) readBlock(_ nativeHandle, block byte) ([nfc.BlockSize]byte, nativeResult) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.readBlocks = append(f.readBlocks, block)
	if f.enforceAuth && !f.authenticated {
		return [nfc.BlockSize]byte{}, nativeResult{code: nativeNotAuthenticated}
	}
	return f.readData, f.readResult
}

func (f *fakeNative) writeBlock(_ nativeHandle, block byte, data [nfc.BlockSize]byte) nativeResult {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.writeCalls++
	f.writtenBlock = block
	f.written = data
	if f.enforceAuth && !f.authenticated {
		return nativeResult{code: nativeNotAuthenticated}
	}
	return f.writeResult
}

func (f *fakeNative) writeManufacturerBlock(_ nativeHandle, data [nfc.BlockSize]byte) nativeResult {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.writeCalls++
	f.writtenBlock = 0
	f.manufacturerWritten = data
	if f.enforceAuth && !f.authenticated {
		return nativeResult{code: nativeNotAuthenticated}
	}
	return f.manufacturerWriteResult
}

type rawBitFrame struct {
	data []byte
	bits int
}

func (f *fakeNative) setRawMode(_ nativeHandle, enabled bool) nativeResult {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.rawModeCalls = append(f.rawModeCalls, enabled)
	if f.rawModeResult.code == nativeOK {
		f.rawModeState = enabled
	}
	return f.rawModeResult
}

func (f *fakeNative) transceiveBytes(_ nativeHandle, tx []byte) ([]byte, nativeResult) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.transceiveFrames = append(f.transceiveFrames, append([]byte(nil), tx...))
	if f.transceiveBytesResult.code != nativeOK {
		return nil, f.transceiveBytesResult
	}
	return append([]byte(nil), f.transceiveBytesResponse...), nativeResult{code: nativeOK}
}

func (f *fakeNative) transceiveBits(_ nativeHandle, tx []byte, bits int) ([]byte, int, nativeResult) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.transceiveBitFrames = append(f.transceiveBitFrames, rawBitFrame{data: append([]byte(nil), tx...), bits: bits})
	if f.transceiveBitsResult.code != nativeOK {
		return nil, 0, f.transceiveBitsResult
	}
	return append([]byte(nil), f.transceiveBitsResponse...), f.transceiveBitsCount, nativeResult{code: nativeOK}
}

func TestReaderAcceptsAndCopiesSupportedUIDLengths(t *testing.T) {
	for _, uidLength := range []int{4, 7, 10} {
		t.Run(fmt.Sprintf("%d-byte", uidLength), func(t *testing.T) {
			uid := make([]byte, uidLength)
			for index := range uid {
				uid[index] = byte(index + 1)
			}
			native := &fakeNative{card: nfc.CardInfo{UID: uid}}
			reader := (&Backend{native: native}).NewReader()
			if err := reader.Open(context.Background(), "mock:test"); err != nil {
				t.Fatal(err)
			}
			defer reader.Close()
			card, err := reader.CardInfo(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			card.UID[0] = 0xff
			if native.card.UID[0] != 1 {
				t.Fatal("CardInfo returned UID storage owned by the native driver")
			}
		})
	}
}

func TestReaderDescribesOpenedDevice(t *testing.T) {
	native := &fakeNative{name: "PN532 over UART"}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "pn532_uart:/dev/test"); err != nil {
		t.Fatal(err)
	}
	info := reader.DeviceInfo()
	if info.Name != "PN532 over UART" || info.ConnString != "pn532_uart:/dev/test" {
		t.Fatalf("DeviceInfo() = %+v", info)
	}
}

func TestOperationBeforeOpenDoesNotLeaveInflightState(t *testing.T) {
	native := &fakeNative{}
	reader := (&Backend{native: native}).NewReader()
	if _, err := reader.CardInfo(context.Background()); !errors.Is(err, nfc.ErrNotOpen) {
		t.Fatalf("CardInfo() error = %v; want ErrNotOpen", err)
	}
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	if err := reader.Close(); err != nil {
		t.Fatal(err)
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.aborts != 0 {
		t.Fatalf("Close() issued %d aborts without an in-flight call", native.aborts)
	}
}

func TestReaderRejectsInvalidUIDLength(t *testing.T) {
	native := &fakeNative{card: nfc.CardInfo{UID: make([]byte, 5)}}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	if _, err := reader.CardInfo(context.Background()); !errors.Is(err, nfc.ErrInternal) {
		t.Fatalf("CardInfo() error = %v; want ErrInternal", err)
	}
}

func TestReaderRawModeAndTransceiveBindings(t *testing.T) {
	native := &fakeNative{
		card:                    nfc.CardInfo{UID: []byte{1, 2, 3, 4}, ATQA: [2]byte{4, 0}, SAK: 0x08},
		transceiveBytesResponse: []byte{0x0a},
		transceiveBitsResponse:  []byte{0x0a},
		transceiveBitsCount:     4,
	}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	if _, err := reader.CardInfo(context.Background()); err != nil {
		t.Fatal(err)
	}
	if err := reader.SetRawMode(context.Background(), true); err != nil {
		t.Fatal(err)
	}
	response, err := reader.TransceiveBytes(context.Background(), []byte{0x50, 0x00, 0x57, 0xcd})
	if err != nil || string(response) != "\x0a" {
		t.Fatalf("TransceiveBytes() = %x, %v", response, err)
	}
	bitsResponse, bits, err := reader.TransceiveBits(context.Background(), []byte{0x40}, 7)
	if err != nil || bits != 4 || string(bitsResponse) != "\x0a" {
		t.Fatalf("TransceiveBits() = %x, %d, %v", bitsResponse, bits, err)
	}
	if err := reader.SetRawMode(context.Background(), false); err != nil {
		t.Fatal(err)
	}

	native.mu.Lock()
	defer native.mu.Unlock()
	if len(native.rawModeCalls) != 2 || !native.rawModeCalls[0] || native.rawModeCalls[1] {
		t.Fatalf("raw mode calls = %v", native.rawModeCalls)
	}
	if len(native.transceiveFrames) != 1 || string(native.transceiveFrames[0]) != "\x50\x00\x57\xcd" {
		t.Fatalf("byte frames = %x", native.transceiveFrames)
	}
	if len(native.transceiveBitFrames) != 1 || native.transceiveBitFrames[0].bits != 7 || string(native.transceiveBitFrames[0].data) != "\x40" {
		t.Fatalf("bit frames = %+v", native.transceiveBitFrames)
	}
}

func TestReaderRejectsInvalidRawBitFrameBeforeNativeCall(t *testing.T) {
	native := &fakeNative{}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	if _, _, err := reader.TransceiveBits(context.Background(), []byte{0x40}, 9); !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("TransceiveBits() error = %v; want ErrInvalidArgument", err)
	}
	if _, err := reader.TransceiveBytes(context.Background(), nil); !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("TransceiveBytes() error = %v; want ErrInvalidArgument", err)
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if len(native.transceiveFrames) != 0 || len(native.transceiveBitFrames) != 0 {
		t.Fatal("invalid raw frames reached the native driver")
	}
}

func TestReaderRawModeRequiresOpen(t *testing.T) {
	reader := (&Backend{native: &fakeNative{}}).NewReader()
	if err := reader.SetRawMode(context.Background(), true); !errors.Is(err, nfc.ErrNotOpen) {
		t.Fatalf("SetRawMode() error = %v; want ErrNotOpen", err)
	}
}

func TestReaderCloseAbortsInflightCallAndIsIdempotent(t *testing.T) {
	native := &fakeNative{
		cardStarted: make(chan struct{}),
		abortCard:   make(chan struct{}),
	}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}

	cardErr := make(chan error, 1)
	go func() {
		_, err := reader.CardInfo(context.Background())
		cardErr <- err
	}()
	<-native.cardStarted
	if err := reader.Close(); err != nil {
		t.Fatal(err)
	}
	if err := <-cardErr; !errors.Is(err, nfc.ErrCanceled) {
		t.Fatalf("CardInfo() error = %v; want ErrCanceled", err)
	}
	if err := reader.Close(); err != nil {
		t.Fatal(err)
	}

	native.mu.Lock()
	defer native.mu.Unlock()
	if native.aborts != 1 || native.closes != 1 || native.live != 0 {
		t.Fatalf("native counts aborts=%d closes=%d live=%d; want 1, 1, 0", native.aborts, native.closes, native.live)
	}
}

func TestReaderContextCancellationAbortsNativeCall(t *testing.T) {
	native := &fakeNative{
		cardStarted: make(chan struct{}),
		abortCard:   make(chan struct{}),
	}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cardErr := make(chan error, 1)
	go func() {
		_, err := reader.CardInfo(ctx)
		cardErr <- err
	}()
	<-native.cardStarted
	cancel()
	err := <-cardErr
	if !errors.Is(err, nfc.ErrCanceled) || !errors.Is(err, context.Canceled) {
		t.Fatalf("CardInfo() error = %v; want NFC and context cancellation", err)
	}
}

func TestReaderCanOpenAndCloseOneHundredTimes(t *testing.T) {
	native := &fakeNative{}
	reader := (&Backend{native: native}).NewReader()
	for iteration := 0; iteration < 100; iteration++ {
		if err := reader.Open(context.Background(), "mock:test"); err != nil {
			t.Fatalf("Open() iteration %d: %v", iteration, err)
		}
		if err := reader.Close(); err != nil {
			t.Fatalf("Close() iteration %d: %v", iteration, err)
		}
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.opens != 100 || native.closes != 100 || native.live != 0 {
		t.Fatalf("native counts opens=%d closes=%d live=%d; want 100, 100, 0", native.opens, native.closes, native.live)
	}
}

func TestOpenFailureDoesNotMakeReaderBusy(t *testing.T) {
	native := &fakeNative{openResult: nativeResult{code: nativeDeviceDisconnected}}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); !errors.Is(err, nfc.ErrDeviceDisconnected) {
		t.Fatalf("first Open() error = %v; want ErrDeviceDisconnected", err)
	}
	native.mu.Lock()
	native.openResult = nativeResult{code: nativeOK}
	native.mu.Unlock()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatalf("second Open() error = %v", err)
	}
}

func openSelectedClassicReader(t *testing.T, native *fakeNative, sak byte) *Reader {
	t.Helper()
	native.card = nfc.CardInfo{UID: []byte{1, 2, 3, 4}, ATQA: [2]byte{0, 4}, SAK: sak}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reader.Close() })
	if _, err := reader.CardInfo(context.Background()); err != nil {
		t.Fatal(err)
	}
	return reader
}

func TestReaderPassesKeyTypeAndExactKeyToNativeAuthentication(t *testing.T) {
	for _, test := range []struct {
		keyType nfc.KeyType
		command byte
	}{{nfc.KeyTypeA, 0x60}, {nfc.KeyTypeB, 0x61}} {
		native := &fakeNative{}
		reader := openSelectedClassicReader(t, native, 0x08)
		key := nfc.Key{1, 2, 3, 4, 5, 6}
		if err := reader.Authenticate(context.Background(), 4, test.keyType, key); err != nil {
			t.Fatal(err)
		}
		native.mu.Lock()
		if native.authCalls != 1 || native.authBlock != 4 || native.authCommand != test.command || native.authKey != key {
			t.Errorf("native authentication = calls %d block %d command %#x key %x", native.authCalls, native.authBlock, native.authCommand, native.authKey)
		}
		native.mu.Unlock()
	}
}

func TestReaderRejectsBlockOutsideSelectedCard(t *testing.T) {
	native := &fakeNative{}
	reader := openSelectedClassicReader(t, native, 0x08)
	if err := reader.Authenticate(context.Background(), 64, nfc.KeyTypeA, nfc.Key{}); !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("Authenticate(block 64) error = %v; want ErrInvalidArgument", err)
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.authCalls != 0 {
		t.Fatalf("invalid block reached native authentication %d times", native.authCalls)
	}
}

func TestReaderRequiresSelectedSupportedClassicCard(t *testing.T) {
	native := &fakeNative{}
	reader := (&Backend{native: native}).NewReader()
	if err := reader.Open(context.Background(), "mock:test"); err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	if _, err := reader.ReadBlock(context.Background(), 1); !errors.Is(err, nfc.ErrNoCard) {
		t.Fatalf("ReadBlock() error = %v; want ErrNoCard", err)
	}
	native.card = nfc.CardInfo{UID: []byte{1, 2, 3, 4}, SAK: 0x20}
	if _, err := reader.CardInfo(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := reader.ReadBlock(context.Background(), 1); !errors.Is(err, nfc.ErrUnsupported) {
		t.Fatalf("ReadBlock() error = %v; want ErrUnsupported", err)
	}
}

func TestReaderWriteProtectsManufacturerAndTrailerBlocks(t *testing.T) {
	native := &fakeNative{}
	reader := openSelectedClassicReader(t, native, 0x08)
	for _, block := range []byte{0, 3} {
		if err := reader.WriteBlock(context.Background(), block, [nfc.BlockSize]byte{}); !errors.Is(err, nfc.ErrInvalidArgument) {
			t.Errorf("WriteBlock(%d) error = %v; want ErrInvalidArgument", block, err)
		}
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.writeCalls != 0 {
		t.Fatalf("protected writes reached native %d times", native.writeCalls)
	}
}

func TestProtectedTrailerWriterAcceptsOnlyTrailer(t *testing.T) {
	native := &fakeNative{enforceAuth: true}
	reader := openSelectedClassicReader(t, native, 0x08)
	if err := reader.Authenticate(context.Background(), 3, nfc.KeyTypeA, nfc.Key{}); err != nil {
		t.Fatal(err)
	}
	data := [nfc.BlockSize]byte{6: 0xff, 7: 0x07, 8: 0x80}
	if err := reader.WriteSectorTrailer(context.Background(), 3, data); err != nil {
		t.Fatal(err)
	}
	if err := reader.WriteSectorTrailer(context.Background(), 2, data); !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("WriteSectorTrailer(data block) error = %v", err)
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.writeCalls != 1 || native.writtenBlock != 3 || native.written != data || len(native.readBlocks) != 0 {
		t.Fatalf("protected native writes=%d block=%d reads=%v data=%x", native.writeCalls, native.writtenBlock, native.readBlocks, native.written)
	}
}

func TestProtectedManufacturerWriterRequiresAuthenticationAndHasNoReadback(t *testing.T) {
	native := &fakeNative{enforceAuth: true}
	reader := openSelectedClassicReader(t, native, 0x08)
	data := [nfc.BlockSize]byte{1, 2, 3, 4, 4, 0xaa}
	if err := reader.WriteManufacturerBlock(context.Background(), data); !errors.Is(err, nfc.ErrNotAuthenticated) {
		t.Fatalf("unauthenticated WriteManufacturerBlock() error = %v", err)
	}
	if err := reader.Authenticate(context.Background(), 0, nfc.KeyTypeA, nfc.Key{}); err != nil {
		t.Fatal(err)
	}
	if err := reader.WriteManufacturerBlock(context.Background(), data); err != nil {
		t.Fatal(err)
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.writeCalls != 2 || native.writtenBlock != 0 || native.manufacturerWritten != data || len(native.readBlocks) != 0 {
		t.Fatalf("manufacturer writes=%d block=%d reads=%v data=%x", native.writeCalls, native.writtenBlock, native.readBlocks, native.manufacturerWritten)
	}
}

func TestReaderWriteImmediatelyReadsBackAndVerifies(t *testing.T) {
	want := [nfc.BlockSize]byte{1, 2, 3, 4}
	native := &fakeNative{readData: want, enforceAuth: true}
	reader := openSelectedClassicReader(t, native, 0x08)
	if err := reader.Authenticate(context.Background(), 1, nfc.KeyTypeA, nfc.Key{}); err != nil {
		t.Fatal(err)
	}
	if err := reader.WriteBlock(context.Background(), 1, want); err != nil {
		t.Fatal(err)
	}
	native.mu.Lock()
	defer native.mu.Unlock()
	if native.writeCalls != 1 || native.writtenBlock != 1 || native.written != want {
		t.Fatalf("native write = calls %d block %d data %x", native.writeCalls, native.writtenBlock, native.written)
	}
	if len(native.readBlocks) != 1 || native.readBlocks[0] != 1 {
		t.Fatalf("verification reads = %v; want [1]", native.readBlocks)
	}
}

func TestReaderWriteMismatchHasIndependentError(t *testing.T) {
	native := &fakeNative{readData: [nfc.BlockSize]byte{0xff}, enforceAuth: true}
	reader := openSelectedClassicReader(t, native, 0x08)
	if err := reader.Authenticate(context.Background(), 1, nfc.KeyTypeA, nfc.Key{}); err != nil {
		t.Fatal(err)
	}
	err := reader.WriteBlock(context.Background(), 1, [nfc.BlockSize]byte{0x01})
	if !errors.Is(err, nfc.ErrVerificationFailed) {
		t.Fatalf("WriteBlock() error = %v; want ErrVerificationFailed", err)
	}
}

func TestReaderReselectionExpiresAuthentication(t *testing.T) {
	native := &fakeNative{enforceAuth: true}
	reader := openSelectedClassicReader(t, native, 0x08)
	if err := reader.Authenticate(context.Background(), 1, nfc.KeyTypeA, nfc.Key{}); err != nil {
		t.Fatal(err)
	}
	if _, err := reader.CardInfo(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := reader.ReadBlock(context.Background(), 1); !errors.Is(err, nfc.ErrNotAuthenticated) {
		t.Fatalf("ReadBlock() after reselection error = %v; want ErrNotAuthenticated", err)
	}
}

func TestReaderReadWithoutAuthenticationHasIndependentError(t *testing.T) {
	native := &fakeNative{enforceAuth: true}
	reader := openSelectedClassicReader(t, native, 0x08)
	if _, err := reader.ReadBlock(context.Background(), 1); !errors.Is(err, nfc.ErrNotAuthenticated) {
		t.Fatalf("ReadBlock() error = %v; want ErrNotAuthenticated", err)
	}
}

func TestReaderInvalidatesSelectionWhenCardChangesMidOperation(t *testing.T) {
	native := &fakeNative{readResult: nativeResult{code: nativeCardChanged}}
	reader := openSelectedClassicReader(t, native, 0x08)
	if _, err := reader.ReadBlock(context.Background(), 1); !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("ReadBlock() error = %v; want ErrCardChanged", err)
	}
	if err := reader.Authenticate(context.Background(), 1, nfc.KeyTypeA, nfc.Key{}); !errors.Is(err, nfc.ErrNoCard) {
		t.Fatalf("Authenticate() after replacement error = %v; want ErrNoCard until reselect", err)
	}
}
