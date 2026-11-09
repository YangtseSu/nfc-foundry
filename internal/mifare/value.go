package mifare

import (
	"encoding/binary"
	"fmt"
)

// ValueBlock is the decoded form of a MIFARE Classic value block. The card
// stores a signed 32-bit little-endian value three times and the block address
// twice, each with a byte-wise inverted copy.
type ValueBlock struct {
	Value   int32
	Address byte
}

// DecodeValueBlock validates every redundant copy. It never repairs a
// malformed value block.
func DecodeValueBlock(data [BlockSize]byte) (ValueBlock, error) {
	value := int32(binary.LittleEndian.Uint32(data[0:4]))
	if !validValueCopy(data[0:4], data[4:8]) {
		return ValueBlock{}, fmt.Errorf("value block inverted value copy does not match bytes 0-3")
	}
	if string(data[0:4]) != string(data[8:12]) {
		return ValueBlock{}, fmt.Errorf("value block repeated value copy does not match bytes 0-3")
	}
	address := data[12]
	if data[14] != address {
		return ValueBlock{}, fmt.Errorf("value block address copies disagree: %02X and %02X", address, data[14])
	}
	if data[13] != ^address {
		return ValueBlock{}, fmt.Errorf("value block inverted address %02X does not match address %02X", data[13], address)
	}
	if data[15] != ^address {
		return ValueBlock{}, fmt.Errorf("value block second inverted address %02X does not match address %02X", data[15], address)
	}
	return ValueBlock{Value: value, Address: address}, nil
}

// EncodeValueBlock produces the canonical 16-byte value block.
func EncodeValueBlock(value int32, address byte) [BlockSize]byte {
	var data [BlockSize]byte
	binary.LittleEndian.PutUint32(data[0:4], uint32(value))
	for index := range 4 {
		data[4+index] = ^data[index]
		data[8+index] = data[index]
	}
	data[12] = address
	data[13] = ^address
	data[14] = address
	data[15] = ^address
	return data
}

func validValueCopy(value, inverted []byte) bool {
	if len(value) != 4 || len(inverted) != 4 {
		return false
	}
	for index := range value {
		if inverted[index] != ^value[index] {
			return false
		}
	}
	return true
}
