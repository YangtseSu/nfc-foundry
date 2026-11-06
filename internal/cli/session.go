package cli

import (
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

func (r *Runner) listDevices(_ context.Context) ([]app.DeviceDTO, error) {
	return r.service.RefreshDevices()
}

func (r *Runner) selectConnString(ctx context.Context) (string, error) {
	if r.global.device != "" {
		return r.global.device, nil
	}
	if value := strings.TrimSpace(os.Getenv("LIBNFC_DEVICE")); value != "" {
		return value, nil
	}
	devices, err := r.listDevices(ctx)
	if err != nil {
		return "", err
	}
	switch len(devices) {
	case 0:
		return "", nfc.NewError("select reader", nfc.CodeNotOpen, "no NFC reader was found; pass --device or set LIBNFC_DEVICE", nil)
	case 1:
		return devices[0].ConnString, nil
	default:
		names := make([]string, len(devices))
		for index, device := range devices {
			names[index] = device.ConnString
		}
		return "", usagef("multiple readers found; pass --device with one of: %s", strings.Join(names, ", "))
	}
}

// connect selects and opens exactly one reader, then records its device DTO for
// the JSON envelope.
func (r *Runner) connect(ctx context.Context) (app.DeviceDTO, error) {
	connString, err := r.selectConnString(ctx)
	if err != nil {
		return app.DeviceDTO{}, err
	}
	if err := r.service.ConnectDevice(connString); err != nil {
		r.device = connString
		return app.DeviceDTO{}, err
	}
	r.device = connString
	dashboard := r.service.Dashboard()
	if device, ok := findDevice(dashboard.Devices, connString); ok {
		return device, nil
	}
	return app.DeviceDTO{Name: connString, ConnString: connString, Available: true}, nil
}

func (r *Runner) waitForCard(ctx context.Context) (app.CardDTO, error) {
	cardCtx := ctx
	if _, hasDeadline := ctx.Deadline(); !hasDeadline {
		var cancel context.CancelFunc
		cardCtx, cancel = context.WithTimeout(ctx, 30*time.Second)
		defer cancel()
	}
	if _, err := r.service.WaitForCard(cardCtx); err != nil {
		return app.CardDTO{}, err
	}
	dashboard := r.service.Dashboard()
	r.card = &dashboard.Card
	return dashboard.Card, nil
}

func (r *Runner) refreshCard() *app.CardDTO {
	dashboard := r.service.Dashboard()
	if dashboard.Card.Present {
		r.card = &dashboard.Card
	}
	return r.card
}

func findDevice(devices []app.DeviceDTO, connString string) (app.DeviceDTO, bool) {
	for _, device := range devices {
		if device.ConnString == connString {
			return device, true
		}
	}
	return app.DeviceDTO{}, false
}

// importKeysAndScan imports a dictionary and verifies it against the current
// card. Calls without --keys return nil without touching the card.
func (r *Runner) importKeysAndScan(ctx context.Context, path string) (*keyscanResult, error) {
	if path == "" {
		return nil, nil
	}
	if _, err := r.service.ImportKeyDictionaryFile(path); err != nil {
		return nil, fmt.Errorf("import keys: %w", err)
	}
	task, err := r.service.StartKeyScan(true)
	if err != nil {
		return nil, err
	}
	if _, err := r.waitTask(ctx, task); err != nil {
		return nil, err
	}
	dashboard := r.service.Dashboard()
	summary := buildKeyscanResult(dashboard, r.global.showKeys)
	return &summary, nil
}

func (r *Runner) requireCard() error {
	if r.card == nil || !r.card.Present {
		return app.ErrCardRequired
	}
	return nil
}
