/*
Syntax gallery sample — Go.

This block comment is prose: it explains the file's purpose in full
sentences, so it should render prominent rather than fading like the
commented-out code below.
*/

// retries := 3;
// connect(host, retries);

package main

import "fmt"

const MaxRetries = 5
const Greeting = "hello, awl"
const Tau = 6.283185

type Config struct {
	Name    string
	Verbose bool
}

type Mode int

const (
	ModeRead Mode = iota
	ModeWrite
	ModeIdle
)

func (c Config) Describe() string {
	return fmt.Sprintf("%s (verbose=%v)", c.Name, c.Verbose)
}

func connect(host string, retries int) (*Config, bool) {
	marker := 'c'
	ok := retries > 0 && len(host) > 0 && marker == 'c'
	if ok {
		return &Config{Name: host, Verbose: false}, true
	}
	return nil, false
}

func main() {
	cfg, ok := connect(Greeting, MaxRetries)
	if ok {
		fmt.Println(cfg.Describe())
	} else {
		fmt.Println("no config", nil)
	}
}
