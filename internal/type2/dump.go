package type2

import (
	"bytes"
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

const (
	// DumpFormat identifies a Type 2 page dump.
	DumpFormat = "nfc-foundry/type2"
	// DumpVersion is the current dump schema.
	DumpVersion = 1
	// PageSize is the fixed Type 2 page size.
	PageSize = 4
)

// Identity is the result of product detection.
type Identity struct {
	Product    Product
	Geometry   Geometry
	Version    []byte
	DetectedBy string
}

// Name returns the human-readable product name.
func (i Identity) Name() string { return ProductName(i.Product) }

// ReadOptions carry optional password authentication for protected tags.
type ReadOptions struct {
	Password []byte
	// Pack is optional. When set, PWD_AUTH must return exactly this value.
	Pack []byte
}

// PageRange is an inclusive page range.
type PageRange struct {
	Start int `json:"start"`
	End   int `json:"end"`
}

// CapabilityContainer is the page-3 NFC Forum Type 2 capability container.
type CapabilityContainer struct {
	Valid    bool   `json:"valid"`
	Raw      string `json:"raw"`
	Magic    byte   `json:"magic"`
	Version  byte   `json:"version"`
	Size     byte   `json:"size"`
	Access   byte   `json:"access"`
	Capacity int    `json:"capacity"`
}

// Writable reports whether the CC allows NDEF updates.
func (c CapabilityContainer) Writable() bool {
	return c.Valid && c.Access&0x0F == 0x00
}

// Document is the stable JSON dump for one Type 2 tag.
type Document struct {
	Format        string              `json:"format"`
	FormatVersion int                 `json:"formatVersion"`
	Product       Product             `json:"product"`
	ProductName   string              `json:"productName"`
	UID           string              `json:"uid"`
	ATQA          string              `json:"atqa"`
	SAK           string              `json:"sak"`
	Version       string              `json:"version"`
	TotalPages    int                 `json:"totalPages"`
	UserPages     PageRange           `json:"userPages"`
	Pages         []string            `json:"pages"`
	CC            CapabilityContainer `json:"cc"`
	Protection    Protection          `json:"protection"`
	NDEF          *NDEF               `json:"ndef,omitempty"`
	NDEFError     string              `json:"ndefError,omitempty"`
	ReadAt        string              `json:"readAt"`
}

// Identify detects the Type 2 product. GET_VERSION is authoritative for
// NTAG21x. A protocol failure from that command falls back to original
// MIFARE Ultralight only after a plausible BCC-valid page-0 read.
func Identify(ctx context.Context, transport FrameTransport) (identity Identity, err error) {
	session, openErr := OpenSession(ctx, transport)
	if openErr != nil {
		return Identity{}, openErr
	}
	defer func() {
		if closeErr := session.Close(context.WithoutCancel(ctx)); closeErr != nil {
			err = errors.Join(err, closeErr)
		}
	}()
	return identifySession(ctx, session)
}

func identifySession(ctx context.Context, session *Session) (Identity, error) {
	version, err := session.GetVersion(ctx)
	if err == nil {
		product, ok := DetectProduct(version)
		if !ok {
			return Identity{}, nfc.NewError("identify Type 2 tag", nfc.CodeUnsupported,
				fmt.Sprintf("GET_VERSION returned unsupported product data % X", version), nil)
		}
		geometry, _ := GeometryFor(product)
		return Identity{Product: product, Geometry: geometry, Version: version, DetectedBy: "get_version"}, nil
	}
	if !isProtocolFailure(err) {
		return Identity{}, err
	}
	first, readErr := session.ReadFourPages(ctx, 0)
	if readErr != nil {
		return Identity{}, nfc.NewError("identify Type 2 tag", nfc.CodeUnsupported,
			"GET_VERSION was not supported and the tag did not return a readable Type 2 page 0", readErr)
	}
	bcc, _ := BCCForUIDPage0(first)
	if first[3] != bcc {
		return Identity{}, nfc.NewError("identify Type 2 tag", nfc.CodeUnsupported,
			"GET_VERSION was not supported and page 0 does not contain a valid Type 2 BCC0", err)
	}
	geometry, _ := GeometryFor(ProductUltralight)
	return Identity{Product: ProductUltralight, Geometry: geometry, Version: nil, DetectedBy: "ultralight_nak"}, nil
}

func isProtocolFailure(err error) bool {
	if errors.Is(err, ErrNAK) {
		return true
	}
	code, ok := nfc.ErrorCodeOf(err)
	return ok && code == nfc.CodeUnsupported
}

// BCCForUIDPage0 returns BCC0 for the first three UID bytes stored in page 0.
func BCCForUIDPage0(page0 []byte) (byte, error) {
	if len(page0) < 3 {
		return 0, ErrMalformedTLV
	}
	return 0x88 ^ page0[0] ^ page0[1] ^ page0[2], nil
}

// BCCForUIDPage1 returns BCC1 for the final four bytes of a seven-byte UID.
func BCCForUIDPage1(page1 []byte) (byte, error) {
	if len(page1) < 4 {
		return 0, ErrMalformedTLV
	}
	return page1[0] ^ page1[1] ^ page1[2] ^ page1[3], nil
}

// ValidateCardPages proves that the dumped identity pages match the selected
// ISO/IEC 14443A card.
func ValidateCardPages(card nfc.CardInfo, pages [][4]byte) error {
	if len(pages) < 4 {
		return nfc.NewError("validate Type 2 dump", nfc.CodeInvalidArgument, "dump does not contain the identity pages", nil)
	}
	switch len(card.UID) {
	case 7:
		if !bytes.Equal(pages[0][0:3], card.UID[0:3]) || !bytes.Equal(pages[1][0:4], card.UID[3:7]) {
			return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "page 0/1 UID does not match the selected card", nil)
		}
		bcc0, _ := BCCForUIDPage0(pages[0][:])
		if pages[0][3] != bcc0 {
			return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "page 0 BCC0 is invalid", nil)
		}
		bcc1, _ := BCCForUIDPage1(pages[1][:])
		if pages[2][0] != bcc1 {
			return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "page 2 BCC1 is invalid", nil)
		}
	case 4:
		if !bytes.Equal(pages[0][0:3], card.UID[0:3]) || pages[1][0] != card.UID[3] {
			return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "page 0/1 UID does not match the selected card", nil)
		}
		bcc0, _ := BCCForUIDPage0(pages[0][:])
		if pages[0][3] != bcc0 {
			return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "page 0 BCC0 is invalid", nil)
		}
	default:
		return nfc.NewError("validate Type 2 dump", nfc.CodeUnsupported,
			fmt.Sprintf("%d-byte UIDs are not supported by the Type 2 module", len(card.UID)), nil)
	}
	return nil
}

