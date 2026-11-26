//go:build integration

package main

import "testing"

// Run with a live daemon:
//
//	maxplus-daemon serve &
//	go test -tags integration ./...
func TestLiveDaemon(t *testing.T) {
	c := newClient(nil)

	ping, err := c.Ping()
	if err != nil {
		t.Skipf("daemon is not available: %v", err)
	}
	if ping.Service != "maxplus" {
		t.Fatalf("unexpected service: %q", ping.Service)
	}

	status, err := c.Status()
	if err != nil {
		t.Fatalf("Status: %v", err)
	}
	if status.State == "" {
		t.Fatal("empty auth state")
	}

	if _, err := c.GetChats(); err != nil {
		t.Fatalf("GetChats: %v", err)
	}
	if _, err := c.GetMessages(42, 10); err != nil {
		t.Fatalf("GetMessages: %v", err)
	}
}
