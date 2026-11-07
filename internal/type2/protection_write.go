package type2

import (
	"encoding/hex"
	"fmt"
	"strings"
)

// accessWritableMask selects the ACCESS bits a protected write may change:
// PROT, CFGLCK, the NFC counter flags, and AUTHLIM. Bit 3 is RFUI.
const accessWritableMask = 0x80 | 0x40 | 0x30 | 0x07

// ProtectionWriteOptions selects individual protection fields. A nil pointer
// leaves that field unchanged.
type ProtectionWriteOptions struct {
	Auth0       *int
	Access      *byte
	Password    *[4]byte
	Pack        *[2]byte
	DynamicLock *[3]byte
	StaticLock  *uint16
}

// PlanProtectionWrites validates a per-field protection edit and returns the
// pages to write in safe order: PWD/PACK, configuration, dynamic lock, and
// static lock last. Lock bits may only be set.
func PlanProtectionWrites(target Document, options ProtectionWriteOptions) ([]PageWrite, []Change, error) {
	if options.Auth0 == nil && options.Access == nil && options.Password == nil &&
		options.Pack == nil && options.DynamicLock == nil && options.StaticLock == nil {
		return nil, nil, fmt.Errorf("%w: no protection field was selected", ErrProtectedPage)
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

	// Password and PACK precede configuration so a new AUTH0/PROT pair
	// applies to the final state.
	if options.Password != nil {
		if geometry.PasswordPage < 0 {
			return nil, nil, fmt.Errorf("%w: %s has no password page", ErrUnsupported, geometry.Name)
		}
		old := pages[geometry.PasswordPage]
		updated := [4]byte(*options.Password)
		if updated != old {
			if target.Protection.ConfigLocked {
				return nil, nil, fmt.Errorf("%w: configuration is locked (CFGLCK)", ErrProtectedPage)
			}
			writes = append(writes, newPageWrite(geometry, geometry.PasswordPage, updated))
			changes = append(changes, Change{Target: "password", Old: "current", New: "new 4-byte value"})
		}
	}
	if options.Pack != nil {
		if geometry.PackPage < 0 {
			return nil, nil, fmt.Errorf("%w: %s has no PACK page", ErrUnsupported, geometry.Name)
		}
		old := pages[geometry.PackPage]
		updated := old
		updated[0], updated[1] = options.Pack[0], options.Pack[1]
		if updated != old {
			if target.Protection.ConfigLocked {
				return nil, nil, fmt.Errorf("%w: configuration is locked (CFGLCK)", ErrProtectedPage)
			}
			writes = append(writes, newPageWrite(geometry, geometry.PackPage, updated))
			changes = append(changes, pageChange(geometry, "PACK", old, updated, false))
		}
	}
	if options.Auth0 != nil || options.Access != nil {
		if geometry.ConfigStart < 0 || geometry.ConfigEnd < 0 {
			return nil, nil, fmt.Errorf("%w: %s has no configuration pages", ErrUnsupported, geometry.Name)
		}
		if options.Auth0 != nil {
			if *options.Auth0 < 0 || *options.Auth0 > 255 {
				return nil, nil, fmt.Errorf("%w: AUTH0 must be between 0 and 255", ErrProtectedPage)
			}
			old := pages[geometry.ConfigStart]
			updated := old
			updated[3] = byte(*options.Auth0)
			if updated != old {
				if target.Protection.ConfigLocked {
					return nil, nil, fmt.Errorf("%w: configuration is locked (CFGLCK)", ErrProtectedPage)
				}
				writes = append(writes, newPageWrite(geometry, geometry.ConfigStart, updated))
				changes = append(changes, Change{Target: "AUTH0", Old: auth0Name(int(old[3])), New: auth0Name(*options.Auth0)})
			}
		}
		if options.Access != nil {
			old := pages[geometry.ConfigEnd]
			updated := old
			updated[0] = old[0]&^byte(accessWritableMask) | *options.Access&accessWritableMask
			if old[0]&0x40 != 0 && updated[0]&0x40 == 0 {
				return nil, nil, fmt.Errorf("%w: CFGLCK is one-way and cannot be cleared", ErrProtectedPage)
			}
			if updated != old {
				if target.Protection.ConfigLocked {
					return nil, nil, fmt.Errorf("%w: configuration is locked (CFGLCK)", ErrProtectedPage)
				}
				writes = append(writes, newPageWrite(geometry, geometry.ConfigEnd, updated))
				changes = append(changes, Change{
					Target: "ACCESS", Old: strings.ToUpper(hex.EncodeToString(old[:1])),
					New: strings.ToUpper(hex.EncodeToString(updated[:1])), OneWay: updated[0]&0x40 != 0,
				})
			}
		}
	}
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
	// Static lock is last because it is the most likely one-way operation to
	// make later writes fail.
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