// ParseCC validates page 3 against the product's user-memory size. A zero
// magic is a valid unformatted tag; other malformed contents return
// ErrMalformedCC with the raw page preserved.
func ParseCC(page [4]byte, geometry Geometry) (CapabilityContainer, error) {
	cc := CapabilityContainer{
		Raw: strings.ToUpper(hex.EncodeToString(page[:])), Magic: page[0],
		Version: page[1], Size: page[2], Access: page[3],
	}
	if page[0] == 0x00 {
		return cc, nil
	}
	if page[0] != 0xE1 {
		return cc, fmt.Errorf("%w: magic 0x%02X is not 0xE1", ErrMalformedCC, page[0])
	}
	if page[1]>>4 != 0x01 {
		return cc, fmt.Errorf("%w: unsupported version 0x%02X", ErrMalformedCC, page[1])
	}
	if page[3]&0xF0 != 0x00 || page[3]&0x0F != 0x00 && page[3]&0x0F != 0x0F {
		return cc, fmt.Errorf("%w: invalid access byte 0x%02X", ErrMalformedCC, page[3])
	}
	capacity := int(page[2]) * 8
	if capacity > geometry.UserBytes() {
		return cc, fmt.Errorf("%w: NDEF capacity %d exceeds %d user bytes", ErrMalformedCC, capacity, geometry.UserBytes())
	}
	cc.Valid = true
	cc.Capacity = capacity
	return cc, nil
}

// Read authenticates when required, reads the complete memory image, validates
// its identity and capability container, and parses NDEF without modifying the
// tag.
func Read(ctx context.Context, transport FrameTransport, card nfc.CardInfo, options ReadOptions) (document Document, err error) {
	session, openErr := OpenSession(ctx, transport)
	if openErr != nil {
		return Document{}, openErr
	}
	defer func() {
		if closeErr := session.Close(context.WithoutCancel(ctx)); closeErr != nil {
			err = errors.Join(err, closeErr)
		}
	}()
	return ReadSession(ctx, session, card, options)
}

