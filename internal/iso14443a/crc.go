// Package iso14443a contains framing primitives shared by ISO/IEC 14443A card
// protocols. It does not implement a card command set.
package iso14443a

// CRCA returns the ISO/IEC 14443-3 CRC-A for data, least-significant byte
// first, as it is transmitted on the air.
func CRCA(data []byte) [2]byte {
	crc := uint16(0x6363)
	for _, value := range data {
		crc ^= uint16(value)
		for range 8 {
			if crc&1 != 0 {
				crc = (crc >> 1) ^ 0x8408
			} else {
				crc >>= 1
			}
		}
	}
	return [2]byte{byte(crc), byte(crc >> 8)}
}

// WithCRCA returns a new frame containing data followed by its CRC-A.
func WithCRCA(data []byte) []byte {
	frame := make([]byte, len(data)+2)
	copy(frame, data)
	crc := CRCA(data)
	frame[len(data)] = crc[0]
	frame[len(data)+1] = crc[1]
	return frame
}
