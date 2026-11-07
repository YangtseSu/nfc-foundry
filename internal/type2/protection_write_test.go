package type2

import (
	"errors"
	"testing"
)

func TestPlanProtectionWritesOrderAndPreservation(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	document.Pages[2] = "11220000"
	document.Pages[40] = "00000077"
	document.Pages[44] = "00005566"
	auth0 := 4
	access := byte(0x8F)
	password := [4]byte{1, 2, 3, 4}
	pack := [2]byte{0xAA, 0xBB}
	dynamic := [3]byte{1, 0, 0}
	static := uint16(0x0002)
	writes, changes, err := PlanProtectionWrites(document, ProtectionWriteOptions{
		Auth0: &auth0, Access: &access, Password: &password,
		Pack: &pack, DynamicLock: &dynamic, StaticLock: &static,
	})
	if err != nil {
		t.Fatal(err)
	}
	wantPages := []int{43, 44, 41, 42, 40, 2}
	if len(writes) != len(wantPages) {
		t.Fatalf("writes = %+v", writes)
	}
	for index, page := range wantPages {
		if writes[index].Page != page {
			t.Fatalf("write %d page = %d; want %d", index, writes[index].Page, page)
		}
	}
	if len(changes) != 6 {
		t.Fatalf("changes = %+v", changes)
	}
	data := func(page int) [4]byte {
		t.Helper()
		for _, write := range writes {
			if write.Page == page {
				value, err := write.Data()
				if err != nil {
					t.Fatal(err)
				}
				return value
			}
		}
		t.Fatalf("page %d is not planned", page)
		return [4]byte{}
	}
	if got := data(41); got[3] != 4 {
		t.Fatalf("AUTH0 page = % X", got)
	}
	if got := data(42); got[0] != 0x87 {
		t.Fatalf("ACCESS = 0x%02X; want 0x87 with bit 3 preserved", got[0])
	}
	if got := data(44); got[0] != 0xAA || got[1] != 0xBB || got[2] != 0x55 || got[3] != 0x66 {
		t.Fatalf("PACK page = % X", got)
	}
	if got := data(40); got[0] != 1 || got[3] != 0x77 {
		t.Fatalf("dynamic lock = % X", got)
	}
	if got := data(2); got[0] != 0x11 || got[1] != 0x22 || got[2] != 0x02 || got[3] != 0 {
		t.Fatalf("static lock = % X", got)
	}
}

func TestPlanProtectionWritesRejectsClearingOneWayBits(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	document.Pages[2] = "00000200"
	static := uint16(0)
	if _, _, err := PlanProtectionWrites(document, ProtectionWriteOptions{StaticLock: &static}); !errors.Is(err, ErrProtectedPage) {
		t.Fatalf("static clear error = %v", err)
	}
	document.Pages[40] = "01000000"
	dynamic := [3]byte{}
	if _, _, err := PlanProtectionWrites(document, ProtectionWriteOptions{DynamicLock: &dynamic}); !errors.Is(err, ErrProtectedPage) {
		t.Fatalf("dynamic clear error = %v", err)
	}
}

func TestPlanProtectionWritesRejectsCFGLCKChanges(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	document.Pages[42] = "40000000"
	updated, err := document.PageBytes()
	if err != nil {
		t.Fatal(err)
	}
	document.Refresh(updated)
	auth0 := 4
	if _, _, err := PlanProtectionWrites(document, ProtectionWriteOptions{Auth0: &auth0}); !errors.Is(err, ErrProtectedPage) {
		t.Fatalf("AUTH0 on locked configuration error = %v", err)
	}
	access := byte(0)
	if _, _, err := PlanProtectionWrites(document, ProtectionWriteOptions{Access: &access}); !errors.Is(err, ErrProtectedPage) {
		t.Fatalf("CFGLCK clear error = %v", err)
	}
	static := uint16(0x0002)
	if _, _, err := PlanProtectionWrites(document, ProtectionWriteOptions{StaticLock: &static}); err != nil {
		t.Fatalf("static lock with CFGLCK set: %v", err)
	}
}

func TestPlanProtectionWritesValidation(t *testing.T) {
	document := testDocument(t, ProductNTAG213)
	if _, _, err := PlanProtectionWrites(document, ProtectionWriteOptions{}); !errors.Is(err, ErrProtectedPage) {
		t.Fatalf("empty selection error = %v", err)
	}
	invalid := 256
	if _, _, err := PlanProtectionWrites(document, ProtectionWriteOptions{Auth0: &invalid}); !errors.Is(err, ErrProtectedPage) {
		t.Fatalf("AUTH0 range error = %v", err)
	}
}
