package type2

import (
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"strings"
)

// Change is one old/new value shown before a destructive operation.
type Change struct {
	Target string `json:"target"`
	Old    string `json:"old"`
	New    string `json:"new"`
	OneWay bool   `json:"oneWay"`
}

// CopyOptions controls a Type 2 content copy.
type CopyOptions struct {
	IncludeProtected bool
	SameUIDOnly      bool
}

// LockOptions selects one or more one-way lock targets. A nil pointer leaves
// that target unchanged.
type LockOptions struct {
	StaticLock  *uint16
	DynamicLock *[3]byte
	CCReadOnly  bool
	ConfigLock  bool
}

// PasswordOptions sets PWD, PACK, AUTH0, and the PROT bit.
type PasswordOptions struct {
	Password [4]byte
	Pack     [2]byte
	Auth0    int
	Protect  bool
}

// PlanCopy returns the pages that copy source content onto target. UID pages
// are never written.
func PlanCopy(source, target Document, options CopyOptions) ([]PageWrite, []Change, error) {
	if err := source.Validate(); err != nil {
		return nil, nil, err
	}
	if err := target.Validate(); err != nil {
		return nil, nil, err
	}
	if source.Product != target.Product {
		return nil, nil, fmt.Errorf("%w: source is %s and target is %s", ErrCapacity, source.ProductName, target.ProductName)
	}
	if options.SameUIDOnly && source.UID != target.UID {
		return nil, nil, fmt.Errorf("%w: source UID %s does not match target UID %s", ErrProtectedPage, source.UID, target.UID)
	}
	if !source.CC.Valid {
		return nil, nil, fmt.Errorf("%w: source has no valid NDEF capability container", ErrMalformedCC)
	}
	geometry, err := target.Geometry()
	if err != nil {
		return nil, nil, err
	}
	if target.CC.Valid && !target.CC.Writable() && !options.IncludeProtected {
		return nil, nil, fmt.Errorf("%w: target capability container is read-only", ErrProtectedPage)
	}
	if !target.CC.Valid && !options.IncludeProtected {
		return nil, nil, fmt.Errorf("%w: target is not formatted for NDEF", ErrMalformedCC)
	}
	sourcePages, err := source.PageBytes()
	if err != nil {
		return nil, nil, err
	}
	targetPages, err := target.PageBytes()
	if err != nil {
		return nil, nil, err
	}
	locked, err := lockedUserPages(geometry, targetPages)
	if err != nil {
		return nil, nil, err
	}
	if len(locked) > 0 {
		return nil, nil, fmt.Errorf("%w: target user pages %v are locked", ErrProtectedPage, locked)
	}
	if source.NDEF != nil && source.NDEF.Present && target.CC.Valid {
		message, err := hex.DecodeString(source.NDEF.Raw)
		if err != nil {
			return nil, nil, fmt.Errorf("%w: source NDEF raw data is invalid", ErrMalformedNDEF)
		}
		if _, err := BuildNDEFArea(target.CC.Capacity, message); err != nil {
			return nil, nil, err
		}
	}

	var writes []PageWrite
	var changes []Change
	differing := 0
	for page := geometry.UserStart; page <= geometry.UserEnd; page++ {
		if sourcePages[page] == targetPages[page] {
			continue
		}
		differing++
		writes = append(writes, newPageWrite(geometry, page, sourcePages[page]))
	}
	if differing > 0 {
		changes = append(changes, Change{
			Target: "user memory",
			Old:    fmt.Sprintf("%d differing pages", differing),
			New:    fmt.Sprintf("%d pages from %s", differing, source.UID),
		})
	}
	if !options.IncludeProtected {
		return writes, changes, nil
	}
	if target.Protection.ConfigLocked {
		return nil, nil, fmt.Errorf("%w: target configuration is locked (CFGLCK)", ErrProtectedPage)
	}

	// Password and PACK go before configuration so a new AUTH0/PROT pair
	// applies to the final state.
	if geometry.PasswordPage >= 0 && geometry.PackPage >= 0 {
		if sourcePages[geometry.PasswordPage] != targetPages[geometry.PasswordPage] {
			writes = append(writes, newPageWrite(geometry, geometry.PasswordPage, sourcePages[geometry.PasswordPage]))
			changes = append(changes, pageChange(geometry, "password", targetPages[geometry.PasswordPage], sourcePages[geometry.PasswordPage], false))
		}
		if sourcePages[geometry.PackPage] != targetPages[geometry.PackPage] {
			writes = append(writes, newPageWrite(geometry, geometry.PackPage, sourcePages[geometry.PackPage]))
			changes = append(changes, pageChange(geometry, "PACK", targetPages[geometry.PackPage], sourcePages[geometry.PackPage], false))
		}
	}
	if geometry.ConfigStart >= 0 {
		for page := geometry.ConfigStart; page <= geometry.ConfigEnd; page++ {
			if sourcePages[page] == targetPages[page] {
				continue
			}
			if page == geometry.ConfigEnd && sourcePages[page][0]&0x40 != 0 {
				changes = append(changes, Change{
					Target: "configuration lock", Old: "unlocked", New: "locked", OneWay: true,
				})
			}
			writes = append(writes, newPageWrite(geometry, page, sourcePages[page]))
		}
		if sourcePages[geometry.ConfigStart] != targetPages[geometry.ConfigStart] ||
			sourcePages[geometry.ConfigEnd] != targetPages[geometry.ConfigEnd] {
			changes = append(changes, Change{
				Target: "configuration", Old: "target values", New: "source values",
			})
		}
	}

	// A read-only capability container is the last protected change.
	targetCC := targetPages[3]
	sourceCC := sourcePages[3]
	if target.Product == ProductUltralight {
		return nil, nil, fmt.Errorf("%w: MIFARE Ultralight page 3 is OTP and cannot be rewritten", ErrProtectedPage)
	}
	if targetPages[2][2]&0x01 != 0 {
		return nil, nil, fmt.Errorf("%w: capability container is locked by static lock bit 0", ErrProtectedPage)
	}
	if targetCC == [4]byte{} {
		newCC := sourceCC
		if _, err := ParseCC(newCC, geometry); err != nil {
			return nil, nil, err
		}
		writes = append(writes, newPageWrite(geometry, 3, newCC))
		changes = append(changes, pageChange(geometry, "capability container", targetCC, newCC, true))
	} else {
		if !target.CC.Valid || !source.CC.Valid {
			return nil, nil, fmt.Errorf("%w: capability container cannot be copied", ErrMalformedCC)
		}
		newCC := targetCC
		newCC[3] |= sourceCC[3]
		if newCC != targetCC {
			writes = append(writes, newPageWrite(geometry, 3, newCC))
			changes = append(changes, pageChange(geometry, "capability access", targetCC, newCC, true))
		}
	}
	return writes, changes, nil
}

