package workflow

import (
	"context"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

func lifecycleService(t *testing.T, reader nfc.Reader) *Type2Service {
	t.Helper()
	return NewType2Service(type2Executor{reader: reader}, FileType2BackupStore{Root: t.TempDir()})
}

func TestType2CopyBetweenDifferentUIDs(t *testing.T) {
	sourceTag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	sourceTag.SetPage(4, [4]byte{0xAA, 0xBB, 0xCC, 0xDD})
	sourceDocument, err := type2.Read(context.Background(), sourceTag, sourceTag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	targetTag := mock.NewType2Tag(type2.ProductNTAG213, []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66})
	targetReader := mock.NewType2Reader(targetTag)
	if err := targetReader.Open(context.Background(), "mock:target"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, targetReader)
	request := Type2LifecycleRequest{
		Card: targetTag.CardInfo(), Operation: Type2OperationCopy, Source: sourceDocument,
	}
	plan, err := service.PreviewType2Lifecycle(context.Background(), request)
	if err != nil {
		t.Fatal(err)
	}
	if plan.SourceUID != sourceDocument.UID || plan.Token == "" || len(plan.Writes) == 0 {
		t.Fatalf("plan = %+v", plan)
	}
	request.ExpectedToken = plan.Token
	result, err := service.ExecuteType2Lifecycle(context.Background(), request)
	if err != nil {
		t.Fatal(err)
	}
	if result.BackupPath == "" || len(result.Verified) != len(result.Written) {
		t.Fatalf("result = %+v", result)
	}
	if page, _ := targetTag.Page(4); page != [4]byte{0xAA, 0xBB, 0xCC, 0xDD} {
		t.Fatalf("page 4 = % X", page)
	}
	if card := targetTag.CardInfo(); string(card.UID) != string([]byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66}) {
		t.Fatalf("target UID changed: % X", card.UID)
	}
	page0, _ := targetTag.Page(0)
	if page0[3] != targetTag.CardInfo().UID[0]^targetTag.CardInfo().UID[1]^targetTag.CardInfo().UID[2]^0x88 {
		t.Fatalf("target BCC0 changed: % X", page0)
	}
}

func TestType2CopySameUIDPolicyAndProduct(t *testing.T) {
	sourceTag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	sourceDocument, err := type2.Read(context.Background(), sourceTag, sourceTag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	targetTag := mock.NewType2Tag(type2.ProductNTAG213, []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66})
	targetReader := mock.NewType2Reader(targetTag)
	if err := targetReader.Open(context.Background(), "mock:target"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, targetReader)
	_, err = service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: targetTag.CardInfo(), Operation: Type2OperationCopy, Source: sourceDocument, SameUIDOnly: true,
	})
	if !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("same-UID policy error = %v", err)
	}
	other := mock.NewType2Tag(type2.ProductNTAG215, nil)
	otherDocument, err := type2.Read(context.Background(), other, other.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	_, err = service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: targetTag.CardInfo(), Operation: Type2OperationCopy, Source: otherDocument,
	})
	if !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("product mismatch error = %v", err)
	}
}

func TestType2EraseAndStaticLock(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	tag.SetPage(4, [4]byte{1, 2, 3, 4})
	tag.SetPage(5, [4]byte{5, 6, 7, 8})
	reader := mock.NewType2Reader(tag)
	if err := reader.Open(context.Background(), "mock:erase"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, reader)
	plan, err := service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationErase,
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.ExecuteType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationErase, ExpectedToken: plan.Token,
	}); err != nil {
		t.Fatal(err)
	}
	if page, _ := tag.Page(4); page != [4]byte{} {
		t.Fatalf("page 4 = % X", page)
	}
	if page, _ := tag.Page(41); page != [4]byte{0x04, 0x00, 0x00, 0xFF} {
		t.Fatalf("configuration changed: % X", page)
	}

	lock := uint16(0x0001 << 1) // page 4
	plan, err = service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationLock, Lock: type2.LockOptions{StaticLock: &lock},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.ExecuteType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationLock, Lock: type2.LockOptions{StaticLock: &lock},
		ExpectedToken: plan.Token,
	}); err != nil {
		t.Fatal(err)
	}
	page2, _ := tag.Page(2)
	if page2[2] != 0x02 || page2[0] != tag.CardInfo().UID[3]^tag.CardInfo().UID[4]^tag.CardInfo().UID[5]^tag.CardInfo().UID[6] {
		t.Fatalf("static lock page = % X", page2)
	}
}

