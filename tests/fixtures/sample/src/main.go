package main

import "fmt"

func main() {
	processRequest("hello")
	authRefresh()
}

func processRequest(input string) string {
	validateInput(input)
	return fmt.Sprintf("processed: %s", input)
}

func validateInput(input string) {
	if input == "" {
		panic("empty input")
	}
}

func authRefresh() {
	token := fetchToken()
	storeToken(token)
}

func fetchToken() string {
	return "token"
}

func storeToken(token string) {
	_ = token
}
