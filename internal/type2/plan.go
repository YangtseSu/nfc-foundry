package type2

import (
	"encoding/hex"
	"fmt"
	"strings"
)

// PageWrite is one planned page update.
type PageWrite struct {
	Page   int    `json:"page"`
	Hex    string `json:"hex"`
	Region string `json:"region"`
}

// Data returns the four bytes to write.
func (w PageWrite) Data() ([4]byte, error) {
	var data [4]byte
	decoded, err := hex.DecodeString(w.Hex)
	if err != nil || len(decoded) != PageSize {
		return data, validationError("plan page write", err, fmt.Sprintf("page %d must be four hexadecimal bytes", w.Page))
	}
	copy(data[:], decoded)
	return data, nil
}

func newPageWrite(geometry Geometry, page int, data [4]byte) PageWrite {
	return PageWrite{Page: page, Hex: strings.ToUpper(hex.EncodeToString(data[:])), Region: geometry.Region(page)}
}

// PlanNDEFWrites validates an NDEF message against the dump and returns only
// the user-memory pages that differ from the current contents.
func PlanNDEFWrites(document Document, message []byte) ([]PageWrite, error) {
	geometry, err := document.Geometry()
	if err != nil {
		return nil, err
	}
	if !document.CC.Valid {
		return nil, fmt.Errorf("%w: tag is not formatted for NDEF", ErrMalformedCC)
	}
	if !document.CC.Writable() {
		return nil, fmt.Errorf("%w: capability container is read-only", ErrProtectedPage)
	}
	area, err := document.NDEFArea()
	if err != nil {
		return nil, err
	}
	tlvs, err := parseTLVs(area)
	if err != nil {
		return nil, err
	}
	ndefCount := 0
	for _, entry := range tlvs {
		switch entry.kind {
		case tlvNDEF:
			ndefCount++
		case tlvLockControl, tlvMemoryControl:
			return nil, fmt.Errorf("%w: TLV 0x%02X precedes the NDEF TLV", ErrUnsupportedTLV, entry.kind)
		default:
			return nil, fmt.Errorf("%w: TLV 0x%02X precedes the NDEF TLV", ErrUnsupportedTLV, entry.kind)
		}
	}
	if ndefCount > 1 {
		return nil, fmt.Errorf("%w: more than one NDEF TLV", ErrMalformedTLV)
	}
	desired, err := BuildNDEFArea(document.CC.Capacity, message)
	if err != nil {
		return nil, err
	}
	pages, err := document.PageBytes()
	if err != nil {
		return nil, err
	}
	writes := make([]PageWrite, 0, len(desired)/PageSize)
	for index := 0; index < len(desired)/PageSize; index++ {
		page := geometry.UserStart + index
		if page > geometry.UserEnd {
			return nil, fmt.Errorf("%w: NDEF area exceeds user memory", ErrCapacity)
		}
		var expected [4]byte
		copy(expected[:], desired[index*PageSize:(index+1)*PageSize])
		if pages[page] == expected {
			continue
		}
		writes = append(writes, newPageWrite(geometry, page, expected))
	}
	return writes, nil
}

// RestorePlan returns the pages required to restore a dump. User memory is
// always included. Protected pages are appended only when includeProtected is
// true, and lock bytes are written last.
func RestorePlan(document Document, includeProtected bool) ([]PageWrite, error) {
	geometry, err := document.Geometry()
	if err != nil {
		return nil, err
	}
	pages, err := document.PageBytes()
	if err != nil {
		return nil, err
	}
	writes := make([]PageWrite, 0, geometry.UserPages()+4)
	for page := geometry.UserStart; page <= geometry.UserEnd; page++ {
		writes = append(writes, newPageWrite(geometry, page, pages[page]))
	}
	if !includeProtected {
		return writes, nil
	}
	if geometry.PasswordPage >= 0 {
		writes = append(writes, newPageWrite(geometry, geometry.PasswordPage, pages[geometry.PasswordPage]))
	}
	if geometry.PackPage >= 0 {
		writes = append(writes, newPageWrite(geometry, geometry.PackPage, pages[geometry.PackPage]))
	}
	if geometry.ConfigStart >= 0 {
		for page := geometry.ConfigStart; page <= geometry.ConfigEnd; page++ {
			writes = append(writes, newPageWrite(geometry, page, pages[page]))
		}
	}
	if geometry.DynamicLockStart >= 0 {
		writes = append(writes, newPageWrite(geometry, geometry.DynamicLockStart, pages[geometry.DynamicLockStart]))
	}
	for _, write := range writes {
		if write.Page < geometry.UserStart && !geometry.InProtectedArea(write.Page) {
			return nil, fmt.Errorf("%w: restore plan contains page %d", ErrProtectedPage, write.Page)
		}
	}
	return writes, nil
}

// ValidateUserPage rejects protected and out-of-range page writes.
func ValidateUserPage(geometry Geometry, page int) error {
	if !geometry.InUserMemory(page) {
		return fmt.Errorf("%w: page %d is not user memory for %s", ErrProtectedPage, page, geometry.Name)
	}
	return nil
}

// ValidateWritablePage rejects pages 0-3 and reserved areas.
func ValidateWritablePage(geometry Geometry, page int) error {
	if geometry.InUserMemory(page) {
		return nil
	}
	if geometry.InProtectedArea(page) {
		return nil
	}
	return fmt.Errorf("%w: page %d is read-only on %s", ErrProtectedPage, page, geometry.Name)
}
