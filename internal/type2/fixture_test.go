package type2_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/YangtseSu/nfc-foundry/internal/type2"
)

func TestSyntheticFixturesValidate(t *testing.T) {
	for _, test := range []struct {
		file     string
		product  type2.Product
		pages    int
		capacity int
	}{
		{"ntag213.json", type2.ProductNTAG213, 45, 144},
		{"ntag215.json", type2.ProductNTAG215, 135, 496},
		{"ntag216.json", type2.ProductNTAG216, 231, 872},
	} {
		data, err := os.ReadFile(filepath.Join("..", "..", "testdata", "type2", test.file))
		if err != nil {
			t.Fatal(err)
		}
		var document type2.Document
		if err := json.Unmarshal(data, &document); err != nil {
			t.Fatal(err)
		}
		if err := document.Validate(); err != nil {
			t.Fatalf("%s: %v", test.file, err)
		}
		if document.Product != test.product || len(document.Pages) != test.pages ||
			!document.CC.Valid || document.CC.Capacity != test.capacity {
			t.Fatalf("%s: product=%q pages=%d cc=%+v", test.file, document.Product, len(document.Pages), document.CC)
		}
		if document.NDEF != nil && document.NDEF.Present {
			t.Fatalf("%s: synthetic fixture unexpectedly has NDEF", test.file)
		}
	}
}
