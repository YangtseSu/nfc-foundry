package main

import (
	"fmt"
	"os"

	"github.com/YangtseSu/nfc-foundry/internal/desktop"
)

func main() {
	if err := desktop.Execute(os.Args, os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
