package desktop

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"time"

	"github.com/YangtseSu/nfc-foundry/internal/diagnostic"
	runtimebundle "github.com/YangtseSu/nfc-foundry/internal/runtime"
)

var ErrSelfCheckFailed = errors.New("NFC Foundry self-check failed")

// Execute runs the release self-check without starting a graphical shell when
// --self-check is present. All other invocations start the native application.
func Execute(args []string, output io.Writer) error {
	selfCheck := false
	for _, argument := range args[1:] {
		if argument == "--self-check" || argument == "--self-check=json" {
			selfCheck = true
		}
	}
	if !selfCheck {
		return NewApplication().Run()
	}
	for _, argument := range args[1:] {
		if argument != "--self-check" && argument != "--self-check=json" && argument != "--json" {
			return fmt.Errorf("unknown self-check argument %q", argument)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	report := diagnostic.Run(ctx, runtimebundle.DefaultLocator(), "")
	encoder := json.NewEncoder(output)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(report); err != nil {
		return fmt.Errorf("write self-check report: %w", err)
	}
	if !report.Healthy {
		return ErrSelfCheckFailed
	}
	return nil
}
