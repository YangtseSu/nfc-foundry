package iso14443a

import (
	"bytes"
	"testing"
)

func TestCRCAVectors(t *testing.T) {
	for _, test := range []struct {
		input []byte
		want  []byte
	}{
		{[]byte{0x50, 0x00}, []byte{0x57, 0xCD}},
		{[]byte{0xA0, 0x00}, []byte{0x5F, 0xB1}},
		{[]byte{0x30, 0x04}, []byte{0x26, 0xEE}},
	} {
		frame := WithCRCA(test.input)
		if !bytes.Equal(frame[len(test.input):], test.want) {
			t.Fatalf("CRC-A(% X) = % X, want % X", test.input, frame[len(test.input):], test.want)
		}
	}
}
