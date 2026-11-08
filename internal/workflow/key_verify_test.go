package workflow_test

import (
	"context"
	"errors"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
	"github.com/YangtseSu/nfc-foundry/internal/nfc/mock"
	"github.com/YangtseSu/nfc-foundry/internal/workflow"
)

func TestKeyVerifyAuthenticatesEachSlotWithoutMutating(t *testing.T) {
	card := classic1KCard(7)
	accepted := nfc.Key{0x11, 0x22, 0x33, 0x44, 0x55, 0x66}
	rejected := nfc.Key{0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF}
	reader := openMockReader(t, mock.ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) { return card, nil },
		Authenticate: func(_ context.Context, _ byte, _ nfc.KeyType, key nfc.Key) error {
			if key == accepted {
				return nil
			}
			return nfc.ErrAuthenticationFailed
		},
	})
	result, err := workflow.NewKeyVerifyService(directExecutor{reader: reader}).Verify(context.Background(), card, []workflow.KeyCheck{
		{Sector: 0, KeyType: nfc.KeyTypeA, KeyID: "key-a", Key: accepted},
		{Sector: 1, KeyType: nfc.KeyTypeB, KeyID: "key-b", Key: rejected},
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Verified != 1 || !result.Checks[0].Verified || result.Checks[1].Verified {
		t.Fatalf("result = %+v", result)
	}
	if result.Checks[1].Error == "" {
		t.Fatal("rejected check has no error message")
	}
}

func TestKeyVerifyRejectsChangedCard(t *testing.T) {
	expected := classic1KCard(7)
	actual := classic1KCard(8)
	reader := openMockReader(t, mock.ReaderFuncs{
		CardInfo: func(context.Context) (nfc.CardInfo, error) { return actual, nil },
		Authenticate: func(context.Context, byte, nfc.KeyType, nfc.Key) error {
			return nil
		},
	})
	_, err := workflow.NewKeyVerifyService(directExecutor{reader: reader}).Verify(context.Background(), expected, []workflow.KeyCheck{
		{Sector: 0, KeyType: nfc.KeyTypeA, Key: nfc.Key{1, 2, 3, 4, 5, 6}},
	})
	if !errors.Is(err, nfc.ErrCardChanged) {
		t.Fatalf("error = %v", err)
	}
}
