package main

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

type recordedCall struct {
	method string
	args   []string
}

func scriptedClient(replies map[string][]string) (*client, *[]recordedCall) {
	calls := new([]recordedCall)
	return newClient(func(_ context.Context, _ string, args ...string) ([]byte, error) {
		if len(args) < 5 {
			return nil, fmt.Errorf("too few dbus-send args: %v", args)
		}
		method := strings.TrimPrefix(args[4], dbusInterface+".")
		*calls = append(*calls, recordedCall{method: method, args: append([]string(nil), args...)})
		queue := replies[method]
		if len(queue) == 0 {
			return nil, fmt.Errorf("unexpected method %s", method)
		}
		replies[method] = queue[1:]
		return []byte(queue[0]), nil
	}), calls
}

func pressKey(t *testing.T, m model, msg tea.KeyMsg) (model, tea.Cmd) {
	t.Helper()
	next, cmd := m.Update(msg)
	got, ok := next.(model)
	if !ok {
		t.Fatalf("Update returned %T, want model", next)
	}
	return got, cmd
}

func typeText(t *testing.T, m model, text string) model {
	t.Helper()
	for _, r := range text {
		var cmd tea.Cmd
		m, cmd = pressKey(t, m, tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{r}})
		if cmd != nil {
			// ignore cmds from typing (should be none)
		}
	}
	return m
}

// execCmd runs a command and feeds the result back, but does NOT follow tick cmds.
func execCmd(t *testing.T, m model, cmd tea.Cmd) model {
	t.Helper()
	if cmd == nil {
		return m
	}
	msg := cmd()
	// skip ticks in tests
	switch msg.(type) {
	case tickMsg, msgTickMsg:
		return m
	}
	next, nextCmd := m.Update(msg)
	got, ok := next.(model)
	if !ok {
		t.Fatalf("Update returned %T, want model", next)
	}
	// follow non-tick follow-up cmds
	if nextCmd != nil {
		// batch?
		got = execBatch(t, got, nextCmd)
	}
	return got
}

func execBatch(t *testing.T, m model, cmd tea.Cmd) model {
	t.Helper()
	if cmd == nil {
		return m
	}
	msg := cmd()
	switch msg := msg.(type) {
	case tea.BatchMsg:
		for _, c := range msg {
			if c == nil {
				continue
			}
			m = execCmd(t, m, c)
		}
		return m
	case tickMsg, msgTickMsg:
		return m
	default:
		next, nextCmd := m.Update(msg)
		got, ok := next.(model)
		if !ok {
			t.Fatalf("Update returned %T, want model", next)
		}
		if nextCmd != nil {
			got = execBatch(t, got, nextCmd)
		}
		return got
	}
}

func initModel(t *testing.T, c *client) model {
	t.Helper()
	m := newModel(c)
	next, _ := m.Update(tea.WindowSizeMsg{Width: 120, Height: 40})
	m = next.(model)
	// Don't exec Init() directly - it includes tea.Tick cmds that sleep.
	m = execCmd(t, m, m.fetchStatus())
	m = execCmd(t, m, m.fetchChats())
	return m
}

const (
	authStatus   = `{"ok":true,"authenticated":true,"connected":true,"phone":"+7000000000","user_id":42,"state":"authenticated","unread":0}`
	noAuthStatus = `{"ok":true,"authenticated":false,"connected":false,"phone":null,"user_id":null,"state":"unauthenticated","unread":0}`
	chatsJSON    = `{"ok":true,"chats":[{"id":7,"title":"Alice","last_message":"hi","unread":2,"kind":"dm"},{"id":8,"title":"Group","last_message":"yo","unread":0,"kind":"group"}]}`
	msgsJSON     = `{"ok":true,"messages":[{"id":1,"text":"hello","outgoing":false,"timestamp":1710000000000,"status":"","sender":"Alice"},{"id":2,"text":"world","outgoing":true,"timestamp":1710000060000,"status":"read","sender":""}]}`
)

func TestInitLoadsStatusAndChats(t *testing.T) {
	c, calls := scriptedClient(map[string][]string{
		"Status":   {authStatus},
		"GetChats": {chatsJSON},
	})
	m := initModel(t, c)

	if !m.status.Authenticated || !m.status.Connected {
		t.Fatalf("status not loaded: %+v", m.status)
	}
	if len(m.chats) != 2 {
		t.Fatalf("chats = %d, want 2", len(m.chats))
	}
	if m.chats[0].Title != "Alice" {
		t.Fatalf("first chat = %+v", m.chats[0])
	}

	var methods []string
	for _, c := range *calls {
		methods = append(methods, c.method)
	}
	if !slices.Contains(methods, "Status") || !slices.Contains(methods, "GetChats") {
		t.Fatalf("methods = %v", methods)
	}
}

