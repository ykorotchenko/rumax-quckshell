package main

import (
	"context"
	"errors"
	"slices"
	"strings"
	"testing"
)

func TestParseReply(t *testing.T) {
	tests := []struct {
		name    string
		in      string
		want    string
		wantErr bool
	}{
		{"plain", "  {\"ok\":true}\n", `{"ok":true}`, false},
		{"string prefix", "string {\"ok\":true}\n", `{"ok":true}`, false},
		{"variant prefix", "variant       string {\"ok\":true}\n", `{"ok":true}`, false},
		{"quoted json", "string \"{\\\"ok\\\":true}\"\n", `{"ok":true}`, false},
		{"empty", "  \n", "", true},
		{"not json", "successful\n", "", true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := parseReply([]byte(tt.in))
			if tt.wantErr {
				if err == nil {
					t.Fatalf("parseReply(%q) succeeded, want error", tt.in)
				}
				return
			}
			if err != nil {
				t.Fatalf("parseReply(%q): %v", tt.in, err)
			}
			if string(got) != tt.want {
				t.Fatalf("parseReply(%q) = %q, want %q", tt.in, got, tt.want)
			}
		})
	}
}

func TestClientMethodArgs(t *testing.T) {
	tests := []struct {
		name     string
		call     func(*client) error
		wantArgs []string
	}{
		{
			name: "Ping",
			call: func(c *client) error { _, err := c.Ping(); return err },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.Ping",
			},
		},
		{
			name: "Status",
			call: func(c *client) error { _, err := c.Status(); return err },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.Status",
			},
		},
		{
			name: "GetChats",
			call: func(c *client) error { _, err := c.GetChats(); return err },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.GetChats",
			},
		},
		{
			name: "GetMessages",
			call: func(c *client) error { _, err := c.GetMessages(7, 50); return err },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.GetMessages", "int64:7", "int32:50",
			},
		},
		{
			name: "SendMessage",
			call: func(c *client) error { return c.SendMessage(7, "hello: world") },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.SendMessage", "int64:7", "string:hello: world",
			},
		},
		{
			name: "MarkRead",
			call: func(c *client) error { return c.MarkRead(7) },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.MarkRead", "int64:7",
			},
		},
		{
			name: "GetTyping",
			call: func(c *client) error { _, err := c.GetTyping(7); return err },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.GetTyping", "int64:7",
			},
		},
		{
			name: "SendTyping",
			call: func(c *client) error { return c.SendTyping(7) },
			wantArgs: []string{
				"--session", "--dest=org.maxplus.Client", "--print-reply=literal",
				"/org/maxplus/Client", "org.maxplus.Client.SendTyping", "int64:7",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var gotName string
			var gotArgs []string
			c := newClient(func(ctx context.Context, name string, args ...string) ([]byte, error) {
				gotName = name
				gotArgs = append([]string(nil), args...)
				if _, ok := ctx.Deadline(); !ok {
					t.Error("runner context has no timeout deadline")
				}
				return []byte("  string {\"ok\":true,\"state\":\"authenticated\"}\n"), nil
			})
			if err := tt.call(c); err != nil {
				t.Fatalf("call: %v", err)
			}
			if gotName != "dbus-send" {
				t.Errorf("command = %q, want dbus-send", gotName)
			}
			if !slices.Equal(gotArgs, tt.wantArgs) {
				t.Errorf("args = %q, want %q", gotArgs, tt.wantArgs)
			}
		})
	}
}

func TestClientReplies(t *testing.T) {
	reply := func(s string) *client {
		return newClient(func(context.Context, string, ...string) ([]byte, error) {
			return []byte(s), nil
		})
	}

	ping, err := reply(`{"ok":true,"service":"maxplus","version":"0.1.0"}`).Ping()
	if err != nil || ping.Service != "maxplus" || ping.Version != "0.1.0" {
		t.Fatalf("Ping() = %+v, %v", ping, err)
	}

	st, err := reply(`{"ok":true,"authenticated":true,"connected":false,"phone":null,"user_id":42,"state":"authenticated","unread":3}`).Status()
	if err != nil || !st.Authenticated || st.Connected || st.Phone != nil || st.UserID == nil || *st.UserID != 42 || st.Unread != 3 {
		t.Fatalf("Status() = %+v, %v", st, err)
	}

	chats, err := reply(`{"ok":true,"chats":[{"id":9,"title":"chat","last_message":"last","unread":3}]}`).GetChats()
	if err != nil || len(chats) != 1 || chats[0].ID != 9 || chats[0].Unread != 3 || chats[0].LastMessage != "last" {
		t.Fatalf("GetChats() = %+v, %v", chats, err)
	}

	msgs, err := reply(`{"ok":true,"messages":[{"id":1,"text":"hi","outgoing":true,"timestamp":123,"status":"sent"}]}`).GetMessages(9, 10)
	if err != nil || len(msgs) != 1 || !msgs[0].Outgoing || msgs[0].Text != "hi" || msgs[0].Timestamp != 123 {
		t.Fatalf("GetMessages() = %+v, %v", msgs, err)
	}

	typing, err := reply(`{"ok":true,"chat_id":7,"typing":true}`).GetTyping(7)
	if err != nil || !typing {
		t.Fatalf("GetTyping() = %v, %v", typing, err)
	}
	if err := reply(`{"ok":true}`).SendTyping(7); err != nil {
		t.Fatalf("SendTyping() = %v", err)
	}
}

func TestClientErrors(t *testing.T) {
	t.Run("daemon unavailable", func(t *testing.T) {
		c := newClient(func(context.Context, string, ...string) ([]byte, error) {
			return nil, errors.New("exit status 1")
		})
		_, err := c.Status()
		if err == nil || !strings.Contains(err.Error(), "daemon unavailable") {
			t.Fatalf("Status() error = %v", err)
		}
	})
	t.Run("daemon error", func(t *testing.T) {
		c := newClient(func(context.Context, string, ...string) ([]byte, error) {
			return []byte(`{"ok":false,"error":"bad"}`), nil
		})
		err := c.SendMessage(1, "hi")
		if err == nil || !strings.Contains(err.Error(), "bad") {
			t.Fatalf("SendMessage() error = %v", err)
		}
	})
	t.Run("invalid json", func(t *testing.T) {
		c := newClient(func(context.Context, string, ...string) ([]byte, error) {
			return []byte("login successful"), nil
		})
		if err := c.SendMessage(1, "hi"); err == nil || !strings.Contains(err.Error(), "bad daemon reply") {
			t.Fatalf("SendMessage() error = %v", err)
		}
	})
}
