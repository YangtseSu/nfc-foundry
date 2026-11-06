package type2

import (
	"encoding/hex"
	"strings"
)

// Protection is the decoded protection state of one Type 2 tag. Password and
// PACK values are deliberately omitted; only their presence is reported.
type Protection struct {
	StaticLock   string `json:"staticLock"`
	DynamicLock  string `json:"dynamicLock,omitempty"`
	Auth0        int    `json:"auth0"`
	Access       byte   `json:"access"`
	Protect      bool   `json:"protect"`
	ConfigLocked bool   `json:"configLocked"`
	AuthLimit    int    `json:"authLimit"`
	PasswordSet  bool   `json:"passwordSet"`
	PackSet      bool   `json:"packSet"`
}

// DecodeProtection extracts static lock, dynamic lock, and configuration facts.
func DecodeProtection(geometry Geometry, pages [][4]byte) Protection {
	protection := Protection{Auth0: -1}
	if len(pages) > 2 {
		protection.StaticLock = strings.ToUpper(hex.EncodeToString(pages[2][2:4]))
	}
	if geometry.DynamicLockStart >= 0 && geometry.DynamicLockStart < len(pages) {
		protection.DynamicLock = strings.ToUpper(hex.EncodeToString(pages[geometry.DynamicLockStart][0:3]))
	}
	if geometry.ConfigStart >= 0 && geometry.ConfigStart < len(pages) {
		config0 := pages[geometry.ConfigStart]
		protection.Auth0 = int(config0[3])
	}
	if geometry.ConfigStart+1 >= 0 && geometry.ConfigStart+1 < len(pages) {
		config1 := pages[geometry.ConfigStart+1]
		protection.Access = config1[0]
		protection.Protect = config1[0]&0x80 != 0
		protection.ConfigLocked = config1[0]&0x40 != 0
		protection.AuthLimit = int(config1[0] & 0x07)
	}
	if geometry.PasswordPage >= 0 && geometry.PasswordPage < len(pages) {
		protection.PasswordSet = pages[geometry.PasswordPage] != [4]byte{}
	}
	if geometry.PackPage >= 0 && geometry.PackPage < len(pages) {
		protection.PackSet = pages[geometry.PackPage][0] != 0
	}
	return protection
}