func TestType2PasswordSetAndRemove(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	reader := mock.NewType2Reader(tag)
	if err := reader.Open(context.Background(), "mock:password"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, reader)
	set := Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationPasswordSet,
		PasswordOptions: type2.PasswordOptions{
			Password: [4]byte{1, 2, 3, 4}, Pack: [2]byte{0xAA, 0xBB}, Auth0: 4, Protect: true,
		},
	}
	plan, err := service.PreviewType2Lifecycle(context.Background(), set)
	if err != nil {
		t.Fatal(err)
	}
	set.ExpectedToken = plan.Token
	if _, err := service.ExecuteType2Lifecycle(context.Background(), set); err != nil {
		t.Fatal(err)
	}
	if page, _ := tag.Page(43); page != [4]byte{1, 2, 3, 4} {
		t.Fatalf("password page = % X", page)
	}
	if page, _ := tag.Page(44); page[0] != 0xAA || page[1] != 0xBB {
		t.Fatalf("PACK page = % X", page)
	}
	if page, _ := tag.Page(42); page[0]&0x80 == 0 {
		t.Fatalf("PROT is not set: % X", page)
	}

	remove := Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationPasswordRemove, Password: []byte{1, 2, 3, 4},
	}
	plan, err = service.PreviewType2Lifecycle(context.Background(), remove)
	if err != nil {
		t.Fatal(err)
	}
	remove.ExpectedToken = plan.Token
	if _, err := service.ExecuteType2Lifecycle(context.Background(), remove); err != nil {
		t.Fatal(err)
	}
	if page, _ := tag.Page(43); page != [4]byte{0xFF, 0xFF, 0xFF, 0xFF} {
		t.Fatalf("password page after remove = % X", page)
	}
	if page, _ := tag.Page(42); page[0]&0x80 != 0 {
		t.Fatalf("PROT survived removal: % X", page)
	}
}

func TestType2LifecycleRejectsStaleToken(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	tag.SetPage(4, [4]byte{1, 2, 3, 4})
	reader := mock.NewType2Reader(tag)
	if err := reader.Open(context.Background(), "mock:token"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, reader)
	_, err := service.ExecuteType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationErase, ExpectedToken: "stale",
	})
	if !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("stale token error = %v", err)
	}
	if page, _ := tag.Page(4); page != [4]byte{1, 2, 3, 4} {
		t.Fatalf("tag changed with a stale token: % X", page)
	}
}

func TestType2AdvancedCopyIncludesProtectionPages(t *testing.T) {
	sourceTag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	sourceTag.SetPage(41, [4]byte{0x04, 0x00, 0x00, 0x04})
	sourceTag.SetPage(42, [4]byte{0x80, 0x00, 0x00, 0x00})
	sourceTag.SetPage(43, [4]byte{1, 2, 3, 4})
	sourceTag.SetPage(44, [4]byte{0xAA, 0xBB, 0x00, 0x00})
	sourceDocument, err := type2.Read(context.Background(), sourceTag, sourceTag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	targetTag := mock.NewType2Tag(type2.ProductNTAG213, []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66})
	targetReader := mock.NewType2Reader(targetTag)
	if err := targetReader.Open(context.Background(), "mock:advanced"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, targetReader)
	request := Type2LifecycleRequest{
		Card: targetTag.CardInfo(), Operation: Type2OperationCopy, Source: sourceDocument, IncludeProtected: true,
	}
	plan, err := service.PreviewType2Lifecycle(context.Background(), request)
	if err != nil {
		t.Fatal(err)
	}
	request.ExpectedToken = plan.Token
	if _, err := service.ExecuteType2Lifecycle(context.Background(), request); err != nil {
		t.Fatal(err)
	}
	for _, page := range []int{41, 42, 43, 44} {
		got, _ := targetTag.Page(page)
		want, _ := sourceTag.Page(page)
		if got != want {
			t.Fatalf("page %d = % X; want % X", page, got, want)
		}
	}
	if page, _ := targetTag.Page(0); page[0] != 0x04 {
		t.Fatalf("target UID page changed: % X", page)
	}
}