// PlanErase zeroes every unlocked user-memory page.
func PlanErase(target Document) ([]PageWrite, []Change, error) {
	if err := target.Validate(); err != nil {
		return nil, nil, err
	}
	geometry, err := target.Geometry()
	if err != nil {
		return nil, nil, err
	}
	pages, err := target.PageBytes()
	if err != nil {
		return nil, nil, err
	}
	locked, err := lockedUserPages(geometry, pages)
	if err != nil {
		return nil, nil, err
	}
	lockedSet := make(map[int]struct{}, len(locked))
	for _, page := range locked {
		lockedSet[page] = struct{}{}
	}
	var writes []PageWrite
	skipped := 0
	for page := geometry.UserStart; page <= geometry.UserEnd; page++ {
		if _, isLocked := lockedSet[page]; isLocked {
			skipped++
			continue
		}
		if pages[page] == [4]byte{} {
			continue
		}
		writes = append(writes, newPageWrite(geometry, page, [4]byte{}))
	}
	changes := []Change{{
		Target: "user memory",
		Old:    fmt.Sprintf("%d pages", geometry.UserPages()),
		New:    "zeroed; lock, configuration, PWD, and PACK pages remain",
	}}
	if skipped > 0 {
		changes = append(changes, Change{Target: "locked pages", Old: fmt.Sprintf("%d pages", skipped), New: "skipped"})
	}
	return writes, changes, nil
}

