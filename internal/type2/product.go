// Package type2 implements NFC Forum Type 2 tag commands, memory geometry,
// dump validation, and NDEF access. It contains no reader or process logic.
package type2

import "fmt"

// Product is the exact Type 2 product family detected for a tag.
type Product string

const (
	ProductUnknown    Product = "unknown"
	ProductUltralight Product = "mifare_ultralight"
	ProductNTAG213    Product = "ntag213"
	ProductNTAG215    Product = "ntag215"
	ProductNTAG216    Product = "ntag216"
)

// Geometry describes one validated Type 2 memory map. Page numbers are
// inclusive. A negative optional page or start means the product has no such
// area.
type Geometry struct {
	Product          Product
	Name             string
	TotalPages       int
	UserStart        int
	UserEnd          int
	DynamicLockStart int
	ConfigStart      int
	ConfigEnd        int
	PasswordPage     int
	PackPage         int
}

var geometries = map[Product]Geometry{
	ProductUltralight: {
		Product: ProductUltralight, Name: "MIFARE Ultralight",
		TotalPages: 16, UserStart: 4, UserEnd: 15,
		DynamicLockStart: -1, ConfigStart: -1, ConfigEnd: -1, PasswordPage: -1, PackPage: -1,
	},
	ProductNTAG213: {
		Product: ProductNTAG213, Name: "NTAG213",
		TotalPages: 45, UserStart: 4, UserEnd: 39,
		DynamicLockStart: 40, ConfigStart: 41, ConfigEnd: 42, PasswordPage: 43, PackPage: 44,
	},
	ProductNTAG215: {
		Product: ProductNTAG215, Name: "NTAG215",
		TotalPages: 135, UserStart: 4, UserEnd: 129,
		DynamicLockStart: 130, ConfigStart: 131, ConfigEnd: 132, PasswordPage: 133, PackPage: 134,
	},
	ProductNTAG216: {
		Product: ProductNTAG216, Name: "NTAG216",
		TotalPages: 231, UserStart: 4, UserEnd: 225,
		DynamicLockStart: 226, ConfigStart: 227, ConfigEnd: 228, PasswordPage: 229, PackPage: 230,
	},
}

// GeometryFor returns the validated memory map for a product.
func GeometryFor(product Product) (Geometry, bool) {
	geometry, ok := geometries[product]
	return geometry, ok
}

// ProductName returns the display name for a product code.
func ProductName(product Product) string {
	if geometry, ok := geometries[product]; ok {
		return geometry.Name
	}
	return "Unknown Type 2 tag"
}

// Valid reports whether the geometry is a supported memory map.
func (g Geometry) Valid() bool {
	return g.TotalPages > 0 && g.UserStart >= 0 && g.UserEnd >= g.UserStart && g.UserEnd < g.TotalPages
}

// UserPages returns the number of user-memory pages.
func (g Geometry) UserPages() int {
	if !g.Valid() {
		return 0
	}
	return g.UserEnd - g.UserStart + 1
}

// UserBytes returns the user-memory size in bytes.
func (g Geometry) UserBytes() int { return g.UserPages() * 4 }

// InUserMemory reports whether page is part of the user-memory range.
func (g Geometry) InUserMemory(page int) bool {
	return g.Valid() && page >= g.UserStart && page <= g.UserEnd
}

// InProtectedArea reports whether page is a dynamic-lock, configuration, or
// password page. Those pages require the explicitly confirmed restore path.
func (g Geometry) InProtectedArea(page int) bool {
	if !g.Valid() {
		return false
	}
	if g.DynamicLockStart >= 0 && page == g.DynamicLockStart {
		return true
	}
	if g.ConfigStart >= 0 && page >= g.ConfigStart && page <= g.ConfigEnd {
		return true
	}
	return page == g.PasswordPage || page == g.PackPage
}

// Region returns the stable region label shown by the CLI and GUI.
func (g Geometry) Region(page int) string {
	if !g.Valid() || page < 0 || page >= g.TotalPages {
		return "outside"
	}
	switch {
	case page <= 1:
		return "uid"
	case page == 2:
		return "static-lock"
	case page == 3:
		if g.Product == ProductUltralight {
			return "otp"
		}
		return "capability"
	case g.InUserMemory(page):
		return "user"
	case g.DynamicLockStart >= 0 && page == g.DynamicLockStart:
		return "dynamic-lock"
	case g.ConfigStart >= 0 && page >= g.ConfigStart && page <= g.ConfigEnd:
		return "config"
	case page == g.PasswordPage:
		return "password"
	case page == g.PackPage:
		return "pack"
	default:
		return "reserved"
	}
}

// DetectProduct maps an eight-byte GET_VERSION response to an exact NTAG21x
// product. Other NTAG families deliberately return false until their memory
// maps are validated.
func DetectProduct(version []byte) (Product, bool) {
	if len(version) != 8 || version[0] != 0x00 || version[1] != 0x04 ||
		version[2] != 0x04 || version[3] != 0x02 || version[4] != 0x01 ||
		version[5] != 0x00 || version[7] != 0x03 {
		return ProductUnknown, false
	}
	switch version[6] {
	case 0x0F:
		return ProductNTAG213, true
	case 0x11:
		return ProductNTAG215, true
	case 0x13:
		return ProductNTAG216, true
	default:
		return ProductUnknown, false
	}
}

// VersionForProduct returns the documented GET_VERSION response for the
// supported NTAG products. It is used by fixtures and the mock tag.
func VersionForProduct(product Product) []byte {
	size := byte(0)
	switch product {
	case ProductNTAG213:
		size = 0x0F
	case ProductNTAG215:
		size = 0x11
	case ProductNTAG216:
		size = 0x13
	default:
		return nil
	}
	return []byte{0x00, 0x04, 0x04, 0x02, 0x01, 0x00, size, 0x03}
}

func invalidGeometry(product Product) error {
	return fmt.Errorf("unsupported Type 2 product %q", product)
}
