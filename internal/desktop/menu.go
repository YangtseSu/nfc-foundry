package desktop

import (
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/i18n"
)

// menu builds the application menu in the current UI language. It is rebuilt
// when MyGo reports a language change.
func (s *shell) menu() *mygo.Menu {
	l := s.locales()
	var languages []*mygo.MenuItem
	for _, tag := range languageTags {
		tag := tag
		languages = append(languages, &mygo.MenuItem{
			Label:   i18n.Autonym(tag),
			Type:    mygo.MenuItemRadio,
			Checked: tag == s.locale,
			Click: func(*mygo.MenuItem, *mygo.Window) {
				s.setLocale(tag, true)
			},
		})
	}
	return mygo.NewMenu([]*mygo.MenuItem{
		{Role: mygo.RoleAppMenu},
		{Role: mygo.RoleEditMenu},
		{Role: mygo.RoleViewMenu},
		{Label: l.MenuDiagnostics(), Click: func(*mygo.MenuItem, *mygo.Window) {
			s.openDiagnostics()
		}},
		{Label: l.MenuLanguage(), Submenu: languages},
		{Role: mygo.RoleWindowMenu},
	})
}