// PlanLock changes one or more one-way lock targets.
func PlanLock(target Document, options LockOptions) ([]PageWrite, []Change, error) {
	if options.StaticLock == nil && options.DynamicLock == nil && !options.CCReadOnly && !options.ConfigLock {
		return nil, nil, fmt.Errorf("%w: no lock target was selected", ErrProtectedPage)
	}
	if err := target.Validate(); err != nil {
		return nil, nil, err
	}
	geometry, err := target.Geometry()
	if err != nil {
		return nil, nil, err
	}
	pages, err := target.PageBytes()
	if err != nil {
		return nil, nil, err
	}
	var writes []PageWrite
	var changes []Change
	if options.DynamicLock != nil {
		if geometry.DynamicLockStart < 0 {
			return nil, nil, fmt.Errorf("%w: %s has no writable dynamic lock page", ErrUnsupported, geometry.Name)
		}
		old := pages[geometry.DynamicLockStart]
		requested := *options.DynamicLock
		if old[0]&^requested[0] != 0 || old[1]&^requested[1] != 0 || old[2]&^requested[2] != 0 {
			return nil, nil, fmt.Errorf("%w: dynamic lock bits cannot be cleared", ErrProtectedPage)
		}
		updated := old
		updated[0] |= requested[0]
		updated[1] |= requested[1]
		updated[2] |= requested[2]
		if updated != old {
			writes = append(writes, newPageWrite(geometry, geometry.DynamicLockStart, updated))
			changes = append(changes, pageChange(geometry, "dynamic lock", old, updated, true))
		}
	}
	if options.ConfigLock {
		if geometry.ConfigEnd < 0 {
			return nil, nil, fmt.Errorf("%w: %s has no configuration page", ErrUnsupported, geometry.Name)
		}
		old := pages[geometry.ConfigEnd]
		if old[0]&0x40 != 0 {
			return nil, nil, fmt.Errorf("%w: configuration is already locked", ErrProtectedPage)
		}
		updated := old
		updated[0] |= 0x40
		writes = append(writes, newPageWrite(geometry, geometry.ConfigEnd, updated))
		changes = append(changes, pageChange(geometry, "configuration lock", old, updated, true))
	}
	if options.CCReadOnly {
		if target.Product == ProductUltralight {
			return nil, nil, fmt.Errorf("%w: MIFARE Ultralight page 3 is OTP", ErrProtectedPage)
		}
		if pages[2][2]&0x01 != 0 {
			return nil, nil, fmt.Errorf("%w: capability container is locked by static lock bit 0", ErrProtectedPage)
		}
		if !target.CC.Valid {
			return nil, nil, fmt.Errorf("%w: target has no valid capability container", ErrMalformedCC)
		}
		old := pages[3]
		updated := old
		updated[3] |= 0x0F
		if updated != old {
			writes = append(writes, newPageWrite(geometry, 3, updated))
			changes = append(changes, pageChange(geometry, "capability access", old, updated, true))
		}
	}
	// Static lock is last because it may prevent every later page write.
	if options.StaticLock != nil {
		old := pages[2]
		requested := *options.StaticLock
		requestedBytes := [2]byte{byte(requested), byte(requested >> 8)}
		// Bits 13-15 of the static lock bytes are RFUI and stay unchanged.
		requestedBytes[1] &= 0x1F
		if old[2]&^requestedBytes[0] != 0 || old[3]&^requestedBytes[1] != 0 {
			return nil, nil, fmt.Errorf("%w: static lock bits cannot be cleared", ErrProtectedPage)
		}
		updated := old
		updated[2] |= requestedBytes[0]
		updated[3] |= requestedBytes[1]
		if updated != old {
			writes = append(writes, newPageWrite(geometry, 2, updated))
			changes = append(changes, pageChange(geometry, "static lock", old, updated, true))
		}
	}
	return writes, changes, nil
}

