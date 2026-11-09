// Package keys owns MIFARE Classic key parsing, provenance and card-scoped
// verification state.
package keys

import (
	"bufio"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

type Source string

const (
	SourceBuiltin      Source = "builtin"
	SourceUserInput    Source = "user_input"
	SourceFileImport   Source = "file_import"
	SourceReadContext  Source = "read_context"
	SourceAttackEngine Source = "attack_engine"
)

func (s Source) Valid() bool {
	switch s {
	case SourceBuiltin, SourceUserInput, SourceFileImport, SourceReadContext, SourceAttackEngine:
		return true
	default:
		return false
	}
}

type Record struct {
	ID       string
	Value    nfc.Key
	Sources  []Source
	Name     string
	AddedAt  time.Time
	LastUsed time.Time
}

func (r Record) Hex() string {
	return strings.ToUpper(hex.EncodeToString(r.Value[:]))
}

func (r Record) HasSource(source Source) bool {
	for _, candidate := range r.Sources {
		if candidate == source {
			return true
		}
	}
	return false
}

type Verification struct {
	CardID     string
	Sector     int
	KeyType    nfc.KeyType
	KeyID      string
	VerifiedAt time.Time
}

type VerificationMergeStatus string

const (
	VerificationAdded    VerificationMergeStatus = "added"
	VerificationExisting VerificationMergeStatus = "existing"
	VerificationConflict VerificationMergeStatus = "conflict"
)

type VerificationMerge struct {
	Status        VerificationMergeStatus
	Record        Record
	ExistingKeyID string
}

// KeySetSource describes where a named key set came from.
type KeySetSource string

const (
	KeySetSourceManual   KeySetSource = "manual"
	KeySetSourceRecovery KeySetSource = "recovery"
)

func (s KeySetSource) Valid() bool {
	return s == KeySetSourceManual || s == KeySetSourceRecovery
}

// KeySet is a named, card-scoped group of key references.
type KeySet struct {
	ID        string
	Name      string
	CardID    string
	KeyIDs    []string
	CreatedAt time.Time
	Source    KeySetSource
}

// LoadReport describes state that LoadWithReport had to change to keep the
// store consistent.
type LoadReport struct {
	Version        int
	PrunedKeyIDs   []string
	DroppedKeySets []string
}

func (r LoadReport) Changed() bool {
	return len(r.PrunedKeyIDs) != 0 || len(r.DroppedKeySets) != 0
}

type ImportIssue struct {
	Line    int
	Message string
}

type ParseReport struct {
	Keys       []nfc.Key
	Valid      int
	Duplicates int
	Ignored    int
	Issues     []ImportIssue
}

func ParseDictionary(reader io.Reader) (ParseReport, error) {
	if reader == nil {
		return ParseReport{}, errors.New("dictionary reader is required")
	}
	var report ParseReport
	seen := make(map[nfc.Key]struct{})
	scanner := bufio.NewScanner(reader)
	for lineNumber := 1; scanner.Scan(); lineNumber++ {
		line := strings.TrimSpace(scanner.Text())
		if comment := strings.IndexByte(line, '#'); comment >= 0 {
			line = strings.TrimSpace(line[:comment])
		}
		if line == "" {
			report.Ignored++
			continue
		}
		compact := strings.Map(func(value rune) rune {
			if unicode.IsSpace(value) {
				return -1
			}
			return value
		}, line)
		if len(compact) != nfc.KeySize*2 {
			report.Issues = append(report.Issues, ImportIssue{Line: lineNumber, Message: "key must be 12 hexadecimal digits"})
			continue
		}
		decoded, err := hex.DecodeString(compact)
		if err != nil {
			report.Issues = append(report.Issues, ImportIssue{Line: lineNumber, Message: "key contains a non-hexadecimal character"})
			continue
		}
		key, _ := nfc.KeyFromBytes(decoded)
		if _, duplicate := seen[key]; duplicate {
			report.Duplicates++
			continue
		}
		seen[key] = struct{}{}
		report.Keys = append(report.Keys, key)
		report.Valid++
	}
	if err := scanner.Err(); err != nil {
		return ParseReport{}, err
	}
	return report, nil
}

func ParseKey(value string) (nfc.Key, error) {
	report, err := ParseDictionary(strings.NewReader(value))
	if err != nil {
		return nfc.Key{}, err
	}
	if len(report.Issues) != 0 || len(report.Keys) != 1 {
		return nfc.Key{}, errors.New("MIFARE Classic key must be 12 hexadecimal digits")
	}
	return report.Keys[0], nil
}

//go:embed data/common.dic
var builtinDictionary string

func Builtin() []nfc.Key {
	report, err := ParseDictionary(strings.NewReader(builtinDictionary))
	if err != nil {
		panic(err)
	}
	return append([]nfc.Key(nil), report.Keys...)
}

type Store struct {
	mu            sync.RWMutex
	records       map[string]*Record
	byValue       map[nfc.Key]string
	verifications map[string]Verification
	keySets       map[string]KeySet
	now           func() time.Time
}

func NewStore() *Store {
	store := &Store{
		records: make(map[string]*Record), byValue: make(map[nfc.Key]string),
		verifications: make(map[string]Verification), keySets: make(map[string]KeySet),
		now: func() time.Time { return time.Now().UTC() },
	}
	for _, key := range Builtin() {
		store.addLocked(key, SourceBuiltin)
	}
	return store
}

func keyID(value nfc.Key) string {
	digest := sha256.Sum256(value[:])
	return "key-" + hex.EncodeToString(digest[:8])
}

func keySetID(name, cardID string, createdAt time.Time) string {
	digest := sha256.Sum256([]byte(name + "\x00" + cardID + "\x00" + createdAt.UTC().Format(time.RFC3339Nano)))
	return "set-" + hex.EncodeToString(digest[:8])
}

// ValidateKeyName checks a display name without changing the store.
func ValidateKeyName(name string) error {
	_, err := validateKeyName(name)
	return err
}

// ValidateKeySetName checks a key-set name without changing the store.
func ValidateKeySetName(name string) error {
	_, err := validateKeySetName(name)
	return err
}

func validateKeyName(name string) (string, error) {
	name = strings.TrimSpace(name)
	if len([]rune(name)) > 64 {
		return "", errors.New("key name must be at most 64 characters")
	}
	for _, value := range name {
		if unicode.IsControl(value) {
			return "", errors.New("key name must not contain control characters")
		}
	}
	return name, nil
}

func validateKeySetName(name string) (string, error) {
	name = strings.TrimSpace(name)
	if name == "" {
		return "", errors.New("key set name is required")
	}
	if len([]rune(name)) > 80 {
		return "", errors.New("key set name must be at most 80 characters")
	}
	for _, value := range name {
		if unicode.IsControl(value) {
			return "", errors.New("key set name must not contain control characters")
		}
	}
	return name, nil
}

func cloneKeySet(value KeySet) KeySet {
	value.KeyIDs = append([]string(nil), value.KeyIDs...)
	return value
}

func verificationID(cardID string, sector int, keyType nfc.KeyType) string {
	return fmt.Sprintf("%s/%d/%d", cardID, sector, keyType)
}

// CardID is deliberately based on the complete selection identity used by
// NFCX. Verification remains card-scoped and is never inferred from a key's
// success on an unrelated card.
func CardID(card nfc.CardInfo) string {
	return fmt.Sprintf("%X/%X/%02X", card.UID, card.ATQA, card.SAK)
}

func (s *Store) Add(value nfc.Key, source Source) (Record, bool, error) {
	if s == nil || !source.Valid() {
		return Record{}, false, errors.New("valid key source is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.addLocked(value, source)
}

func (s *Store) addLocked(value nfc.Key, source Source) (Record, bool, error) {
	if id, exists := s.byValue[value]; exists {
		record := s.records[id]
		if !record.HasSource(source) {
			record.Sources = append(record.Sources, source)
			sort.SliceStable(record.Sources, func(i, j int) bool { return sourceRank(record.Sources[i]) < sourceRank(record.Sources[j]) })
		}
		return cloneRecord(*record), false, nil
	}
	now := time.Now().UTC()
	if s.now != nil {
		now = s.now()
	}
	record := &Record{ID: keyID(value), Value: value, Sources: []Source{source}, AddedAt: now}
	s.records[record.ID] = record
	s.byValue[value] = record.ID
	return cloneRecord(*record), true, nil
}

func (s *Store) AddHex(value string, source Source) (Record, bool, error) {
	key, err := ParseKey(value)
	if err != nil {
		return Record{}, false, err
	}
	return s.Add(key, source)
}

// RenameKey sets or clears the display name of an existing key record.
func (s *Store) RenameKey(id, name string) (Record, error) {
	validated, err := validateKeyName(name)
	if err != nil {
		return Record{}, err
	}
	if id == "" {
		return Record{}, errors.New("key ID is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	record, exists := s.records[id]
	if !exists {
		return Record{}, fmt.Errorf("key %q was not found", id)
	}
	record.Name = validated
	return cloneRecord(*record), nil
}

// KeySets returns every named key set in creation order.
func (s *Store) KeySets() []KeySet {
	s.mu.RLock()
	defer s.mu.RUnlock()
	result := make([]KeySet, 0, len(s.keySets))
	for _, set := range s.keySets {
		result = append(result, cloneKeySet(set))
	}
	sort.Slice(result, func(i, j int) bool {
		if !result[i].CreatedAt.Equal(result[j].CreatedAt) {
			return result[i].CreatedAt.Before(result[j].CreatedAt)
		}
		return result[i].ID < result[j].ID
	})
	return result
}

func (s *Store) KeySet(id string) (KeySet, bool) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	set, exists := s.keySets[id]
	if !exists {
		return KeySet{}, false
	}
	return cloneKeySet(set), true
}

// KeySetByName finds a set by trimmed name and exact card scope.
func (s *Store) KeySetByName(name, cardID string) (KeySet, bool) {
	validated, err := validateKeySetName(name)
	if err != nil {
		return KeySet{}, false
	}
	s.mu.RLock()
	defer s.mu.RUnlock()
	for _, set := range s.keySets {
		if set.Name == validated && set.CardID == cardID {
			return cloneKeySet(set), true
		}
	}
	return KeySet{}, false
}

// SetsContaining reports every set that references keyID.
func (s *Store) SetsContaining(keyID string) []KeySet {
	s.mu.RLock()
	defer s.mu.RUnlock()
	result := make([]KeySet, 0)
	for _, set := range s.keySets {
		for _, member := range set.KeyIDs {
			if member == keyID {
				result = append(result, cloneKeySet(set))
				break
			}
		}
	}
	sort.Slice(result, func(i, j int) bool { return result[i].ID < result[j].ID })
	return result
}

// CreateKeySet creates an empty named set, or returns the existing set with the
// same name and card scope. Membership changes use SetKeySetKeys or
// AddKeySetKeys.
func (s *Store) CreateKeySet(name, cardID string, source KeySetSource) (KeySet, error) {
	validated, err := validateKeySetName(name)
	if err != nil {
		return KeySet{}, err
	}
	if !source.Valid() {
		return KeySet{}, errors.New("valid key set source is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, set := range s.keySets {
		if set.Name == validated && set.CardID == cardID {
			return cloneKeySet(set), nil
		}
	}
	now := time.Now().UTC()
	if s.now != nil {
		now = s.now()
	}
	set := KeySet{ID: keySetID(validated, cardID, now), Name: validated, CardID: cardID, CreatedAt: now, Source: source}
	if _, exists := s.keySets[set.ID]; exists {
		return KeySet{}, errors.New("key set already exists")
	}
	s.keySets[set.ID] = set
	return cloneKeySet(set), nil
}

func (s *Store) SetKeySetKeys(id string, keyIDs []string) (KeySet, error) {
	return s.updateKeySetKeys(id, keyIDs, false)
}

func (s *Store) AddKeySetKeys(id string, keyIDs []string) (KeySet, error) {
	return s.updateKeySetKeys(id, keyIDs, true)
}

func (s *Store) updateKeySetKeys(id string, keyIDs []string, union bool) (KeySet, error) {
	if id == "" {
		return KeySet{}, errors.New("key set ID is required")
	}
	if len(keyIDs) == 0 {
		return KeySet{}, errors.New("at least one key is required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	set, exists := s.keySets[id]
	if !exists {
		return KeySet{}, fmt.Errorf("key set %q was not found", id)
	}
	members := make(map[string]struct{}, len(set.KeyIDs)+len(keyIDs))
	if union {
		for _, member := range set.KeyIDs {
			members[member] = struct{}{}
		}
	}
	for _, member := range keyIDs {
		if _, ok := s.records[member]; !ok {
			return KeySet{}, fmt.Errorf("key %q was not found", member)
		}
		members[member] = struct{}{}
	}
	set.KeyIDs = set.KeyIDs[:0]
	for member := range members {
		set.KeyIDs = append(set.KeyIDs, member)
	}
	sort.Strings(set.KeyIDs)
	s.keySets[id] = set
	return cloneKeySet(set), nil
}

func (s *Store) RenameKeySet(id, name string) (KeySet, error) {
	validated, err := validateKeySetName(name)
	if err != nil {
		return KeySet{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	set, exists := s.keySets[id]
	if !exists {
		return KeySet{}, fmt.Errorf("key set %q was not found", id)
	}
	for otherID, other := range s.keySets {
		if otherID != id && other.Name == validated && other.CardID == set.CardID {
			return KeySet{}, fmt.Errorf("a key set named %q already exists for this card", validated)
		}
	}
	set.Name = validated
	s.keySets[id] = set
	return cloneKeySet(set), nil
}

func (s *Store) DeleteKeySet(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, exists := s.keySets[id]; !exists {
		return fmt.Errorf("key set %q was not found", id)
	}
	delete(s.keySets, id)
	return nil
}

func (s *Store) Delete(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	record, exists := s.records[id]
	if !exists {
		return fmt.Errorf("key %q was not found", id)
	}
	kept := record.Sources[:0]
	for _, source := range record.Sources {
		if source == SourceBuiltin {
			kept = append(kept, source)
		}
	}
	if len(kept) != 0 {
		record.Sources = append([]Source(nil), kept...)
		record.Name = ""
		return nil
	}
	delete(s.records, id)
	delete(s.byValue, record.Value)
	for matchID, match := range s.verifications {
		if match.KeyID == id {
			delete(s.verifications, matchID)
		}
	}
	for setID, set := range s.keySets {
		kept := set.KeyIDs[:0]
		for _, member := range set.KeyIDs {
			if member != id {
				kept = append(kept, member)
			}
		}
		if len(kept) == 0 {
			delete(s.keySets, setID)
			continue
		}
		set.KeyIDs = kept
		s.keySets[setID] = set
	}
	return nil
}

func (s *Store) Entries() []Record {
	s.mu.RLock()
	defer s.mu.RUnlock()
	result := make([]Record, 0, len(s.records))
	for _, record := range s.records {
		result = append(result, cloneRecord(*record))
	}
	sort.Slice(result, func(i, j int) bool {
		left, right := recordRank(result[i]), recordRank(result[j])
		if left != right {
			return left < right
		}
		return result[i].ID < result[j].ID
	})
	return result
}

func cloneRecord(value Record) Record {
	value.Sources = append([]Source(nil), value.Sources...)
	return value
}

func sourceRank(source Source) int {
	switch source {
	case SourceReadContext:
		return 0
	case SourceUserInput:
		return 1
	case SourceFileImport:
		return 2
	case SourceAttackEngine:
		return 3
	case SourceBuiltin:
		return 4
	default:
		return 5
	}
}

func recordRank(record Record) int {
	rank := 99
	for _, source := range record.Sources {
		if candidate := sourceRank(source); candidate < rank {
			rank = candidate
		}
	}
	return rank
}

func (s *Store) Verify(cardID string, sector int, keyType nfc.KeyType, keyID string) error {
	if cardID == "" || sector < 0 || !keyType.Valid() {
		return errors.New("card, sector and key type are required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	record, exists := s.records[keyID]
	if !exists {
		return fmt.Errorf("key %q was not found", keyID)
	}
	now := time.Now().UTC()
	if s.now != nil {
		now = s.now()
	}
	record.LastUsed = now
	s.verifications[verificationID(cardID, sector, keyType)] = Verification{
		CardID: cardID, Sector: sector, KeyType: keyType, KeyID: keyID, VerifiedAt: now,
	}
	return nil
}

// Unverify removes a card-scoped verification that a fresh authentication
// attempt no longer supports, without deleting the key record itself.
func (s *Store) Unverify(cardID string, sector int, keyType nfc.KeyType) {
	if s == nil || cardID == "" || sector < 0 || !keyType.Valid() {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.verifications, verificationID(cardID, sector, keyType))
}

// MergeVerified atomically imports a freshly authenticated key without
// replacing a different verification already stored for the same card slot.
// A conflicting candidate is not added to the global key catalog.
func (s *Store) MergeVerified(cardID string, sector int, keyType nfc.KeyType, value nfc.Key, source Source) (VerificationMerge, error) {
	if s == nil || cardID == "" || sector < 0 || !keyType.Valid() || !source.Valid() {
		return VerificationMerge{}, errors.New("card, sector, key type and source are required")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	verificationKey := verificationID(cardID, sector, keyType)
	if existing, ok := s.verifications[verificationKey]; ok {
		record, present := s.records[existing.KeyID]
		if !present {
			return VerificationMerge{}, fmt.Errorf("verified key %q was not found", existing.KeyID)
		}
		if record.Value != value {
			return VerificationMerge{Status: VerificationConflict, ExistingKeyID: existing.KeyID}, nil
		}
		if !record.HasSource(source) {
			record.Sources = append(record.Sources, source)
			sort.SliceStable(record.Sources, func(i, j int) bool { return sourceRank(record.Sources[i]) < sourceRank(record.Sources[j]) })
		}
		now := time.Now().UTC()
		if s.now != nil {
			now = s.now()
		}
		record.LastUsed = now
		existing.VerifiedAt = now
		s.verifications[verificationKey] = existing
		return VerificationMerge{Status: VerificationExisting, Record: cloneRecord(*record), ExistingKeyID: existing.KeyID}, nil
	}
	record, _, err := s.addLocked(value, source)
	if err != nil {
		return VerificationMerge{}, err
	}
	now := time.Now().UTC()
	if s.now != nil {
		now = s.now()
	}
	stored := s.records[record.ID]
	stored.LastUsed = now
	s.verifications[verificationKey] = Verification{CardID: cardID, Sector: sector, KeyType: keyType, KeyID: record.ID, VerifiedAt: now}
	return VerificationMerge{Status: VerificationAdded, Record: cloneRecord(*stored)}, nil
}

func (s *Store) Verified(cardID string) []Verification {
	s.mu.RLock()
	defer s.mu.RUnlock()
	result := make([]Verification, 0)
	for _, match := range s.verifications {
		if match.CardID == cardID {
			result = append(result, match)
		}
	}
	sort.Slice(result, func(i, j int) bool {
		if result[i].Sector != result[j].Sector {
			return result[i].Sector < result[j].Sector
		}
		return result[i].KeyType < result[j].KeyType
	})
	return result
}

// Verifications returns every card-scoped verification for persistence.
func (s *Store) Verifications() []Verification {
	s.mu.RLock()
	defer s.mu.RUnlock()
	result := make([]Verification, 0, len(s.verifications))
	for _, match := range s.verifications {
		result = append(result, match)
	}
	sort.Slice(result, func(i, j int) bool {
		if result[i].CardID != result[j].CardID {
			return result[i].CardID < result[j].CardID
		}
		if result[i].Sector != result[j].Sector {
			return result[i].Sector < result[j].Sector
		}
		return result[i].KeyType < result[j].KeyType
	})
	return result
}

func keyTypeName(value nfc.KeyType) string {
	if value == nfc.KeyTypeB {
		return "B"
	}
	return "A"
}

// Candidates returns the deterministic priority order for one authentication
// slot: its exact prior match, other successes on this card, then user and
// imported entries, and finally the built-in dictionary.
func (s *Store) Candidates(cardID string, sector int, keyType nfc.KeyType) []Record {
	s.mu.RLock()
	defer s.mu.RUnlock()
	entries := make([]Record, 0, len(s.records))
	for _, record := range s.records {
		entries = append(entries, cloneRecord(*record))
	}
	exact := ""
	if match, ok := s.verifications[verificationID(cardID, sector, keyType)]; ok {
		exact = match.KeyID
	}
	succeeded := make(map[string]time.Time)
	for _, match := range s.verifications {
		if match.CardID == cardID {
			if previous, ok := succeeded[match.KeyID]; !ok || match.VerifiedAt.After(previous) {
				succeeded[match.KeyID] = match.VerifiedAt
			}
		}
	}
	sort.SliceStable(entries, func(i, j int) bool {
		left, right := candidateRank(entries[i], exact, succeeded), candidateRank(entries[j], exact, succeeded)
		if left != right {
			return left < right
		}
		leftTime, leftOK := succeeded[entries[i].ID]
		rightTime, rightOK := succeeded[entries[j].ID]
		if leftOK && rightOK && !leftTime.Equal(rightTime) {
			return leftTime.After(rightTime)
		}
		return entries[i].ID < entries[j].ID
	})
	return entries
}

func candidateRank(record Record, exact string, succeeded map[string]time.Time) int {
	if record.ID == exact {
		return -2
	}
	if _, ok := succeeded[record.ID]; ok {
		return -1
	}
	return recordRank(record)
}

func (s *Store) Record(id string) (Record, bool) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	record, exists := s.records[id]
	if !exists {
		return Record{}, false
	}
	return cloneRecord(*record), true
}

type persistedStore struct {
	Version       int                     `json:"version"`
	Keys          []persistedKey          `json:"keys"`
	Verifications []persistedVerification `json:"verifications,omitempty"`
	KeySets       []persistedKeySet       `json:"keySets,omitempty"`
}

type persistedKey struct {
	Value   string    `json:"value"`
	Sources []Source  `json:"sources"`
	Name    string    `json:"name,omitempty"`
	AddedAt time.Time `json:"addedAt,omitempty"`
}

type persistedVerification struct {
	CardID     string    `json:"cardId"`
	Sector     int       `json:"sector"`
	KeyType    string    `json:"keyType"`
	KeyID      string    `json:"keyId"`
	VerifiedAt time.Time `json:"verifiedAt"`
}

type persistedKeySet struct {
	ID        string       `json:"id"`
	Name      string       `json:"name"`
	CardID    string       `json:"cardId,omitempty"`
	KeyIDs    []string     `json:"keyIds"`
	CreatedAt time.Time    `json:"createdAt"`
	Source    KeySetSource `json:"source"`
}

// Load keeps the historical signature and discards the migration report.
func (s *Store) Load(path string) error {
	_, err := s.LoadWithReport(path)
	return err
}

// LoadWithReport loads version 1 or 2 and reports pruned key-set members.
func (s *Store) LoadWithReport(path string) (LoadReport, error) {
	var report LoadReport
	encoded, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return report, nil
	}
	if err != nil {
		return report, err
	}
	var stored persistedStore
	if err := json.Unmarshal(encoded, &stored); err != nil {
		return report, err
	}
	if stored.Version != 1 && stored.Version != 2 {
		return report, fmt.Errorf("unsupported key store version %d", stored.Version)
	}
	report.Version = stored.Version
	now := time.Now().UTC()
	if s.now != nil {
		now = s.now()
	}
	for _, item := range stored.Keys {
		key, err := ParseKey(item.Value)
		if err != nil {
			return report, err
		}
		name, err := validateKeyName(item.Name)
		if err != nil {
			return report, err
		}
		loaded := false
		for _, source := range item.Sources {
			if source == SourceBuiltin || !source.Valid() {
				continue
			}
			record, _, addErr := s.Add(key, source)
			if addErr != nil {
				return report, addErr
			}
			loaded = true
			if name != "" {
				if renameErr := s.renameLoadedKey(record.ID, name); renameErr != nil {
					return report, renameErr
				}
			}
			if !item.AddedAt.IsZero() {
				s.setAddedAt(record.ID, item.AddedAt)
			}
		}
		if !loaded {
			if name == "" {
				continue
			}
			record, exists := s.records[keyID(key)]
			if !exists || !record.HasSource(SourceBuiltin) {
				continue
			}
			if renameErr := s.renameLoadedKey(record.ID, name); renameErr != nil {
				return report, renameErr
			}
		}
	}
	for _, item := range stored.Verifications {
		keyType, err := parsePersistedKeyType(item.KeyType)
		if err != nil {
			return report, err
		}
		record, exists := s.records[item.KeyID]
		if !exists || item.CardID == "" || item.Sector < 0 {
			continue
		}
		verifiedAt := item.VerifiedAt
		if verifiedAt.IsZero() {
			verifiedAt = now
		}
		s.verifications[verificationID(item.CardID, item.Sector, keyType)] = Verification{
			CardID: item.CardID, Sector: item.Sector, KeyType: keyType, KeyID: record.ID, VerifiedAt: verifiedAt,
		}
	}
	seenIDs := make(map[string]struct{}, len(stored.KeySets))
	seenNames := make(map[string]struct{}, len(stored.KeySets))
	pruned := make(map[string]struct{})
	for _, item := range stored.KeySets {
		if item.ID == "" {
			return report, errors.New("key set ID is required")
		}
		if _, duplicate := seenIDs[item.ID]; duplicate {
			return report, fmt.Errorf("duplicate key set ID %q", item.ID)
		}
		seenIDs[item.ID] = struct{}{}
		name, err := validateKeySetName(item.Name)
		if err != nil {
			return report, err
		}
		nameKey := name + "\x00" + item.CardID
		if _, duplicate := seenNames[nameKey]; duplicate {
			return report, fmt.Errorf("duplicate key set name %q for card %q", name, item.CardID)
		}
		seenNames[nameKey] = struct{}{}
		source := item.Source
		if source == "" {
			source = KeySetSourceManual
		}
		if !source.Valid() {
			return report, fmt.Errorf("unsupported key set source %q", source)
		}
		members := make([]string, 0, len(item.KeyIDs))
		seenMembers := make(map[string]struct{}, len(item.KeyIDs))
		for _, member := range item.KeyIDs {
			if _, duplicate := seenMembers[member]; duplicate {
				continue
			}
			seenMembers[member] = struct{}{}
			if _, exists := s.records[member]; !exists {
				pruned[member] = struct{}{}
				continue
			}
			members = append(members, member)
		}
		if len(members) == 0 {
			report.DroppedKeySets = append(report.DroppedKeySets, item.ID)
			continue
		}
		createdAt := item.CreatedAt
		if createdAt.IsZero() {
			createdAt = now
		}
		s.keySets[item.ID] = KeySet{
			ID: item.ID, Name: name, CardID: item.CardID, KeyIDs: members, CreatedAt: createdAt, Source: source,
		}
	}
	for member := range pruned {
		report.PrunedKeyIDs = append(report.PrunedKeyIDs, member)
	}
	sort.Strings(report.PrunedKeyIDs)
	sort.Strings(report.DroppedKeySets)
	return report, nil
}

func (s *Store) renameLoadedKey(id, name string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	record, exists := s.records[id]
	if !exists {
		return fmt.Errorf("key %q was not found", id)
	}
	record.Name = name
	return nil
}

func (s *Store) setAddedAt(id string, addedAt time.Time) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if record, exists := s.records[id]; exists {
		record.AddedAt = addedAt
	}
}

// Reload atomically replaces the in-memory state from path. Services that
// already hold the store pointer observe the reloaded data.
func (s *Store) Reload(path string) (LoadReport, error) {
	if strings.TrimSpace(path) == "" {
		return LoadReport{}, errors.New("key store path is required")
	}
	fresh := NewStore()
	report, err := fresh.LoadWithReport(path)
	if err != nil {
		return report, err
	}
	fresh.mu.Lock()
	records, byValue := fresh.records, fresh.byValue
	verifications, keySets := fresh.verifications, fresh.keySets
	fresh.mu.Unlock()
	s.mu.Lock()
	s.records, s.byValue, s.verifications, s.keySets = records, byValue, verifications, keySets
	s.mu.Unlock()
	return report, nil
}

func parsePersistedKeyType(value string) (nfc.KeyType, error) {
	switch strings.ToUpper(strings.TrimSpace(value)) {
	case "A":
		return nfc.KeyTypeA, nil
	case "B":
		return nfc.KeyTypeB, nil
	default:
		return 0, fmt.Errorf("unsupported persisted key type %q", value)
	}
}

func (s *Store) Save(path string) error {
	stored := persistedStore{Version: 2}
	for _, record := range s.Entries() {
		item := persistedKey{
			Value: strings.ToUpper(hex.EncodeToString(record.Value[:])), Name: record.Name, AddedAt: record.AddedAt,
		}
		for _, source := range record.Sources {
			if source != SourceBuiltin {
				item.Sources = append(item.Sources, source)
			}
		}
		if len(item.Sources) != 0 || (item.Name != "" && record.HasSource(SourceBuiltin)) {
			stored.Keys = append(stored.Keys, item)
		}
	}
	for _, verification := range s.Verifications() {
		stored.Verifications = append(stored.Verifications, persistedVerification{
			CardID: verification.CardID, Sector: verification.Sector,
			KeyType: keyTypeName(verification.KeyType), KeyID: verification.KeyID, VerifiedAt: verification.VerifiedAt,
		})
	}
	for _, set := range s.KeySets() {
		if len(set.KeyIDs) == 0 {
			continue
		}
		stored.KeySets = append(stored.KeySets, persistedKeySet{
			ID: set.ID, Name: set.Name, CardID: set.CardID, KeyIDs: set.KeyIDs,
			CreatedAt: set.CreatedAt, Source: set.Source,
		})
	}
	encoded, err := json.MarshalIndent(stored, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	temporary, err := os.CreateTemp(filepath.Dir(path), ".nfcx-keys-*")
	if err != nil {
		return err
	}
	temporaryPath := temporary.Name()
	defer os.Remove(temporaryPath)
	if err := temporary.Chmod(0o600); err != nil {
		temporary.Close()
		return err
	}
	if _, err := temporary.Write(append(encoded, '\n')); err != nil {
		temporary.Close()
		return err
	}
	if err := temporary.Close(); err != nil {
		return err
	}
	return os.Rename(temporaryPath, path)
}

func (s *Store) Export(writer io.Writer) (int, error) {
	if writer == nil {
		return 0, errors.New("export writer is required")
	}
	count := 0
	for _, record := range s.Entries() {
		if _, err := fmt.Fprintln(writer, record.Hex()); err != nil {
			return count, err
		}
		count++
	}
	return count, nil
}
