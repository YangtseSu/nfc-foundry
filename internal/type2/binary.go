package type2

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"strings"
)

// BinaryDumpMagic starts a binary Type 2 full-image dump.
var BinaryDumpMagic = [8]byte{'N', 'F', 'C', 'T', '2', 'B', 'I', 'N'}

// BinaryDumpVersion is the current binary dump schema.
const BinaryDumpVersion byte = 1

var binaryProductCodes = map[Product]byte{
	ProductUltralight: 0,
	ProductNTAG213:    1,
	ProductNTAG215:    2,
	ProductNTAG216:    3,
}

var binaryProducts = map[byte]Product{
	0: ProductUltralight,
	1: ProductNTAG213,
	2: ProductNTAG215,
	3: ProductNTAG216,
}

// MarshalBinary encodes a complete dump with a self-describing header. The
// header carries the product, UID, ATQA, and SAK so a restore can validate the
// target before writing.
func (d Document) MarshalBinary() ([]byte, error) {
	if err := d.Validate(); err != nil {
		return nil, err
	}
	uid, err := hex.DecodeString(d.UID)
	if err != nil || len(uid) != 4 && len(uid) != 7 {
		return nil, validationError("encode binary Type 2 dump", err, "UID must contain 8 or 14 hexadecimal digits")
	}
	atqa, err := hex.DecodeString(d.ATQA)
	if err != nil || len(atqa) != 2 {
		return nil, validationError("encode binary Type 2 dump", err, "ATQA must contain 4 hexadecimal digits")
	}
	sak, err := hex.DecodeString(d.SAK)
	if err != nil || len(sak) != 1 {
		return nil, validationError("encode binary Type 2 dump", err, "SAK must contain 2 hexadecimal digits")
	}
	code, ok := binaryProductCodes[d.Product]
	if !ok {
		return nil, validationError("encode binary Type 2 dump", nil, fmt.Sprintf("unsupported product %q", d.Product))
	}
	output := make([]byte, 0, 16+len(uid)+len(d.Pages)*PageSize)
	output = append(output, BinaryDumpMagic[:]...)
	output = append(output, BinaryDumpVersion, code)
	output = binary.BigEndian.AppendUint16(output, uint16(d.TotalPages))
	output = append(output, byte(len(uid)))
	output = append(output, uid...)
	output = append(output, atqa...)
	output = append(output, sak...)
	for index, page := range d.Pages {
		decoded, err := hex.DecodeString(page)
		if err != nil || len(decoded) != PageSize {
			return nil, validationError("encode binary Type 2 dump", err, fmt.Sprintf("page %d is not four hexadecimal bytes", index))
		}
		output = append(output, decoded...)
	}
	return output, nil
}

// UnmarshalBinary decodes a complete binary dump and revalidates its identity,
// geometry, and capability container.
func (d *Document) UnmarshalBinary(data []byte) error {
	const fixed = 16
	if len(data) < fixed {
		return validationError("decode binary Type 2 dump", nil, "binary dump is truncated")
	}
	if !bytes.Equal(data[0:8], BinaryDumpMagic[:]) {
		return validationError("decode binary Type 2 dump", nil, "binary dump magic is invalid")
	}
	if data[8] != BinaryDumpVersion {
		return validationError("decode binary Type 2 dump", nil, fmt.Sprintf("unsupported binary dump version %d", data[8]))
	}
	product, ok := binaryProducts[data[9]]
	if !ok {
		return validationError("decode binary Type 2 dump", nil, fmt.Sprintf("unknown product code %d", data[9]))
	}
	geometry, ok := GeometryFor(product)
	if !ok {
		return validationError("decode binary Type 2 dump", nil, fmt.Sprintf("unsupported product %q", product))
	}
	totalPages := int(binary.BigEndian.Uint16(data[10:12]))
	if totalPages != geometry.TotalPages {
		return validationError("decode binary Type 2 dump", nil, "page count does not match the product geometry")
	}
	uidLength := int(data[12])
	if uidLength != 4 && uidLength != 7 {
		return validationError("decode binary Type 2 dump", nil, "UID length must be 4 or 7 bytes")
	}
	header := fixed + uidLength
	expected := header + totalPages*PageSize
	if len(data) != expected {
		return validationError("decode binary Type 2 dump", nil, fmt.Sprintf("binary dump contains %d bytes; expected %d", len(data), expected))
	}
	uid := data[13 : 13+uidLength]
	atqa := data[13+uidLength : 15+uidLength]
	sak := data[15+uidLength]
	document := Document{
		Format: DumpFormat, FormatVersion: DumpVersion,
		Product: product, ProductName: ProductName(product),
		UID:        strings.ToUpper(hex.EncodeToString(uid)),
		ATQA:       strings.ToUpper(hex.EncodeToString(atqa)),
		SAK:        strings.ToUpper(hex.EncodeToString([]byte{sak})),
		Version:    strings.ToUpper(hex.EncodeToString(VersionForProduct(product))),
		TotalPages: totalPages,
		UserPages:  PageRange{Start: geometry.UserStart, End: geometry.UserEnd},
		Pages:      make([]string, totalPages),
	}
	for index := 0; index < totalPages; index++ {
		page := data[header+index*PageSize : header+(index+1)*PageSize]
		document.Pages[index] = strings.ToUpper(hex.EncodeToString(page))
	}
	if err := document.Validate(); err != nil {
		return err
	}
	pages, err := document.PageBytes()
	if err != nil {
		return err
	}
	document.Refresh(pages)
	*d = document
	return nil
}

// Refresh recomputes the capability container, protection summary, and NDEF
// view from the page image.
func (d *Document) Refresh(pages [][4]byte) {
	geometry, ok := GeometryFor(d.Product)
	if !ok || len(pages) < 4 {
		return
	}
	cc, ccErr := ParseCC(pages[3], geometry)
	d.CC = cc
	d.Protection = DecodeProtection(geometry, pages)
	d.NDEF = nil
	d.NDEFError = ""
	if ccErr != nil {
		d.NDEFError = ccErr.Error()
		return
	}
	if !cc.Valid {
		d.NDEFError = "tag has no valid NDEF capability container"
		return
	}
	area := make([]byte, 0, cc.Capacity)
	for page := geometry.UserStart; page <= geometry.UserEnd && len(area) < cc.Capacity; page++ {
		area = append(area, pages[page][:]...)
	}
	if len(area) < cc.Capacity {
		d.NDEFError = "capability container exceeds user memory"
		return
	}
	ndef, err := ParseNDEF(area[:cc.Capacity])
	if err != nil {
		d.NDEFError = err.Error()
		return
	}
	d.NDEF = ndef
}
