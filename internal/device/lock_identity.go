package device

import (
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// readerLockIdentity returns the physical reader identity used for the
// cross-process lock. Direct USB and PC/SC connstrings for an ACR122 refer to
// the same device, so they must share one lock. Other drivers use their exact
// connstring because their aliases cannot be correlated reliably.
func readerLockIdentity(connString string) string {
	switch nfc.DriverOf(connString) {
	case nfc.DriverACR122USB, nfc.DriverACR122PCSC:
		return "acr122"
	case nfc.DriverPCSC:
		if isACR122PCSCReader(connString) {
			return "acr122"
		}
	}
	return connString
}
