package libnfc

import (
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// connectionHint explains the ownership and daemon rules that make an open
// failure actionable. It never changes the stable error category.
func connectionHint(connString string) string {
	switch nfc.DriverOf(connString) {
	case nfc.DriverPCSC, nfc.DriverACR122PCSC:
		return "PC/SC reader access failed; ensure pcscd is running and that no other process holds the reader"
	case nfc.DriverACR122USB, nfc.DriverPN53XUSB:
		return "USB reader open failed; if pcscd has claimed the reader, close the PC/SC client or use the PC/SC connstring instead"
	default:
		return ""
	}
}

func wrapOpenHint(connString string, err error) error {
	if err == nil {
		return nil
	}
	hint := connectionHint(connString)
	if hint == "" {
		return err
	}
	code, ok := nfc.ErrorCodeOf(err)
	if !ok {
		code = nfc.CodeDeviceDisconnected
	}
	return nfc.NewError("open", code, fmt.Sprintf("%s: %v", hint, err), err)
}
