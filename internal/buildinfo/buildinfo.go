// Package buildinfo exposes release metadata injected by the build pipeline.
package buildinfo

import (
	"runtime"
	"runtime/debug"
	"strings"
)

// These values are overridden with -ldflags for release builds. Keeping safe
// development defaults makes ordinary go test and native UI builds useful.
var (
	Version   = "dev"
	Commit    = "unknown"
	BuildDate = "unknown"
	Dirty     = "false"
)

// UIFramework names the desktop toolkit linked into this build.
const UIFramework = "MyGo"

type Info struct {
	Version      string `json:"version"`
	Commit       string `json:"commit"`
	BuildDate    string `json:"buildDate"`
	Dirty        bool   `json:"dirty"`
	GoVersion    string `json:"goVersion"`
	MyGoVersion  string `json:"mygoVersion,omitempty"`
	OS           string `json:"os"`
	Architecture string `json:"architecture"`
}

func Current() Info {
	version, commit, buildDate := Version, Commit, BuildDate
	mygoVersion := ""
	if details, ok := debug.ReadBuildInfo(); ok {
		if version == "dev" && details.Main.Version != "" && details.Main.Version != "(devel)" {
			version = details.Main.Version
		}
		for _, setting := range details.Settings {
			switch setting.Key {
			case "vcs.revision":
				if commit == "unknown" && setting.Value != "" {
					commit = setting.Value
				}
			case "vcs.time":
				if buildDate == "unknown" && setting.Value != "" {
					buildDate = setting.Value
				}
			}
		}
		for _, dependency := range details.Deps {
			if dependency.Path == "github.com/egoist/mygo" {
				mygoVersion = dependency.Version
				break
			}
		}
	}
	return Info{
		Version:      clean(version, "dev"),
		Commit:       clean(commit, "unknown"),
		BuildDate:    clean(buildDate, "unknown"),
		Dirty:        strings.EqualFold(strings.TrimSpace(Dirty), "true"),
		GoVersion:    runtime.Version(),
		MyGoVersion:  mygoVersion,
		OS:           runtime.GOOS,
		Architecture: runtime.GOARCH,
	}
}

func clean(value, fallback string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return fallback
	}
	return value
}
