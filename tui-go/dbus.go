package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

const (
	dbusService   = "org.maxplus.Client"
	dbusPath      = "/org/maxplus/Client"
	dbusInterface = "org.maxplus.Client"
	dbusTimeout   = 5 * time.Second
)

type commandRunner func(ctx context.Context, name string, args ...string) ([]byte, error)

func execCommand(ctx context.Context, name string, args ...string) ([]byte, error) {
	out, err := exec.CommandContext(ctx, name, args...).Output()
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && len(exitErr.Stderr) > 0 {
			return out, fmt.Errorf("%w: %s", err, strings.TrimSpace(string(exitErr.Stderr)))
		}
	}
	return out, err
}

type client struct {
	run     commandRunner
	timeout time.Duration
}

func newClient(run commandRunner) *client {
	if run == nil {
		run = execCommand
	}
	return &client{run: run, timeout: dbusTimeout}
}

func (c *client) call(method string, args ...string) ([]byte, error) {
	timeout := c.timeout
	if timeout <= 0 {
		timeout = dbusTimeout
	}
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()

	dbusArgs := []string{
		"--session",
		"--dest=" + dbusService,
		"--print-reply=literal",
		dbusPath,
		dbusInterface + "." + method,
	}
	dbusArgs = append(dbusArgs, args...)

	out, err := c.run(ctx, "dbus-send", dbusArgs...)
	if err != nil {
		return nil, fmt.Errorf("daemon unavailable (D-Bus %s): %w", method, err)
	}
	reply, err := parseReply(out)
	if err != nil {
		return nil, fmt.Errorf("bad daemon reply for %s: %w", method, err)
	}
	return reply, nil
}

func parseReply(out []byte) ([]byte, error) {
	s := strings.TrimSpace(string(out))
	for {
		fields := strings.Fields(s)
		if len(fields) == 0 {
			return nil, errors.New("empty reply")
		}
		if fields[0] != "variant" && fields[0] != "string" {
			break
		}
		s = strings.TrimSpace(strings.TrimPrefix(s, fields[0]))
	}
	if len(s) >= 2 && s[0] == '"' && s[len(s)-1] == '"' {
		if unquoted, err := strconv.Unquote(s); err == nil {
			s = strings.TrimSpace(unquoted)
		}
	}
	if !json.Valid([]byte(s)) {
		return nil, fmt.Errorf("expected JSON, got %q", s)
	}
	return []byte(s), nil
}

func decode[T any](c *client, method string, reply *T, args ...string) error {
	raw, err := c.call(method, args...)
	if err != nil {
		return err
	}
	if err := json.Unmarshal(raw, reply); err != nil {
		return fmt.Errorf("decode %s reply: %w", method, err)
	}
	return nil
}

func checkOK(ok bool, msg string) error {
	if ok {
		return nil
	}
	if msg == "" {
		msg = "unknown error"
	}
	return fmt.Errorf("daemon error: %s", msg)
}

type pingReply struct {
	OK      bool   `json:"ok"`
	Service string `json:"service"`
	Version string `json:"version"`
}

type statusReply struct {
	OK            bool    `json:"ok"`
	Authenticated bool    `json:"authenticated"`
	Connected     bool    `json:"connected"`
	Phone         *string `json:"phone"`
	UserID        *int64  `json:"user_id"`
	State         string  `json:"state"`
	Unread        int     `json:"unread"`
}

type okReply struct {
	OK    bool   `json:"ok"`
	Error string `json:"error"`
}

type typingReply struct {
	OK     bool   `json:"ok"`
	Typing bool   `json:"typing"`
	Error  string `json:"error"`
}

type chatsReply struct {
	OK    bool   `json:"ok"`
	Error string `json:"error"`
	Chats []chat `json:"chats"`
}

type foldersReply struct {
	OK      bool     `json:"ok"`
	Error   string   `json:"error"`
	Folders []folder `json:"folders"`
}

type folder struct {
	ID      string        `json:"id"`
	Title   string        `json:"title"`
	Include []interface{} `json:"include"`
	Filters []interface{} `json:"filters"`
}

type messagesReply struct {
	OK       bool      `json:"ok"`
	Error    string    `json:"error"`
	Messages []message `json:"messages"`
}

type chat struct {
	ID          int64  `json:"id"`
	Title       string `json:"title"`
	LastMessage string `json:"last_message"`
	Unread      int    `json:"unread"`
	Kind        string `json:"kind"`
	AvatarURL   string `json:"avatar_url"`
}

type message struct {
	ID        int64           `json:"id"`
	Text      string          `json:"text"`
	Outgoing  bool            `json:"outgoing"`
	Timestamp int64           `json:"timestamp"`
	Status    json.RawMessage `json:"status"`
	Sender    string          `json:"sender"`
	Attaches  json.RawMessage `json:"attaches"`
}

func (c *client) Ping() (pingReply, error) {
	var r pingReply
	if err := decode(c, "Ping", &r); err != nil {
		return r, err
	}
	if err := checkOK(r.OK, ""); err != nil {
		return r, err
	}
	return r, nil
}

func (c *client) Status() (statusReply, error) {
	var r statusReply
	if err := decode(c, "Status", &r); err != nil {
		return r, err
	}
	return r, nil
}

func (c *client) GetChats() ([]chat, error) {
	var r chatsReply
	if err := decode(c, "GetChats", &r); err != nil {
		return nil, err
	}
	if err := checkOK(r.OK, r.Error); err != nil {
		return nil, err
	}
	return r.Chats, nil
}

func (c *client) GetMessages(chatID int64, limit int) ([]message, error) {
	var r messagesReply
	if err := decode(c, "GetMessages", &r,
		fmt.Sprintf("int64:%d", chatID), fmt.Sprintf("int32:%d", limit)); err != nil {
		return nil, err
	}
	if err := checkOK(r.OK, r.Error); err != nil {
		return nil, err
	}
	return r.Messages, nil
}

func (c *client) SendMessage(chatID int64, text string) error {
	var r okReply
	if err := decode(c, "SendMessage", &r, fmt.Sprintf("int64:%d", chatID), "string:"+text); err != nil {
		return err
	}
	return checkOK(r.OK, r.Error)
}

func (c *client) MarkRead(chatID int64) error {
	var r okReply
	if err := decode(c, "MarkRead", &r, fmt.Sprintf("int64:%d", chatID)); err != nil {
		return err
	}
	return checkOK(r.OK, r.Error)
}

func (c *client) GetTyping(chatID int64) (bool, error) {
	var r typingReply
	if err := decode(c, "GetTyping", &r, fmt.Sprintf("int64:%d", chatID)); err != nil {
		return false, err
	}
	if err := checkOK(r.OK, r.Error); err != nil {
		return false, err
	}
	return r.Typing, nil
}

func (c *client) SendTyping(chatID int64) error {
	var r okReply
	if err := decode(c, "SendTyping", &r, fmt.Sprintf("int64:%d", chatID)); err != nil {
		return err
	}
	return checkOK(r.OK, r.Error)
}

func (c *client) GetFolders() ([]folder, error) {
	var r foldersReply
	if err := decode(c, "GetFolders", &r); err != nil {
		return nil, err
	}
	if err := checkOK(r.OK, r.Error); err != nil {
		return nil, err
	}
	return r.Folders, nil
}