// ReadSession reads a complete image inside an already active raw session.
func ReadSession(ctx context.Context, session *Session, card nfc.CardInfo, options ReadOptions) (Document, error) {
	identity, err := identifySession(ctx, session)
	if err != nil {
		return Document{}, err
	}
	if len(options.Password) != 0 {
		if len(options.Password) != 4 {
			return Document{}, nfc.NewError("password authentication", nfc.CodeInvalidArgument, "password must contain exactly 4 bytes", nil)
		}
		var password [4]byte
		copy(password[:], options.Password)
		pack, err := session.PwdAuth(ctx, password)
		if err != nil {
			return Document{}, err
		}
		if len(options.Pack) != 0 {
			if len(options.Pack) != 2 || !bytes.Equal(pack[:], options.Pack) {
				return Document{}, nfc.NewError("password authentication", nfc.CodeAuthenticationFailed,
					"PWD_AUTH returned a PACK that does not match the expected value", nil)
			}
		}
	}

	pages, err := readPages(ctx, session, identity.Geometry)
	if err != nil {
		return Document{}, err
	}
	if err := ValidateCardPages(card, pages); err != nil {
		return Document{}, err
	}
	cc, ccErr := ParseCC(pages[3], identity.Geometry)
	document := NewDocument(card, identity, pages, cc)
	if ccErr != nil {
		document.NDEFError = ccErr.Error()
		return document, nil
	}
	if !cc.Valid {
		document.NDEFError = "tag has no valid NDEF capability container"
		return document, nil
	}
	area, err := document.NDEFArea()
	if err != nil {
		document.NDEFError = err.Error()
		return document, nil
	}
	ndef, err := ParseNDEF(area)
	if err != nil {
		document.NDEFError = err.Error()
		return document, nil
	}
	document.NDEF = ndef
	return document, nil
}

func readPages(ctx context.Context, session *Session, geometry Geometry) ([][4]byte, error) {
	if !geometry.Valid() {
		return nil, invalidGeometry(geometry.Product)
	}
	first, err := session.ReadFourPages(ctx, 0)
	if err != nil {
		return nil, err
	}
	pages := make([][4]byte, geometry.TotalPages)
	for index := 0; index < 4; index++ {
		copy(pages[index][:], first[index*4:(index+1)*4])
	}
	for start := 4; start < geometry.TotalPages; start += 32 {
		end := min(start+31, geometry.TotalPages-1)
		chunk, err := session.FastRead(ctx, start, end)
		if err != nil {
			return nil, err
		}
		for offset := 0; offset < end-start+1; offset++ {
			copy(pages[start+offset][:], chunk[offset*4:(offset+1)*4])
		}
	}
	return pages, nil
}

// NewDocument builds a validated dump structure from a read result.
func NewDocument(card nfc.CardInfo, identity Identity, pages [][4]byte, cc CapabilityContainer) Document {
	document := Document{
		Format: DumpFormat, FormatVersion: DumpVersion,
		Product: identity.Product, ProductName: identity.Name(),
		UID:        strings.ToUpper(hex.EncodeToString(card.UID)),
		ATQA:       strings.ToUpper(hex.EncodeToString(card.ATQA[:])),
		SAK:        fmt.Sprintf("%02X", card.SAK),
		Version:    strings.ToUpper(hex.EncodeToString(identity.Version)),
		TotalPages: identity.Geometry.TotalPages,
		UserPages:  PageRange{Start: identity.Geometry.UserStart, End: identity.Geometry.UserEnd},
		Pages:      make([]string, len(pages)),
		CC:         cc, Protection: DecodeProtection(identity.Geometry, pages),
		ReadAt: time.Now().UTC().Format(time.RFC3339Nano),
	}
	for index, page := range pages {
		document.Pages[index] = strings.ToUpper(hex.EncodeToString(page[:]))
	}
	return document
}

// Geometry returns the validated geometry encoded by the document.
func (d Document) Geometry() (Geometry, error) {
	if err := d.Validate(); err != nil {
		return Geometry{}, err
	}
	geometry, _ := GeometryFor(d.Product)
	return geometry, nil
}

