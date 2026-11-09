package mifare

import "testing"

func TestDataAccessPermissionsFollowTheMIFARETable(t *testing.T) {
	cases := []struct {
		code      uint8
		read      KeyMask
		write     KeyMask
		increment KeyMask
		decrement KeyMask
	}{
		{0, KeyMaskAny, KeyMaskAny, KeyMaskAny, KeyMaskAny},
		{1, KeyMaskAny, KeyMaskNone, KeyMaskNone, KeyMaskAny},
		{2, KeyMaskAny, KeyMaskNone, KeyMaskNone, KeyMaskNone},
		{3, KeyMaskB, KeyMaskB, KeyMaskNone, KeyMaskNone},
		{4, KeyMaskAny, KeyMaskB, KeyMaskNone, KeyMaskNone},
		{5, KeyMaskB, KeyMaskNone, KeyMaskNone, KeyMaskNone},
		{6, KeyMaskAny, KeyMaskB, KeyMaskB, KeyMaskB},
		{7, KeyMaskNone, KeyMaskNone, KeyMaskNone, KeyMaskNone},
	}
	for _, test := range cases {
		condition := conditionForCode(test.code)
		if got := condition.DataReadKeys(); got != test.read {
			t.Fatalf("code %03b read keys = %d, want %d", test.code, got, test.read)
		}
		if got := condition.DataWriteKeys(); got != test.write {
			t.Fatalf("code %03b write keys = %d, want %d", test.code, got, test.write)
		}
		if got := condition.DataIncrementKeys(); got != test.increment {
			t.Fatalf("code %03b increment keys = %d, want %d", test.code, got, test.increment)
		}
		if got := condition.DataDecrementKeys(); got != test.decrement {
			t.Fatalf("code %03b decrement keys = %d, want %d", test.code, got, test.decrement)
		}
	}
}

func TestTrailerAccessFollowsTheMIFARETable(t *testing.T) {
	cases := []struct {
		code      uint8
		writeA    KeyMask
		writeBits KeyMask
		writeB    KeyMask
		keyBRead  bool
	}{
		{0, KeyMaskA, KeyMaskNone, KeyMaskA, true},
		{1, KeyMaskA, KeyMaskA, KeyMaskA, true},
		{2, KeyMaskNone, KeyMaskNone, KeyMaskNone, true},
		{3, KeyMaskB, KeyMaskB, KeyMaskB, false},
		{4, KeyMaskB, KeyMaskNone, KeyMaskB, false},
		{5, KeyMaskNone, KeyMaskB, KeyMaskNone, false},
		{6, KeyMaskNone, KeyMaskNone, KeyMaskNone, false},
		{7, KeyMaskNone, KeyMaskNone, KeyMaskNone, false},
	}
	for _, test := range cases {
		permissions := conditionForCode(test.code).TrailerAccess()
		if permissions.WriteKeyA != test.writeA || permissions.WriteAccess != test.writeBits || permissions.WriteKeyB != test.writeB {
			t.Fatalf("code %03b trailer writes = %d/%d/%d, want %d/%d/%d",
				test.code, permissions.WriteKeyA, permissions.WriteAccess, permissions.WriteKeyB,
				test.writeA, test.writeBits, test.writeB)
		}
		if permissions.KeyBReadable != test.keyBRead {
			t.Fatalf("code %03b key B readable = %v, want %v", test.code, permissions.KeyBReadable, test.keyBRead)
		}
	}
}

func conditionForCode(code uint8) AccessCondition {
	return AccessCondition{C1: code&4 != 0, C2: code&2 != 0, C3: code&1 != 0}
}
