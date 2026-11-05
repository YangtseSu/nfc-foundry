package app

import "errors"

var ErrDialogUnavailable = errors.New("file dialog is unavailable")

// FileDialogs is the platform file-picker contract. The desktop shell injects
// native dialogs; tests inject fixed paths.
type FileDialogs interface {
	OpenFile(title, pattern string) (string, error)
	SaveFile(title, defaultName, pattern string) (string, error)
}

type noFileDialogs struct{}

func (noFileDialogs) OpenFile(string, string) (string, error) { return "", ErrDialogUnavailable }

func (noFileDialogs) SaveFile(string, string, string) (string, error) {
	return "", ErrDialogUnavailable
}

// SetFileDialogs replaces the file picker used by the import/export flows.
func (s *Service) SetFileDialogs(dialogs FileDialogs) {
	if dialogs == nil {
		dialogs = noFileDialogs{}
	}
	s.mu.Lock()
	s.dialogs = dialogs
	s.mu.Unlock()
}
