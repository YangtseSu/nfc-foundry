package nfc

import "strings"

// Driver identifies a libnfc backend from a connstring prefix.
type Driver string

const (
	DriverPN532UART  Driver = "pn532_uart"
	DriverPN53XUSB   Driver = "pn53x_usb"
	DriverACR122USB  Driver = "acr122_usb"
	DriverACR122PCSC Driver = "acr122_pcsc"
	DriverPCSC       Driver = "pcsc"
)

// ValidationLevel records how much of a driver profile has been exercised on
// real hardware. It never changes whether an operation is implemented.
type ValidationLevel string

const (
	ValidationNone       ValidationLevel = "none"
	ValidationPartial    ValidationLevel = "partial"
	ValidationUnverified ValidationLevel = "unverified"
)

// ReaderCapabilities is the explicit support contract for one reader profile.
// Application code must use it instead of inferring support from enumeration.
type ReaderCapabilities struct {
	Driver          Driver          `json:"driver"`
	Name            string          `json:"name,omitempty"`
	Transport       string          `json:"transport,omitempty"`
	Known           bool            `json:"known"`
	Enumerate       bool            `json:"enumerate"`
	SelectISO14443A bool            `json:"selectIso14443a"`
	ClassicRead     bool            `json:"classicRead"`
	ClassicWrite    bool            `json:"classicWrite"`
	Type2Read       bool            `json:"type2Read"`
	Type2Write      bool            `json:"type2Write"`
	RawBytes        bool            `json:"rawBytes"`
	RawBits         bool            `json:"rawBits"`
	APDU            bool            `json:"apdu"`
	Nested          bool            `json:"nested"`
	Darkside        bool            `json:"darkside"`
	Hardnested      bool            `json:"hardnested"`
	Validation      ValidationLevel `json:"validation"`
	Note            string          `json:"note,omitempty"`
}

// UnknownReaderCapabilities keeps development backends and test doubles usable
// without advertising them as validated readers. It is not used for any known
// production driver.
func UnknownReaderCapabilities(driver Driver) ReaderCapabilities {
	return ReaderCapabilities{
		Driver:          driver,
		Name:            string(driver),
		Known:           false,
		Enumerate:       true,
		SelectISO14443A: true,
		ClassicRead:     true,
		ClassicWrite:    true,
		Type2Read:       true,
		Type2Write:      true,
		RawBytes:        true,
		RawBits:         true,
		APDU:            true,
		Validation:      ValidationUnverified,
		Note:            "Unclassified reader profile; operations are attempted and may fail.",
	}
}

var readerProfiles = []ReaderCapabilities{
	{
		Driver:          DriverPN532UART,
		Name:            "PN532 UART",
		Transport:       "UART / PN532",
		Known:           true,
		Enumerate:       true,
		SelectISO14443A: true,
		ClassicRead:     true,
		ClassicWrite:    true,
		Type2Read:       true,
		Type2Write:      true,
		RawBytes:        true,
		RawBits:         true,
		APDU:            true,
		Nested:          true,
		Darkside:        true,
		Hardnested:      true,
		Validation:      ValidationPartial,
		Note:            "Classic and card selection hardware-tested; ISO-DEP card workflows pending.",
	},
	{
		Driver:          DriverPN53XUSB,
		Name:            "PN53x USB",
		Transport:       "USB / PN53x",
		Known:           true,
		Enumerate:       true,
		SelectISO14443A: true,
		ClassicRead:     true,
		ClassicWrite:    true,
		Type2Read:       true,
		Type2Write:      true,
		RawBytes:        true,
		RawBits:         true,
		APDU:            true,
		Validation:      ValidationNone,
		Note:            "Compiled in the application profile; no PN53x USB hardware validation recorded. Recovery engines remain PN532 UART-only.",
	},
	{
		Driver:          DriverACR122USB,
		Name:            "ACS ACR122U",
		Transport:       "USB / ACR122",
		Known:           true,
		Enumerate:       true,
		SelectISO14443A: true,
		ClassicRead:     true,
		ClassicWrite:    true,
		Type2Read:       true,
		Type2Write:      true,
		RawBytes:        true,
		RawBits:         true,
		APDU:            true,
		Validation:      ValidationPartial,
		Note:            "Classic, Gen1A, Type 2/NDEF, and Classic NDEF hardware-tested; ISO-DEP card workflows pending. Recovery engines remain PN532 UART-only.",
	},
	{
		Driver:          DriverACR122PCSC,
		Name:            "ACS ACR122U (PC/SC)",
		Transport:       "PC/SC / ACR122",
		Known:           true,
		Enumerate:       true,
		SelectISO14443A: true,
		ClassicRead:     true,
		ClassicWrite:    true,
		Type2Read:       true,
		Type2Write:      true,
		RawBytes:        true,
		RawBits:         true,
		APDU:            true,
		Validation:      ValidationNone,
		Note:            "ACR122U firmware 2.14 failed to open through this driver; not hardware-validated. Recovery engines remain PN532 UART-only.",
	},
	{
		Driver:          DriverPCSC,
		Name:            "PC/SC reader",
		Transport:       "PC/SC",
		Known:           true,
		Enumerate:       true,
		SelectISO14443A: true,
		Validation:      ValidationNone,
		Note:            "APDU-oriented driver; NFC Foundry has no PC/SC-native APDU transport yet.",
	},
}

// DriverOf returns the driver prefix of a libnfc connstring.
func DriverOf(connString string) Driver {
	prefix := connString
	if index := strings.IndexByte(prefix, ':'); index >= 0 {
		prefix = prefix[:index]
	}
	return Driver(strings.TrimSpace(prefix))
}

// ReaderProfiles returns the known production profiles in canonical order.
func ReaderProfiles() []ReaderCapabilities {
	return append([]ReaderCapabilities(nil), readerProfiles...)
}

// CapabilitiesFor returns the explicit profile for a connstring. Unknown
// drivers are reported as unverified but remain usable for development and
// test transports.
func CapabilitiesFor(connString string) ReaderCapabilities {
	driver := DriverOf(connString)
	for _, profile := range readerProfiles {
		if profile.Driver == driver {
			return profile
		}
	}
	return UnknownReaderCapabilities(driver)
}

// SupportsClassic reports whether both classic read and write are available.
func (c ReaderCapabilities) SupportsClassic() bool {
	return c.ClassicRead && c.ClassicWrite
}

// SupportsType2 reports whether both Type 2 read and write are available.
func (c ReaderCapabilities) SupportsType2() bool {
	return c.Type2Read && c.Type2Write
}

// Advertised reports whether the profile is backed by recorded hardware
// validation.
func (c ReaderCapabilities) Advertised() bool {
	return c.Known && c.Validation != ValidationNone && c.Validation != ValidationUnverified
}
