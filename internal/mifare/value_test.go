package mifare

import (
	"encoding/hex"
	"strings"
	"testing"
)

func TestValueBlockRoundTrip(t *testing.T) {
	cases := []struct {
		value   int32
		address byte
		hex     string
	}{
		{0, 0, "00000000FFFFFFFF0000000000FF00FF"},
		{1, 4, "01000000FEFFFFFF0100000004FB04FB"},
		{100, 5, "640000009BFFFFFF6400000005FA05FA"},
		{-1, 3, "FFFFFFFF00000000FFFFFFFF03FC03FC"},
	}
	for _, test := range cases {
		encoded := EncodeValueBlock(test.value, test.address)
		if got := strings.ToUpper(hex.EncodeToString(encoded[:])); got != test.hex {
			t.Fatalf("EncodeValueBlock(%d, %d) = %s, want %s", test.value, test.address, got, test.hex)
		}
		decoded, err := DecodeValueBlock(encoded)
		if err != nil {
			t.Fatalf("DecodeValueBlock(%d, %d): %v", test.value, test.address, err)
		}
		if decoded.Value != test.value || decoded.Address != test.address {
			t.Fatalf("round trip = %+v, want value %d address %d", decoded, test.value, test.address)
		}
	}
}

func TestDecodeValueBlockRejectsInconsistentCopies(t *testing.T) {
	valid := EncodeValueBlock(100, 5)
	cases := []struct {
		name   string
		mutate func(*[BlockSize]byte)
		want   string
	}{
		{"inverted value", func(data *[BlockSize]byte) { data[4] ^= 0x01 }, "inverted value"},
		{"repeated value", func(data *[BlockSize]byte) { data[8] ^= 0x01 }, "repeated value"},
		{"address copies", func(data *[BlockSize]byte) { data[14] ^= 0x01 }, "address copies"},
		{"inverted address", func(data *[BlockSize]byte) { data[13] ^= 0x01 }, "inverted address"},
		{"second inverted address", func(data *[BlockSize]byte) { data[15] ^= 0x01 }, "second inverted address"},
	}
	for _, test := range cases {
		data := valid
		test.mutate(&data)
		_, err := DecodeValueBlock(data)
		if err == nil || !strings.Contains(err.Error(), test.want) {
			t.Fatalf("%s: DecodeValueBlock error = %v, want %q", test.name, err, test.want)
		}
	}
}

func TestDecodeValueBlockRejectsNonValueData(t *testing.T) {
	var data [BlockSize]byte
	copy(data[:], "NFC Foundry 1K\x00\x00")
	if _, err := DecodeValueBlock(data); err == nil {
		t.Fatal("ordinary data decoded as a value block")
	}
}
