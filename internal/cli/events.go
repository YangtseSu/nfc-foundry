package cli

import (
	"context"
	"errors"
	"time"

	"github.com/YangtseSu/nfc-foundry/app"
)

// eventHub decouples the application service from the CLI consumer. Progress
// may be dropped when full; terminal task events always block so a command can
// never miss completion.
type eventHub struct {
	tasks  chan app.TaskEventDetailDTO
	cards  chan app.CardEventDTO
	states chan app.DeviceStateEventDTO
}

func newEventHub() *eventHub {
	return &eventHub{
		tasks:  make(chan app.TaskEventDetailDTO, 2048),
		cards:  make(chan app.CardEventDTO, 64),
		states: make(chan app.DeviceStateEventDTO, 64),
	}
}

func (h *eventHub) Emit(name string, payload any) {
	switch name {
	case app.TaskEventName:
		event, ok := payload.(app.TaskEventDetailDTO)
		if !ok {
			return
		}
		switch event.Type {
		case app.TaskEventCompleted, app.TaskEventCancelled, "failed":
			h.tasks <- event
		default:
			select {
			case h.tasks <- event:
			default:
			}
		}
	case app.CardEventName:
		event, ok := payload.(app.CardEventDTO)
		if !ok {
			return
		}
		select {
		case h.cards <- event:
		default:
		}
	case app.DeviceEventName:
		event, ok := payload.(app.DeviceStateEventDTO)
		if !ok {
			return
		}
		select {
		case h.states <- event:
		default:
		}
	}
}

var errTaskFinished = errors.New("task finished without a terminal event")

// waitTask consumes progress for one task and returns its terminal event. On
// context cancellation it requests cancellation and gives the service time to
// close the reader and terminate external processes before returning.
func (r *Runner) waitTask(ctx context.Context, task app.TaskDTO) (app.TaskEventDetailDTO, error) {
	r.lastProgress = app.TaskEventDetailDTO{}
	var last app.TaskEventDetailDTO
	for {
		select {
		case event := <-r.hub.tasks:
			if event.TaskID != task.ID {
				continue
			}
			r.out.progress(event)
			last = event
			if terminal, err := terminalTaskResult(event); terminal {
				return event, err
			}
			r.lastProgress = event
		case <-ctx.Done():
			_ = r.service.CancelTask(task.ID)
			timer := time.NewTimer(30 * time.Second)
			defer timer.Stop()
			for {
				select {
				case event := <-r.hub.tasks:
					if event.TaskID != task.ID {
						continue
					}
					r.out.progress(event)
					last = event
					if terminal, _ := terminalTaskResult(event); terminal {
						return event, ctx.Err()
					}
					r.lastProgress = event
				case <-timer.C:
					return last, ctx.Err()
				}
			}
		}
	}
}

func terminalTaskResult(event app.TaskEventDetailDTO) (bool, error) {
	switch event.Type {
	case app.TaskEventCompleted:
		return true, nil
	case app.TaskEventCancelled:
		return true, context.Canceled
	case "failed":
		if event.Message == "" {
			return true, errTaskFinished
		}
		return true, errors.New(event.Message)
	default:
		return false, nil
	}
}