// Validate checks the dump schema and memory geometry without a card.
func (d Document) Validate() error {
	if d.Format != DumpFormat {
		return validationError("validate Type 2 dump", nil, fmt.Sprintf("unknown dump format %q", d.Format))
	}
	if d.FormatVersion != DumpVersion {
		return validationError("validate Type 2 dump", nil, fmt.Sprintf("unsupported dump version %d", d.FormatVersion))
	}
	geometry, ok := GeometryFor(d.Product)
	if !ok {
		return validationError("validate Type 2 dump", nil, fmt.Sprintf("unsupported product %q", d.Product))
	}
	if d.TotalPages != geometry.TotalPages || d.Pages == nil || len(d.Pages) != geometry.TotalPages {
		return validationError("validate Type 2 dump", nil, "page count does not match the product geometry")
	}
	if d.UserPages.Start != geometry.UserStart || d.UserPages.End != geometry.UserEnd {
		return validationError("validate Type 2 dump", nil, "user-page range does not match the product geometry")
	}
	uid, err := hex.DecodeString(d.UID)
	if err != nil || len(uid) != 4 && len(uid) != 7 {
		return validationError("validate Type 2 dump", nil, "UID must contain 8 or 14 hexadecimal digits")
	}
	for index, page := range d.Pages {
		decoded, err := hex.DecodeString(page)
		if err != nil || len(decoded) != PageSize {
			return validationError("validate Type 2 dump", nil, fmt.Sprintf("page %d is not a four-byte hexadecimal value", index))
		}
	}
	if d.CC.Valid && d.CC.Capacity > geometry.UserBytes() {
		return validationError("validate Type 2 dump", nil, "capability container exceeds user memory")
	}
	return nil
}

// ValidateForCard checks that a dump belongs to the selected product and UID.
func (d Document) ValidateForCard(card nfc.CardInfo) error {
	if err := d.Validate(); err != nil {
		return err
	}
	uid := strings.ToUpper(hex.EncodeToString(card.UID))
	if d.UID != uid {
		return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "dump UID does not match the selected card", nil)
	}
	if d.ATQA != "" && d.ATQA != strings.ToUpper(hex.EncodeToString(card.ATQA[:])) {
		return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "dump ATQA does not match the selected card", nil)
	}
	if d.SAK != "" && d.SAK != fmt.Sprintf("%02X", card.SAK) {
		return nfc.NewError("validate Type 2 dump", nfc.CodeCardChanged, "dump SAK does not match the selected card", nil)
	}
	geometry, _ := GeometryFor(d.Product)
	pages, err := d.PageBytes()
	if err != nil {
		return err
	}
	if err := ValidateCardPages(card, pages); err != nil {
		return err
	}
	if d.CC.Valid && d.CC.Capacity > geometry.UserBytes() {
		return validationError("validate Type 2 dump", nil, "capability container exceeds user memory")
	}
	return nil
}

// Page returns the four-byte page value.
func (d Document) Page(page int) ([4]byte, error) {
	var value [4]byte
	if page < 0 || page >= len(d.Pages) {
		return value, validationError("read dump page", nil, fmt.Sprintf("page %d is outside the dump", page))
	}
	decoded, err := hex.DecodeString(d.Pages[page])
	if err != nil || len(decoded) != PageSize {
		return value, validationError("read dump page", nil, fmt.Sprintf("page %d is not a four-byte hexadecimal value", page))
	}
	copy(value[:], decoded)
	return value, nil
}

// PageBytes decodes every page.
func (d Document) PageBytes() ([][4]byte, error) {
	pages := make([][4]byte, len(d.Pages))
	for index := range d.Pages {
		page, err := d.Page(index)
		if err != nil {
			return nil, err
		}
		pages[index] = page
	}
	return pages, nil
}

// UserMemory returns the complete user-memory bytes.
func (d Document) UserMemory() ([]byte, error) {
	geometry, err := d.Geometry()
	if err != nil {
		return nil, err
	}
	pages, err := d.PageBytes()
	if err != nil {
		return nil, err
	}
	memory := make([]byte, 0, geometry.UserBytes())
	for page := geometry.UserStart; page <= geometry.UserEnd; page++ {
		memory = append(memory, pages[page][:]...)
	}
	return memory, nil
}

// NDEFArea returns the capability-container-sized NDEF area from user memory.
func (d Document) NDEFArea() ([]byte, error) {
	if !d.CC.Valid {
		return nil, ErrMalformedCC
	}
	memory, err := d.UserMemory()
	if err != nil {
		return nil, err
	}
	if d.CC.Capacity < 0 || d.CC.Capacity > len(memory) {
		return nil, fmt.Errorf("%w: capacity %d exceeds %d user bytes", ErrCapacity, d.CC.Capacity, len(memory))
	}
	return memory[:d.CC.Capacity], nil
}