func TestType2AdvancedCopyFormatsBlankCapabilityContainer(t *testing.T) {
	sourceTag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	sourceDocument, err := type2.Read(context.Background(), sourceTag, sourceTag.CardInfo(), type2.ReadOptions{})
	if err != nil {
		t.Fatal(err)
	}
	targetTag := mock.NewType2Tag(type2.ProductNTAG213, []byte{0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66})
	targetTag.SetPage(3, [4]byte{})
	targetReader := mock.NewType2Reader(targetTag)
	if err := targetReader.Open(context.Background(), "mock:blank"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, targetReader)
	request := Type2LifecycleRequest{
		Card: targetTag.CardInfo(), Operation: Type2OperationCopy, Source: sourceDocument, IncludeProtected: true,
	}
	plan, err := service.PreviewType2Lifecycle(context.Background(), request)
	if err != nil {
		t.Fatal(err)
	}
	request.ExpectedToken = plan.Token
	if _, err := service.ExecuteType2Lifecycle(context.Background(), request); err != nil {
		t.Fatal(err)
	}
	page, _ := targetTag.Page(3)
	if page != [4]byte{0xE1, 0x10, 0x12, 0x00} {
		t.Fatalf("capability container = % X", page)
	}
}

func TestType2LockTargetsAndEraseSkipsLockedPages(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	page2, _ := tag.Page(2)
	page2[2] = 0x02
	tag.SetPage(2, page2)
	tag.SetPage(4, [4]byte{1, 2, 3, 4})
	reader := mock.NewType2Reader(tag)
	if err := reader.Open(context.Background(), "mock:lock"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, reader)
	plan, err := service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationErase,
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.ExecuteType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationErase, ExpectedToken: plan.Token,
	}); err != nil {
		t.Fatal(err)
	}
	if page, _ := tag.Page(4); page != [4]byte{1, 2, 3, 4} {
		t.Fatalf("locked page was erased: % X", page)
	}

	dynamic := [3]byte{0x01, 0x00, 0x00}
	plan, err = service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationLock,
		Lock: type2.LockOptions{DynamicLock: &dynamic, CCReadOnly: true, ConfigLock: true},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.ExecuteType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationLock,
		Lock: type2.LockOptions{DynamicLock: &dynamic, CCReadOnly: true, ConfigLock: true}, ExpectedToken: plan.Token,
	}); err != nil {
		t.Fatal(err)
	}
	if page, _ := tag.Page(40); page[0] != 0x01 {
		t.Fatalf("dynamic lock page = % X", page)
	}
	if page, _ := tag.Page(42); page[0]&0x40 == 0 {
		t.Fatalf("CFGLCK is not set: % X", page)
	}
	if page, _ := tag.Page(3); page[3] != 0x0F {
		t.Fatalf("CC access = % X", page)
	}
	_, err = service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationLock, Lock: type2.LockOptions{ConfigLock: true},
	})
	if !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("second CFGLCK error = %v", err)
	}
}

func TestType2PasswordRemoveRefusesConfigLock(t *testing.T) {
	tag := mock.NewType2Tag(type2.ProductNTAG213, nil)
	tag.SetPage(42, [4]byte{0x40, 0x00, 0x00, 0x00})
	reader := mock.NewType2Reader(tag)
	if err := reader.Open(context.Background(), "mock:cfglck"); err != nil {
		t.Fatal(err)
	}
	service := lifecycleService(t, reader)
	_, err := service.PreviewType2Lifecycle(context.Background(), Type2LifecycleRequest{
		Card: tag.CardInfo(), Operation: Type2OperationPasswordRemove,
	})
	if !errors.Is(err, nfc.ErrInvalidArgument) {
		t.Fatalf("CFGLCK password removal error = %v", err)
	}
}
