package desktop

import "github.com/egoist/mygo"

type mygoFileDialogs struct{}

func (mygoFileDialogs) OpenFile(title, pattern string) (string, error) {
	paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
		Title:   title,
		Filters: []mygo.FileFilter{{Name: title, Extensions: patternExtensions(pattern)}},
	})
	if err != nil || len(paths) == 0 {
		return "", err
	}
	return paths[0], nil
}

func (mygoFileDialogs) SaveFile(title, defaultName, pattern string) (string, error) {
	return mygo.Dialog.Save(mygo.SaveDialogOptions{
		Title:       title,
		DefaultPath: defaultName,
		Filters:     []mygo.FileFilter{{Name: title, Extensions: patternExtensions(pattern)}},
	})
}
