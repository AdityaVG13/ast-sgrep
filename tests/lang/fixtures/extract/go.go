// Package fixtures mentions docOnlyGo and should not become code.
package fixtures

import "fmt"

type GoldenWidget struct {
	Name string
}

// MakeWidget docs mention docOnlyGo.
func MakeWidget(name string) GoldenWidget {
	return GoldenWidget{Name: name}
}

// Render docs mention docOnlyGo.
func (w GoldenWidget) Render() string {
	return formatWidget(w.Name)
}

func formatWidget(name string) string {
	return fmt.Sprintf("%s", name)
}