func TestOpenChatLoadsMessagesAndMarksRead(t *testing.T) {
	c, calls := scriptedClient(map[string][]string{
		"Status":      {authStatus},
		"GetChats":    {chatsJSON, chatsJSON},
		"GetMessages": {msgsJSON},
		"MarkRead":    {`{"ok":true}`},
	})
	m := initModel(t, c)

	// enter opens selected chat
	m, cmd := pressKey(t, m, tea.KeyMsg{Type: tea.KeyEnter})
	m = execBatch(t, m, cmd)

	if m.activeChat == nil || m.activeChat.ID != 7 {
		t.Fatalf("activeChat = %+v", m.activeChat)
	}
	if len(m.messages) != 2 {
		t.Fatalf("messages = %d, want 2", len(m.messages))
	}
	if m.messages[0].Text != "hello" || m.messages[1].Text != "world" {
		t.Fatalf("messages = %+v", m.messages)
	}
	if m.focus != focusInput {
		t.Fatalf("focus = %v, want focusInput", m.focus)
	}

	var methods []string
	for _, c := range *calls {
		methods = append(methods, c.method)
	}
	if !slices.Contains(methods, "GetMessages") {
		t.Fatalf("no GetMessages in %v", methods)
	}
	if !slices.Contains(methods, "MarkRead") {
		t.Fatalf("no MarkRead in %v", methods)
	}
}

func TestSendMessage(t *testing.T) {
	c, calls := scriptedClient(map[string][]string{
		"Status":      {authStatus},
		"GetChats":    {chatsJSON, chatsJSON, chatsJSON},
		"GetMessages": {msgsJSON, `{"ok":true,"messages":[{"id":1,"text":"hello","outgoing":false,"timestamp":1710000000000,"status":"","sender":"Alice"},{"id":2,"text":"world","outgoing":true,"timestamp":1710000060000,"status":"read","sender":""},{"id":3,"text":"test msg","outgoing":true,"timestamp":1710000120000,"status":"sent","sender":""}]}`},
		"MarkRead":    {`{"ok":true}`},
		"SendMessage": {`{"ok":true}`},
	})
	m := initModel(t, c)
	m, cmd := pressKey(t, m, tea.KeyMsg{Type: tea.KeyEnter}) // open chat
	m = execBatch(t, m, cmd)

	m = typeText(t, m, "test msg")
	m, cmd = pressKey(t, m, tea.KeyMsg{Type: tea.KeyEnter}) // send
	m = execBatch(t, m, cmd)

	if m.input != "" {
		t.Fatalf("input not cleared: %q", m.input)
	}
	if m.sending {
		t.Fatalf("still sending")
	}
	if m.inputErr != "" {
		t.Fatalf("inputErr = %q", m.inputErr)
	}

	found := false
	for _, c := range *calls {
		if c.method == "SendMessage" {
			found = true
			if !slices.Equal(c.args[5:], []string{"int64:7", "string:test msg"}) {
				t.Fatalf("SendMessage args = %v", c.args[5:])
			}
		}
	}
	if !found {
		t.Fatalf("SendMessage not called: %+v", *calls)
	}
}

func TestDaemonUnavailable(t *testing.T) {
	c := newClient(func(context.Context, string, ...string) ([]byte, error) {
		return nil, fmt.Errorf("dbus-send: not found")
	})
	m := newModel(c)
	next, _ := m.Update(tea.WindowSizeMsg{Width: 120, Height: 40})
	m = next.(model)
	m = execBatch(t, m, m.Init())

	if m.statusErr == "" {
		t.Fatalf("expected statusErr, got empty; status=%+v", m.status)
	}
	if !strings.Contains(m.statusErr, "daemon unavailable") {
		t.Fatalf("statusErr = %q", m.statusErr)
	}
}

func TestNavigation(t *testing.T) {
	c, _ := scriptedClient(map[string][]string{
		"Status":   {authStatus},
		"GetChats": {chatsJSON},
	})
	m := initModel(t, c)

	if m.chatCursor != 0 {
		t.Fatalf("cursor = %d", m.chatCursor)
	}
	m, _ = pressKey(t, m, tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'j'}})
	if m.chatCursor != 1 {
		t.Fatalf("cursor after j = %d", m.chatCursor)
	}
	m, _ = pressKey(t, m, tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'k'}})
	if m.chatCursor != 0 {
		t.Fatalf("cursor after k = %d", m.chatCursor)
	}
}