// PlanPasswordSet writes PWD, PACK, AUTH0, and the PROT bit. Auth0 below zero
// uses the first user-memory page.
func PlanPasswordSet(target Document, options PasswordOptions) ([]PageWrite, []Change, error) {
	if err := target.Validate(); err != nil {
		return nil, nil, err
	}
	geometry, err := target.Geometry()
	if err != nil {
		return nil, nil, err
	}
	if geometry.PasswordPage < 0 || geometry.ConfigStart < 0 {
		return nil, nil, fmt.Errorf("%w: %s has no password or configuration pages", ErrUnsupported, geometry.Name)
	}
	if target.Protection.ConfigLocked {
		return nil, nil, fmt.Errorf("%w: configuration is locked (CFGLCK)", ErrProtectedPage)
	}
	auth0 := options.Auth0
	if auth0 < 0 {
		auth0 = geometry.UserStart
	}
	if auth0 < geometry.UserStart || auth0 > geometry.UserEnd {
		return nil, nil, fmt.Errorf("%w: AUTH0 must be a user-memory page", ErrProtectedPage)
	}
	pages, err := target.PageBytes()
	if err != nil {
		return nil, nil, err
	}
	var writes []PageWrite
	var changes []Change
	password := [4]byte(options.Password)
	pack := [4]byte{options.Pack[0], options.Pack[1], pages[geometry.PackPage][2], pages[geometry.PackPage][3]}
	if pages[geometry.PasswordPage] != password {
		writes = append(writes, newPageWrite(geometry, geometry.PasswordPage, password))
		changes = append(changes, Change{Target: "password", Old: "current", New: "new 4-byte value"})
	}
	if pages[geometry.PackPage] != pack {
		writes = append(writes, newPageWrite(geometry, geometry.PackPage, pack))
		changes = append(changes, pageChange(geometry, "PACK", pages[geometry.PackPage], pack, false))
	}
	config0 := pages[geometry.ConfigStart]
	updatedConfig0 := config0
	updatedConfig0[3] = byte(auth0)
	if updatedConfig0 != config0 {
		writes = append(writes, newPageWrite(geometry, geometry.ConfigStart, updatedConfig0))
		changes = append(changes, Change{Target: "AUTH0", Old: auth0Name(int(config0[3])), New: auth0Name(auth0)})
	}
	config1 := pages[geometry.ConfigEnd]
	updatedConfig1 := config1
	if options.Protect {
		updatedConfig1[0] |= 0x80
	} else {
		updatedConfig1[0] &^= 0x80
	}
	if updatedConfig1 != config1 {
		writes = append(writes, newPageWrite(geometry, geometry.ConfigEnd, updatedConfig1))
		changes = append(changes, Change{
			Target: "PROT", Old: fmt.Sprintf("0x%02X", config1[0]), New: fmt.Sprintf("0x%02X", updatedConfig1[0]),
		})
	}
	return writes, changes, nil
}

