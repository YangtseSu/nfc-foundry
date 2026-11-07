package libnfc

import "sort"

// RuntimeInfo is safe to expose through diagnostics; it contains no native
// handles or user-specific paths.
type RuntimeInfo struct {
	Available bool     `json:"available"`
	Version   string   `json:"version,omitempty"`
	Drivers   []string `json:"drivers"`
	Error     string   `json:"error,omitempty"`
}

// Info reports the linked native runtime and the driver set compiled into it.
// Driver availability in a build is not a hardware-validation claim; the
// capability profiles in internal/nfc define the advertised support level.
func Info() RuntimeInfo {
	runtime, result := platformRuntimeInfo()
	info := RuntimeInfo{
		Available: result.code == nativeOK,
		Version:   runtime.version,
		Drivers:   append([]string{}, runtime.drivers...),
	}
	if result.code != nativeOK {
		info.Version = ""
		info.Drivers = []string{}
		info.Error = result.detail
		return info
	}
	sort.Strings(info.Drivers)
	return info
}
