package app

import (
	"fmt"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func (s *Service) requireReaderCapability(op string, allowed func(nfc.ReaderCapabilities) bool, feature string) error {
	if s == nil || s.deviceManager == nil {
		return nil
	}
	snapshot := s.deviceManager.Snapshot()
	if allowed(snapshot.Capabilities) {
		return nil
	}
	return nfc.NewError(op, nfc.CodeUnsupported,
		fmt.Sprintf("reader %q does not support %s", snapshot.Device.ConnString, feature), nil)
}

func (s *Service) requireClassicRead(op string) error {
	return s.requireReaderCapability(op, func(c nfc.ReaderCapabilities) bool { return c.ClassicRead }, "MIFARE Classic read operations")
}

func (s *Service) requireClassicWrite(op string) error {
	return s.requireReaderCapability(op, func(c nfc.ReaderCapabilities) bool { return c.ClassicWrite }, "MIFARE Classic write operations")
}

func (s *Service) requireType2Read(op string) error {
	return s.requireReaderCapability(op, func(c nfc.ReaderCapabilities) bool { return c.Type2Read }, "Type 2 read operations")
}

func (s *Service) requireType2Write(op string) error {
	return s.requireReaderCapability(op, func(c nfc.ReaderCapabilities) bool { return c.Type2Write }, "Type 2 write operations")
}

func (s *Service) requireAPDU(op string) error {
	return s.requireReaderCapability(op, func(c nfc.ReaderCapabilities) bool { return c.APDU }, "ISO-DEP/APDU operations")
}