// PlanPasswordRemove disables password protection. The workflow must
// authenticate with the current password when the target is protected.
func PlanPasswordRemove(target Document) ([]PageWrite, []Change, error) {
	if err := target.Validate(); err != nil {
		return nil, nil, err
	}
	geometry, err := target.Geometry()
	if err != nil {
		return nil, nil, err
	}
	if geometry.PasswordPage < 0 || geometry.ConfigStart < 0 {
		return nil, nil, fmt.Errorf("%w: %s has no password or configuration pages", ErrUnsupported, geometry.Name)
	}
	if target.Protection.ConfigLocked {
		return nil, nil, fmt.Errorf("%w: configuration is locked (CFGLCK)", ErrProtectedPage)
	}
	pages, err := target.PageBytes()
	if err != nil {
		return nil, nil, err
	}
	var writes []PageWrite
	var changes []Change
	password := [4]byte{0xFF, 0xFF, 0xFF, 0xFF}
	if pages[geometry.PasswordPage] != password {
		writes = append(writes, newPageWrite(geometry, geometry.PasswordPage, password))
		changes = append(changes, Change{Target: "password", Old: "set", New: "FF FF FF FF"})
	}
	pack := pages[geometry.PackPage]
	updatedPack := [4]byte{0, 0, pack[2], pack[3]}
	if updatedPack != pack {
		writes = append(writes, newPageWrite(geometry, geometry.PackPage, updatedPack))
		changes = append(changes, pageChange(geometry, "PACK", pack, updatedPack, false))
	}
	config0 := pages[geometry.ConfigStart]
	updatedConfig0 := config0
	updatedConfig0[3] = 0xFF
	if updatedConfig0 != config0 {
		writes = append(writes, newPageWrite(geometry, geometry.ConfigStart, updatedConfig0))
		changes = append(changes, Change{Target: "AUTH0", Old: auth0Name(int(config0[3])), New: "disabled (FF)"})
	}
	config1 := pages[geometry.ConfigEnd]
	updatedConfig1 := config1
	updatedConfig1[0] &^= 0x80
	if updatedConfig1 != config1 {
		writes = append(writes, newPageWrite(geometry, geometry.ConfigEnd, updatedConfig1))
		changes = append(changes, Change{
			Target: "PROT", Old: fmt.Sprintf("0x%02X", config1[0]), New: fmt.Sprintf("0x%02X", updatedConfig1[0]),
		})
	}
	return writes, changes, nil
}

func pageChange(geometry Geometry, target string, old, updated [4]byte, oneWay bool) Change {
	return Change{
		Target: target,
		Old:    strings.ToUpper(hex.EncodeToString(old[:])),
		New:    strings.ToUpper(hex.EncodeToString(updated[:])),
		OneWay: oneWay,
	}
}

func auth0Name(value int) string {
	if value == 0xFF {
		return "disabled (FF)"
	}
	return fmt.Sprintf("%d", value)
}

// lockedUserPages returns static-locked user pages and dynamic-locked pages
// using the NTAG21x group sizes (two pages on NTAG213, sixteen on NTAG215/216).
func lockedUserPages(geometry Geometry, pages [][4]byte) ([]int, error) {
	if len(pages) < 4 {
		return nil, fmt.Errorf("%w: image has no lock page", ErrMalformedTLV)
	}
	locked := make(map[int]struct{})
	static := binary.LittleEndian.Uint16(pages[2][2:4])
	for page := 4; page <= geometry.UserEnd; page++ {
		if page <= 15 && static&(1<<uint(page-3)) != 0 {
			locked[page] = struct{}{}
		}
	}
	if geometry.DynamicLockStart >= 0 && geometry.DynamicLockStart < len(pages) {
		dynamic := pages[geometry.DynamicLockStart]
		group := 16
		if geometry.Product == ProductUltralight {
			group = 0
		} else if geometry.Product == ProductNTAG213 {
			group = 2
		}
		if group > 0 {
			for bit := 0; bit < 24; bit++ {
				if dynamic[bit/8]&(1<<uint(bit%8)) == 0 {
					continue
				}
				start := 16 + bit*group
				for page := start; page < start+group; page++ {
					if page >= geometry.UserStart && page <= geometry.UserEnd {
						locked[page] = struct{}{}
					}
				}
			}
		}
	}
	result := make([]int, 0, len(locked))
	for page := geometry.UserStart; page <= geometry.UserEnd; page++ {
		if _, ok := locked[page]; ok {
			result = append(result, page)
		}
	}
	return result, nil
}
