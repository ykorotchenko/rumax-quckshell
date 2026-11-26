package main

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

const (
	messageLimit    = 60
	refreshEvery    = 4 * time.Second
	msgRefreshEvery = 3 * time.Second
)

type focusArea int

const (
	focusChats focusArea = iota
	focusInput
)

type searchMode int

const (
	searchNone searchMode = iota
	searchChats
	searchMessages
)

type tickMsg time.Time
type msgTickMsg time.Time

type statusMsg struct {
	status statusReply
	err    error
}

type chatsMsg struct {
	chats []chat
	err   error
}

type messagesMsg struct {
	chatID   int64
	messages []message
	err      error
}

type sentMsg struct {
	err error
}

type markedReadMsg struct {
	chatID int64
	err    error
}

type typingMsg struct {
	chatID int64
	typing bool
}

type sentTypingMsg struct {
	err error
}

type foldersMsg struct {
	folders []folder
	err     error
}

type model struct {
	client *client

	width  int
	height int

	focus     focusArea
	status    statusReply
	statusErr string

	chats      []chat
	chatCursor int
	chatOffset int

	folders   []folder
	folderIdx int

	activeChat *chat
	messages   []message
	msgOffset  int // scroll offset from bottom in lines, 0 = bottom
	msgErr     string

	input      string
	inputErr   string
	sending    bool
	loading    bool
	loadingMsg bool

	typing         bool
	lastTypingSent time.Time

	search           searchMode
	searchQuery      string
	msgSearchMatches []int // indices into m.messages
	msgSearchIdx     int
}

func newModel(c *client) model {
	return model{
		client:  c,
		focus:   focusChats,
		loading: true,
	}
}

func (m model) Init() tea.Cmd {
	return tea.Batch(m.fetchStatus(), m.fetchChats(), m.fetchFolders(), tick(), msgTick())
}

func tick() tea.Cmd {
	return tea.Tick(refreshEvery, func(t time.Time) tea.Msg { return tickMsg(t) })
}

func msgTick() tea.Cmd {
	return tea.Tick(msgRefreshEvery, func(t time.Time) tea.Msg { return msgTickMsg(t) })
}

func (m model) fetchStatus() tea.Cmd {
	return func() tea.Msg {
		s, err := m.client.Status()
		return statusMsg{status: s, err: err}
	}
}

func (m model) fetchChats() tea.Cmd {
	return func() tea.Msg {
		chats, err := m.client.GetChats()
		return chatsMsg{chats: chats, err: err}
	}
}

func (m model) fetchFolders() tea.Cmd {
	return func() tea.Msg {
		folders, err := m.client.GetFolders()
		return foldersMsg{folders: folders, err: err}
	}
}

func (m model) fetchMessages(chatID int64) tea.Cmd {
	return func() tea.Msg {
		msgs, err := m.client.GetMessages(chatID, messageLimit)
		return messagesMsg{chatID: chatID, messages: msgs, err: err}
	}
}

func (m model) sendMessage(chatID int64, text string) tea.Cmd {
	return func() tea.Msg {
		return sentMsg{err: m.client.SendMessage(chatID, text)}
	}
}

func (m model) markRead(chatID int64) tea.Cmd {
	return func() tea.Msg {
		return markedReadMsg{chatID: chatID, err: m.client.MarkRead(chatID)}
	}
}

func (m model) fetchTyping(chatID int64) tea.Cmd {
	return func() tea.Msg {
		typing, err := m.client.GetTyping(chatID)
		if err != nil {
			return typingMsg{chatID: chatID, typing: false}
		}
		return typingMsg{chatID: chatID, typing: typing}
	}
}

func (m model) sendTyping(chatID int64) tea.Cmd {
	return func() tea.Msg {
		return sentTypingMsg{err: m.client.SendTyping(chatID)}
	}
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.clampScroll()
		return m, nil

	case tickMsg:
		cmds := []tea.Cmd{tick(), m.fetchStatus(), m.fetchChats(), m.fetchFolders()}
		return m, tea.Batch(cmds...)

	case msgTickMsg:
		cmds := []tea.Cmd{msgTick()}
		if m.activeChat != nil {
			if !m.loadingMsg {
				cmds = append(cmds, m.fetchMessages(m.activeChat.ID))
			}
			cmds = append(cmds, m.fetchTyping(m.activeChat.ID))
		}
		return m, tea.Batch(cmds...)

	case statusMsg:
		m.loading = false
		if msg.err != nil {
			m.statusErr = msg.err.Error()
		} else {
			m.status = msg.status
			m.statusErr = ""
		}
		return m, nil

	case chatsMsg:
		if msg.err != nil {
			if m.statusErr == "" {
				m.statusErr = msg.err.Error()
			}
			return m, nil
		}
		m.chats = msg.chats
		// keep cursor in bounds
		display := m.displayChats()
		if len(display) == 0 {
			m.chatCursor = 0
			m.chatOffset = 0
		} else {
			if m.chatCursor >= len(display) {
				m.chatCursor = len(display) - 1
			}
			if m.chatCursor < 0 {
				m.chatCursor = 0
			}
		}
		// update active chat metadata
		if m.activeChat != nil {
			for i := range m.chats {
				if m.chats[i].ID == m.activeChat.ID {
					c := m.chats[i]
					m.activeChat = &c
					break
				}
			}
		}
		m.clampChatScroll()
		return m, nil

	case foldersMsg:
		if msg.err == nil {
			m.folders = msg.folders
			if m.folderIdx >= len(m.folders) {
				m.folderIdx = 0
			}
			m.clampChatScroll()
		}
		return m, nil

	case messagesMsg:
		m.loadingMsg = false
		if m.activeChat == nil || msg.chatID != m.activeChat.ID {
			return m, nil
		}
		if msg.err != nil {
			m.msgErr = msg.err.Error()
			return m, nil
		}
		m.msgErr = ""
		msgs := append([]message(nil), msg.messages...)
		sort.Slice(msgs, func(i, j int) bool {
			if msgs[i].Timestamp == msgs[j].Timestamp {
				return msgs[i].ID < msgs[j].ID
			}
			return msgs[i].Timestamp < msgs[j].Timestamp
		})
		// preserve scroll position if user scrolled up and message set didn't change at bottom
		oldLen := len(m.messages)
		oldLastID := int64(0)
		if oldLen > 0 {
			oldLastID = m.messages[oldLen-1].ID
		}
		newLastID := int64(0)
		if len(msgs) > 0 {
			newLastID = msgs[len(msgs)-1].ID
		}
		m.messages = msgs
		if m.msgOffset > 0 && newLastID == oldLastID && len(msgs) >= oldLen {
			// keep user scroll position
		} else if m.msgOffset == 0 {
			// stick to bottom
		} else if newLastID != oldLastID {
			// new messages arrived while scrolled up - keep offset roughly
			m.msgOffset += max(0, len(msgs)-oldLen)
		}
		m.clampScroll()
		// invalidate stale search matches on message refresh
		if len(m.msgSearchMatches) > 0 {
			m.msgSearchMatches = nil
			m.msgSearchIdx = 0
		}
		return m, nil

	case sentMsg:
		m.sending = false
		if msg.err != nil {
			m.inputErr = msg.err.Error()
			return m, nil
		}
		m.input = ""
		m.inputErr = ""
		m.msgOffset = 0
		if m.activeChat != nil {
			m.loadingMsg = true
			return m, tea.Batch(m.fetchMessages(m.activeChat.ID), m.fetchChats())
		}
		return m, m.fetchChats()

	case markedReadMsg:
		// best-effort; refresh chats to update unread badge
		if msg.err == nil {
			return m, m.fetchChats()
		}
		return m, nil

	case typingMsg:
		if m.activeChat != nil && msg.chatID == m.activeChat.ID {
			m.typing = msg.typing
		}
		return m, nil

	case sentTypingMsg:
		return m, nil

	case tea.KeyMsg:
		return m.handleKey(msg)
	}
	return m, nil
}

func (m model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "ctrl+c":
		return m, tea.Quit
	}

	if m.search != searchNone {
		return m.handleSearchKey(msg)
	}

	// Global keys
	switch msg.String() {
	case "tab":
		if m.focus == focusChats {
			if m.activeChat != nil {
				m.focus = focusInput
			}
		} else {
			m.focus = focusChats
		}
		return m, nil
	case "ctrl+r":
		m.loading = true
		cmds := []tea.Cmd{m.fetchStatus(), m.fetchChats()}
		if m.activeChat != nil {
			m.loadingMsg = true
			cmds = append(cmds, m.fetchMessages(m.activeChat.ID))
		}
		return m, tea.Batch(cmds...)
	}

	if m.focus == focusInput {
		return m.handleInputKey(msg)
	}
	return m.handleChatListKey(msg)
}

func (m *model) maybeSendTyping() tea.Cmd {
	if m.activeChat == nil || !m.status.Authenticated || !m.status.Connected {
		return nil
	}
	if time.Since(m.lastTypingSent) < 4*time.Second {
		return nil
	}
	m.lastTypingSent = time.Now()
	return m.sendTyping(m.activeChat.ID)
}

func (m model) handleSearchKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "esc":
		wasMsg := m.search == searchMessages
		wasChat := m.search == searchChats
		m.search = searchNone
		m.searchQuery = ""
		if wasMsg {
			m.msgSearchMatches = nil
			m.msgSearchIdx = 0
		}
		if wasChat {
			m.clampChatScroll()
		}
		return m, nil
	case "enter":
		if m.search == searchChats {
			chats := m.filteredChats()
			if len(chats) > 0 {
				idx := m.chatCursor
				if idx < 0 || idx >= len(chats) {
					idx = 0
				}
				c := chats[idx]
				// restore cursor to full list index
				for i, fc := range m.chats {
					if fc.ID == c.ID {
						m.chatCursor = i
						break
					}
				}
				m.search = searchNone
				m.searchQuery = ""
				m.activeChat = &c
				m.messages = nil
				m.msgErr = ""
				m.msgOffset = 0
				m.loadingMsg = true
				m.typing = false
				m.focus = focusInput
				m.clampChatScroll()
				return m, tea.Batch(m.fetchMessages(c.ID), m.markRead(c.ID), m.fetchTyping(c.ID))
			}
			m.search = searchNone
			m.searchQuery = ""
			return m, nil
		}
		// searchMessages
		m.performMessageSearch()
		m.search = searchNone
		if len(m.msgSearchMatches) > 0 {
			m.msgSearchIdx = 0
			m.scrollToMsgMatch()
		}
		return m, nil
	case "backspace":
		if len(m.searchQuery) > 0 {
			r := []rune(m.searchQuery)
			m.searchQuery = string(r[:len(r)-1])
			if m.search == searchChats {
				m.clampChatCursorToFiltered()
			}
		}
		return m, nil
	case "ctrl+u":
		m.searchQuery = ""
		if m.search == searchChats {
			m.clampChatCursorToFiltered()
		}
		return m, nil
	case "up", "ctrl+p":
		if m.search == searchChats {
			if m.chatCursor > 0 {
				m.chatCursor--
				m.clampChatScroll()
			}
		}
		return m, nil
	case "down", "ctrl+n":
		if m.search == searchChats {
			chats := m.filteredChats()
			if m.chatCursor < len(chats)-1 {
				m.chatCursor++
				m.clampChatScroll()
			}
		}
		return m, nil
	}

	if msg.Type == tea.KeyRunes {
		m.searchQuery += string(msg.Runes)
		if m.search == searchChats {
			m.clampChatCursorToFiltered()
		}
		return m, nil
	}
	if msg.Type == tea.KeySpace {
		m.searchQuery += " "
		if m.search == searchChats {
			m.clampChatCursorToFiltered()
		}
		return m, nil
	}
	return m, nil
}

func folderIncludesChat(f folder, chatID int64) bool {
	for _, v := range f.Include {
		switch n := v.(type) {
		case float64:
			if int64(n) == chatID {
				return true
			}
		case int64:
			if n == chatID {
				return true
			}
		case int:
			if int64(n) == chatID {
				return true
			}
		case json.Number:
			if id, err := n.Int64(); err == nil && id == chatID {
				return true
			}
		}
	}
	return false
}

func folderHasFilter(f folder, want int) bool {
	for _, v := range f.Filters {
		switch n := v.(type) {
		case float64:
			if int(n) == want {
				return true
			}
		case int:
			if n == want {
				return true
			}
		}
	}
	return false
}

func (m model) folderChats() []chat {
	if len(m.folders) == 0 || m.folderIdx < 0 || m.folderIdx >= len(m.folders) {
		return m.chats
	}
	f := m.folders[m.folderIdx]
	// "All" folder: empty include and empty filters
	if len(f.Include) == 0 && len(f.Filters) == 0 {
		return m.chats
	}
	if len(f.Include) > 0 {
		var out []chat
		for _, c := range m.chats {
			if folderIncludesChat(f, c.ID) {
				out = append(out, c)
			}
		}
		return out
	}
	// filter-only folder (e.g. channels filters:[2]) - best effort
	if folderHasFilter(f, 2) {
		var out []chat
		for _, c := range m.chats {
			k := strings.ToLower(c.Kind)
			if k == "channel" || k == "channels" {
				out = append(out, c)
			}
		}
		if len(out) > 0 {
			return out
		}
	}
	// fallback: show all rather than hiding everything
	return m.chats
}

func (m model) displayChats() []chat {
	base := m.folderChats()
	if m.search != searchChats || m.searchQuery == "" {
		return base
	}
	q := strings.ToLower(m.searchQuery)
	var out []chat
	for _, c := range base {
		if strings.Contains(strings.ToLower(c.Title), q) ||
			strings.Contains(strings.ToLower(c.LastMessage), q) {
			out = append(out, c)
		}
	}
	return out
}

func (m model) filteredChats() []chat {
	return m.displayChats()
}

func (m *model) clampChatCursorToFiltered() {
	m.clampChatScroll()
}

func (m *model) performMessageSearch() {
	m.msgSearchMatches = nil
	m.msgSearchIdx = 0
	q := strings.ToLower(strings.TrimSpace(m.searchQuery))
	if q == "" {
		return
	}
	for i, msg := range m.messages {
		if strings.Contains(strings.ToLower(msg.Text), q) ||
			strings.Contains(strings.ToLower(msg.Sender), q) {
			m.msgSearchMatches = append(m.msgSearchMatches, i)
		}
	}
}

func (m *model) scrollToMsgMatch() {
	if len(m.msgSearchMatches) == 0 || m.msgSearchIdx < 0 || m.msgSearchIdx >= len(m.msgSearchMatches) {
		return
	}
	// approximate: set msgOffset so target message is visible
	target := m.msgSearchMatches[m.msgSearchIdx]
	// msgOffset is lines from bottom; easier to set offset so target is near middle
	total := len(m.messages)
	if total == 0 {
		return
	}
	// position from bottom
	fromBottom := total - 1 - target
	h := m.messageViewHeight()
	m.msgOffset = fromBottom - h/2
	if m.msgOffset < 0 {
		m.msgOffset = 0
	}
	m.clampScroll()
}

func (m model) handleChatListKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q":
		return m, tea.Quit
	case "up", "k":
		if m.chatCursor > 0 {
			m.chatCursor--
			m.clampChatScroll()
		}
		return m, nil
	case "down", "j":
		chats := m.displayChats()
		if m.chatCursor < len(chats)-1 {
			m.chatCursor++
			m.clampChatScroll()
		}
		return m, nil
	case "enter":
		chats := m.displayChats()
		if len(chats) == 0 || m.chatCursor < 0 || m.chatCursor >= len(chats) {
			return m, nil
		}
		c := chats[m.chatCursor]
		m.activeChat = &c
		m.messages = nil
		m.msgErr = ""
		m.msgOffset = 0
		m.loadingMsg = true
		m.typing = false
		m.focus = focusInput
		return m, tea.Batch(m.fetchMessages(c.ID), m.markRead(c.ID), m.fetchTyping(c.ID))
	case "g":
		m.chatCursor = 0
		m.clampChatScroll()
		return m, nil
	case "G":
		chats := m.displayChats()
		if len(chats) > 0 {
			m.chatCursor = len(chats) - 1
			m.clampChatScroll()
		}
		return m, nil
	case "left", "h", "[":
		if len(m.folders) > 0 {
			m.folderIdx--
			if m.folderIdx < 0 {
				m.folderIdx = len(m.folders) - 1
			}
			m.chatCursor = 0
			m.chatOffset = 0
			m.clampChatScroll()
		}
		return m, nil
	case "right", "l", "]":
		if len(m.folders) > 0 {
			m.folderIdx = (m.folderIdx + 1) % len(m.folders)
			m.chatCursor = 0
			m.chatOffset = 0
			m.clampChatScroll()
		}
		return m, nil
	case "/":
		m.search = searchChats
		m.searchQuery = ""
		m.chatCursor = 0
		m.chatOffset = 0
		return m, nil
	case "n":
		if len(m.msgSearchMatches) > 0 {
			m.msgSearchIdx = (m.msgSearchIdx + 1) % len(m.msgSearchMatches)
			m.scrollToMsgMatch()
		}
		return m, nil
	case "N":
		if len(m.msgSearchMatches) > 0 {
			m.msgSearchIdx--
			if m.msgSearchIdx < 0 {
				m.msgSearchIdx = len(m.msgSearchMatches) - 1
			}
			m.scrollToMsgMatch()
		}
		return m, nil
	}
	return m, nil
}

func (m model) handleInputKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	if msg.String() == "/" && m.input == "" && m.activeChat != nil {
		m.search = searchMessages
		m.searchQuery = ""
		m.msgSearchMatches = nil
		m.msgSearchIdx = 0
		return m, nil
	}
	switch msg.String() {
	case "esc":
		m.focus = focusChats
		return m, nil
	case "enter":
		text := strings.TrimSpace(m.input)
		if text == "" || m.activeChat == nil || m.sending {
			return m, nil
		}
		if !m.status.Authenticated || !m.status.Connected {
			m.inputErr = "не подключено к Max"
			return m, nil
		}
		m.sending = true
		m.inputErr = ""
		return m, m.sendMessage(m.activeChat.ID, text)
	case "backspace":
		if len(m.input) > 0 {
			r := []rune(m.input)
			m.input = string(r[:len(r)-1])
		}
		return m, nil
	case "ctrl+u":
		m.input = ""
		return m, nil
	case "pgup":
		m.msgOffset += 5
		m.clampScroll()
		return m, nil
	case "pgdown":
		m.msgOffset -= 5
		if m.msgOffset < 0 {
			m.msgOffset = 0
		}
		return m, nil
	case "up":
		if m.msgOffset == 0 {
			// if at bottom, start scrolling up
		}
		m.msgOffset++
		m.clampScroll()
		return m, nil
	case "down":
		if m.msgOffset > 0 {
			m.msgOffset--
		}
		return m, nil
	}

	// text input
	if msg.Type == tea.KeyRunes {
		m.input += string(msg.Runes)
		return m, m.maybeSendTyping()
	}
	if msg.Type == tea.KeySpace {
		m.input += " "
		return m, m.maybeSendTyping()
	}
	return m, nil
}

func (m *model) clampChatScroll() {
	h := m.chatListHeight()
	headerLines := 1
	if len(m.folders) > 0 {
		headerLines++
	}
	listLen := len(m.displayChats())
	if m.search == searchChats {
		headerLines++ // search line
	}
	visibleItems := (h - headerLines) / 2
	if visibleItems < 1 {
		visibleItems = 1
	}
	if m.chatCursor >= listLen {
		m.chatCursor = listLen - 1
	}
	if m.chatCursor < 0 {
		m.chatCursor = 0
	}
	if m.chatCursor < m.chatOffset {
		m.chatOffset = m.chatCursor
	}
	if m.chatCursor >= m.chatOffset+visibleItems {
		m.chatOffset = m.chatCursor - visibleItems + 1
	}
	if m.chatOffset < 0 {
		m.chatOffset = 0
	}
}

func (m *model) clampScroll() {
	// rough clamp: msgOffset can't exceed number of messages
	maxOff := len(m.messages)
	if m.msgOffset > maxOff {
		m.msgOffset = maxOff
	}
	if m.msgOffset < 0 {
		m.msgOffset = 0
	}
}

func (m model) chatListHeight() int {
	h := m.height - 4 // header + footer + borders
	if h < 1 {
		h = 1
	}
	return h
}

func (m model) messageViewHeight() int {
	h := m.height - 6 // header + chat title + input + footer + borders
	if h < 3 {
		h = 3
	}
	return h
}

// ---------- Styles ----------

var (
	colorBg      = lipgloss.Color("#1e1e2e")
	colorPanel   = lipgloss.Color("#181825")
	colorBorder  = lipgloss.Color("#45475a")
	colorAccent  = lipgloss.Color("#89b4fa")
	colorGreen   = lipgloss.Color("#a6e3a1")
	colorYellow  = lipgloss.Color("#f9e2af")
	colorRed     = lipgloss.Color("#f38ba8")
	colorText    = lipgloss.Color("#cdd6f4")
	colorSubtext = lipgloss.Color("#a6adc8")
	colorMuted   = lipgloss.Color("#6c7086")
	colorSelBg   = lipgloss.Color("#313244")
	colorOutMsg  = lipgloss.Color("#89b4fa")
	colorInMsg   = lipgloss.Color("#cdd6f4")
	colorOutBg   = lipgloss.Color("#1e3a5f")
	colorInBg    = lipgloss.Color("#313244")

	headerStyle = lipgloss.NewStyle().
			Foreground(colorText).
			Background(colorPanel).
			Bold(true).
			Padding(0, 1)

	statusOKStyle   = lipgloss.NewStyle().Foreground(colorGreen).Bold(true)
	statusErrStyle  = lipgloss.NewStyle().Foreground(colorRed).Bold(true)
	statusWarnStyle = lipgloss.NewStyle().Foreground(colorYellow).Bold(true)
	mutedStyle      = lipgloss.NewStyle().Foreground(colorMuted)
	subtextStyle    = lipgloss.NewStyle().Foreground(colorSubtext)
	accentStyle     = lipgloss.NewStyle().Foreground(colorAccent).Bold(true)

	chatItemStyle   = lipgloss.NewStyle().Padding(0, 1)
	chatSelStyle    = lipgloss.NewStyle().Padding(0, 1).Background(colorSelBg).Foreground(colorText).Bold(true)
	chatUnreadStyle = lipgloss.NewStyle().Foreground(colorAccent).Bold(true)

	msgOutStyle    = lipgloss.NewStyle().Foreground(colorText).Background(colorOutBg).Padding(0, 1)
	msgInStyle     = lipgloss.NewStyle().Foreground(colorText).Background(colorInBg).Padding(0, 1)
	msgMetaStyle   = lipgloss.NewStyle().Foreground(colorMuted)
	msgSenderStyle = lipgloss.NewStyle().Foreground(colorAccent).Bold(true)

	inputStyle = lipgloss.NewStyle().
			Foreground(colorText).
			Background(colorPanel).
			Padding(0, 1)

	inputFocusBorder = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(colorAccent)

	inputBlurBorder = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(colorBorder)

	panelBorder = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(colorBorder)

	panelFocusBorder = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(colorAccent)

	helpStyle = lipgloss.NewStyle().Foreground(colorMuted).Background(colorPanel).Padding(0, 1)
)

// ---------- View ----------

func (m model) View() string {
	if m.width == 0 || m.height == 0 {
		return "Загрузка Max+ TUI..."
	}

	header := m.renderHeader()
	footer := m.renderFooter()

	innerH := m.height - lipgloss.Height(header) - lipgloss.Height(footer)
	if innerH < 3 {
		innerH = 3
	}

	leftW := m.width * 2 / 5
	if leftW < 24 {
		leftW = 24
	}
	if leftW > 44 {
		leftW = 44
	}
	if m.width < 60 {
		// narrow terminal: show only chat list or only messages
		if m.activeChat != nil && m.focus == focusInput {
			return lipgloss.JoinVertical(lipgloss.Left, header, m.renderMessages(m.width, innerH), footer)
		}
		return lipgloss.JoinVertical(lipgloss.Left, header, m.renderChatList(m.width, innerH), footer)
	}
	rightW := m.width - leftW
	if rightW < 20 {
		rightW = 20
		leftW = m.width - rightW
	}

	left := m.renderChatList(leftW, innerH)
	right := m.renderMessages(rightW, innerH)
	body := lipgloss.JoinHorizontal(lipgloss.Top, left, right)

	return lipgloss.JoinVertical(lipgloss.Left, header, body, footer)
}

func (m model) renderHeader() string {
	title := " Max+ TUI "
	status := ""
	if m.statusErr != "" {
		status = statusErrStyle.Render("● ошибка")
	} else if m.status.Authenticated && m.status.Connected {
		who := ""
		if m.status.Phone != nil && *m.status.Phone != "" {
			who = " " + *m.status.Phone
		}
		status = statusOKStyle.Render("● online") + subtextStyle.Render(who)
	} else if m.status.Authenticated {
		status = statusWarnStyle.Render("● подключение...")
	} else {
		status = statusWarnStyle.Render("● не авторизован")
	}

	unread := ""
	if m.status.Unread > 0 {
		unread = chatUnreadStyle.Render(fmt.Sprintf("  unread: %d", m.status.Unread))
	}

	right := status + unread
	gap := m.width - lipgloss.Width(title) - lipgloss.Width(right) - 2
	if gap < 1 {
		gap = 1
	}
	line := title + strings.Repeat(" ", gap) + right + " "
	return headerStyle.Width(m.width).Render(line)
}

func (m model) renderFooter() string {
	var parts []string
	switch m.search {
	case searchChats:
		parts = []string{"/" + m.searchQuery, "Enter открыть", "Esc отмена", "↑/↓ выбор", "Ctrl+U очистить"}
	case searchMessages:
		parts = []string{"/" + m.searchQuery, "Enter найти", "Esc отмена", "Ctrl+U очистить"}
	default:
		if m.focus == focusChats {
			parts = []string{"/ поиск", "←/→ папки", "↑/↓ навигация", "Enter открыть", "Tab ввод", "Ctrl+R", "q выход"}
		} else {
			parts = []string{"/ поиск", "Enter отправить", "Esc чаты", "↑/↓ скролл", "PgUp/PgDn", "Ctrl+R", "Ctrl+C выход"}
		}
		if len(m.msgSearchMatches) > 0 {
			parts = append([]string{fmt.Sprintf("match %d/%d (n/N)", m.msgSearchIdx+1, len(m.msgSearchMatches))}, parts...)
		}
	}
	if m.statusErr != "" {
		parts = append([]string{"ERR: " + truncate(m.statusErr, 40)}, parts...)
	}
	return helpStyle.Width(m.width).Render(strings.Join(parts, "  •  "))
}

func (m model) renderFolderTabs(width int) string {
	if len(m.folders) == 0 {
		return ""
	}
	var parts []string
	for i, f := range m.folders {
		title := f.Title
		if title == "" {
			title = f.ID
		}
		label := " " + title + " "
		if i == m.folderIdx {
			parts = append(parts, accentStyle.Render(label))
		} else {
			parts = append(parts, mutedStyle.Render(label))
		}
	}
	line := strings.Join(parts, "")
	// truncate to width
	if lipgloss.Width(line) > width {
		// show window around selected
		var left, right []string
		// build selected
		sel := m.folders[m.folderIdx]
		selTitle := sel.Title
		if selTitle == "" {
			selTitle = sel.ID
		}
		selLabel := accentStyle.Render(" " + selTitle + " ")
		selW := lipgloss.Width(selLabel)
		budget := width - selW - 2
		if budget < 0 {
			budget = 0
		}
		// add neighbors
		l, r := m.folderIdx-1, m.folderIdx+1
		leftW, rightW := 0, 0
		for budget > 0 && (l >= 0 || r < len(m.folders)) {
			added := false
			if r < len(m.folders) {
				t := m.folders[r].Title
				if t == "" {
					t = m.folders[r].ID
				}
				lbl := mutedStyle.Render(" " + t + " ")
				w := lipgloss.Width(lbl)
				if rightW+w <= budget/2+budget%2 || l < 0 {
					if rightW+w <= budget {
						right = append(right, lbl)
						rightW += w
						budget -= w
						added = true
					}
				}
				r++
			}
			if l >= 0 && budget > 0 {
				t := m.folders[l].Title
				if t == "" {
					t = m.folders[l].ID
				}
				lbl := mutedStyle.Render(" " + t + " ")
				w := lipgloss.Width(lbl)
				if leftW+w <= budget {
					left = append([]string{lbl}, left...)
					leftW += w
					budget -= w
					added = true
				}
				l--
			}
			if !added {
				break
			}
		}
		line = strings.Join(left, "") + selLabel + strings.Join(right, "")
		if m.folderIdx > 0 {
			line = mutedStyle.Render("‹") + line
		}
		if m.folderIdx < len(m.folders)-1 {
			line = line + mutedStyle.Render("›")
		}
	}
	return truncateStyled(line, width)
}

func (m model) renderChatList(w, h int) string {
	border := panelBorder
	if m.focus == focusChats {
		border = panelFocusBorder
	}
	innerW := w - 2
	innerH := h - 2
	if innerW < 10 {
		innerW = 10
	}
	if innerH < 1 {
		innerH = 1
	}

	baseChats := m.folderChats()
	searching := m.search == searchChats
	displayChats := baseChats
	if searching {
		displayChats = m.displayChats()
	}

	title := accentStyle.Render(" Чаты ")
	if searching {
		title += mutedStyle.Render(fmt.Sprintf("(%d/%d)", len(displayChats), len(baseChats)))
	} else if len(baseChats) > 0 {
		title += mutedStyle.Render(fmt.Sprintf("(%d)", len(baseChats)))
	}

	var lines []string
	lines = append(lines, title)
	if len(m.folders) > 0 {
		lines = append(lines, m.renderFolderTabs(innerW))
	}
	if searching {
		lines = append(lines, statusWarnStyle.Render("/"+m.searchQuery+"▌"))
	}

	if m.loading && len(m.chats) == 0 {
		lines = append(lines, mutedStyle.Render(" Загрузка..."))
	} else if len(m.chats) == 0 {
		lines = append(lines, mutedStyle.Render(" Нет чатов"))
		if !m.status.Authenticated {
			lines = append(lines, "")
			lines = append(lines, statusWarnStyle.Render(" Нужна авторизация:"))
			lines = append(lines, mutedStyle.Render(" maxplus-daemon login"))
		}
	} else if len(displayChats) == 0 {
		if searching {
			lines = append(lines, mutedStyle.Render(" Нет совпадений"))
		} else {
			lines = append(lines, mutedStyle.Render(" Нет чатов в папке"))
		}
	} else {
		headerLines := 1
		if len(m.folders) > 0 {
			headerLines++
		}
		if searching {
			headerLines++
		}
		visibleItems := (innerH - headerLines) / 2
		if visibleItems < 1 {
			visibleItems = 1
		}
		// clamp cursor/offset to display list
		cursor := m.chatCursor
		if cursor >= len(displayChats) {
			cursor = len(displayChats) - 1
		}
		if cursor < 0 {
			cursor = 0
		}
		offset := m.chatOffset
		if cursor < offset {
			offset = cursor
		}
		if cursor >= offset+visibleItems {
			offset = cursor - visibleItems + 1
		}
		if offset < 0 {
			offset = 0
		}
		end := offset + visibleItems
		if end > len(displayChats) {
			end = len(displayChats)
		}
		for i := offset; i < end; i++ {
			c := displayChats[i]
			lines = append(lines, m.renderChatItem(c, i == cursor, innerW))
		}
	}

	content := strings.Join(lines, "\n")
	return border.Width(w - 2).Height(innerH).Render(fitLines(content, innerW, innerH))
}

func (m model) renderChatItem(c chat, selected bool, w int) string {
	prefix := "  "
	if selected {
		prefix = "▸ "
	}
	unread := ""
	if c.Unread > 0 {
		unread = fmt.Sprintf(" (%d)", c.Unread)
	}
	title := truncate(c.Title, max(8, w-len(prefix)-len(unread)-2))
	last := truncate(strings.ReplaceAll(c.LastMessage, "\n", " "), max(8, w-4))

	line1 := prefix + title + unread
	line2 := "  " + last

	if selected {
		return chatSelStyle.Width(w).Render(truncate(line1, w)) + "\n" + mutedStyle.Width(w).Render(truncate(line2, w))
	}
	if c.Unread > 0 {
		return chatUnreadStyle.Render(truncate(line1, w)) + "\n" + mutedStyle.Render(truncate(line2, w))
	}
	return truncate(line1, w) + "\n" + mutedStyle.Render(truncate(line2, w))
}

func (m model) renderMessages(w, h int) string {
	border := panelBorder
	if m.focus == focusInput {
		border = panelFocusBorder
	}
	innerW := w - 2
	innerH := h - 2
	if innerW < 10 {
		innerW = 10
	}
	if innerH < 3 {
		innerH = 3
	}

	if m.activeChat == nil {
		content := mutedStyle.Render("Выберите чат слева (Enter)\n\nTab - переключение фокуса\nCtrl+R - обновить\nq - выход")
		if !m.status.Authenticated {
			content = statusWarnStyle.Render("Не авторизован") + "\n\n" + mutedStyle.Render("Запустите: maxplus-daemon login\nЗатем нажмите Ctrl+R")
		}
		return border.Width(w - 2).Height(innerH).Render(fitLines(content, innerW, innerH))
	}

	// layout: title(1) + messages + input(3) + err(0/1)
	titleH := 1
	inputH := 3
	errH := 0
	if m.inputErr != "" || m.msgErr != "" {
		errH = 1
	}
	msgH := innerH - titleH - inputH - errH
	if msgH < 1 {
		msgH = 1
	}

	chatTitle := m.activeChat.Title
	if chatTitle == "" {
		chatTitle = fmt.Sprintf("Chat %d", m.activeChat.ID)
	}
	kindLabel := ""
	if strings.EqualFold(m.activeChat.Kind, "group") || strings.EqualFold(m.activeChat.Kind, "channel") {
		kindLabel = " [" + strings.ToLower(m.activeChat.Kind) + "]"
	}
	title := accentStyle.Render(" " + truncate(chatTitle+kindLabel, innerW-2) + " ")
	if m.typing {
		title += statusWarnStyle.Render(" печатает...")
	} else if m.loadingMsg {
		title += mutedStyle.Render(" загрузка...")
	}
	if m.search == searchMessages {
		title += statusWarnStyle.Render(" /" + m.searchQuery + "▌")
	} else if len(m.msgSearchMatches) > 0 {
		title += mutedStyle.Render(fmt.Sprintf(" [%d/%d]", m.msgSearchIdx+1, len(m.msgSearchMatches)))
	}

	msgArea := m.renderMessageLines(innerW, msgH)
	errLine := ""
	if m.msgErr != "" {
		errLine = statusErrStyle.Render(truncate(" "+m.msgErr, innerW))
	} else if m.inputErr != "" {
		errLine = statusErrStyle.Render(truncate(" "+m.inputErr, innerW))
	}

	input := m.renderInput(innerW)

	parts := []string{title, msgArea}
	if errLine != "" {
		parts = append(parts, errLine)
	}
	parts = append(parts, input)
	content := strings.Join(parts, "\n")
	return border.Width(w - 2).Height(innerH).Render(content)
}

func (m model) renderMessageLines(w, h int) string {
	if len(m.messages) == 0 {
		if m.loadingMsg {
			return fitLines(mutedStyle.Render(" Загрузка сообщений..."), w, h)
		}
		return fitLines(mutedStyle.Render(" Нет сообщений"), w, h)
	}

	// render all messages to lines, then take the visible window from bottom
	var all []string
	maxBubble := w - 4
	if maxBubble < 20 {
		maxBubble = 20
	}
	if maxBubble > 72 {
		maxBubble = 72
	}

	var lastDate string
	for _, msg := range m.messages {
		tsMs := msg.Timestamp
		if tsMs < 1_000_000_000_000 {
			tsMs = tsMs * 1000
		}
		ts := time.UnixMilli(tsMs)
		date := ts.Format("02.01.2006")
		if date != lastDate {
			sep := mutedStyle.Render(centerText(date, w))
			all = append(all, sep)
			lastDate = date
		}

		timeStr := ts.Format("15:04")
		statusMark := ""
		if msg.Outgoing {
			statusMark = formatStatus(msg.Status)
		}

		sender := ""
		if !msg.Outgoing && msg.Sender != "" {
			sender = msg.Sender
		}

		text := msg.Text
		if text == "" {
			text = attachLabel(msg.Attaches)
		}
		if text == "" {
			text = "(пустое сообщение)"
		}

		contentWidth := maxBubble - 2 // padding inside bubble
		if contentWidth < 10 {
			contentWidth = 10
		}
		wrapped := wrapText(text, contentWidth)

		// build bubble lines
		var bubbleLines []string
		if sender != "" {
			bubbleLines = append(bubbleLines, msgSenderStyle.Render(truncate(sender, contentWidth)))
		}
		for _, wl := range wrapped {
			bubbleLines = append(bubbleLines, wl)
		}
		metaRendered := msgMetaStyle.Render(timeStr)
		if statusMark != "" {
			stStyle := msgMetaStyle
			if isReadStatus(msg.Status) {
				stStyle = lipgloss.NewStyle().Foreground(colorGreen)
			}
			metaRendered += stStyle.Render(statusMark)
		}
		bubbleLines = append(bubbleLines, metaRendered)

		bubbleText := strings.Join(bubbleLines, "\n")
		var rendered string
		if msg.Outgoing {
			rendered = msgOutStyle.Width(min(maxBubble, lipgloss.Width(bubbleText)+2)).Render(bubbleText)
			// right align
			pad := w - lipgloss.Width(rendered)
			if pad < 0 {
				pad = 0
			}
			for _, l := range strings.Split(rendered, "\n") {
				all = append(all, strings.Repeat(" ", pad)+l)
			}
		} else {
			rendered = msgInStyle.Width(min(maxBubble, lipgloss.Width(bubbleText)+2)).Render(bubbleText)
			for _, l := range strings.Split(rendered, "\n") {
				all = append(all, l)
			}
		}
	}

	// apply scroll offset from bottom
	total := len(all)
	end := total - m.msgOffset
	if end > total {
		end = total
	}
	if end < 0 {
		end = 0
	}
	start := end - h
	if start < 0 {
		start = 0
	}

	visible := all[start:end]
	// pad top if fewer lines than height
	for len(visible) < h {
		visible = append([]string{""}, visible...)
	}
	if len(visible) > h {
		visible = visible[len(visible)-h:]
	}
	return strings.Join(visible, "\n")
}

func (m model) renderInput(w int) string {
	borderStyle := inputBlurBorder
	if m.focus == focusInput {
		borderStyle = inputFocusBorder
	}
	innerW := w - 4
	if innerW < 10 {
		innerW = 10
	}

	prompt := "> "
	if m.sending {
		prompt = "… "
	}
	placeholder := "Введите сообщение..."
	text := m.input
	if text == "" && m.focus != focusInput {
		text = mutedStyle.Render(placeholder)
	} else if text == "" {
		text = mutedStyle.Render(placeholder)
	} else {
		text = truncateLeft(text, innerW-len(prompt))
	}

	cursor := ""
	if m.focus == focusInput && !m.sending {
		cursor = "▌"
	}

	line := prompt + text + cursor
	return borderStyle.Width(w - 2).Render(truncate(line, innerW))
}

func isReadStatus(raw json.RawMessage) bool {
	s := strings.ToLower(strings.Trim(strings.TrimSpace(string(raw)), "\""))
	return s == "read" || s == "3"
}

func formatStatus(raw json.RawMessage) string {
	s := strings.TrimSpace(string(raw))
	s = strings.Trim(s, "\"")
	switch strings.ToLower(s) {
	case "read", "3":
		return " ✓✓"
	case "delivered", "2":
		return " ✓✓"
	case "sent", "1":
		return " ✓"
	case "error", "failed", "-1":
		return " !"
	default:
		if s == "" || s == "null" || s == "0" {
			return ""
		}
		return ""
	}
}

func attachLabel(attaches json.RawMessage) string {
	s := strings.TrimSpace(string(attaches))
	if s == "" || s == "[]" || s == "null" {
		return ""
	}
	a := strings.ToLower(s)
	switch {
	case strings.Contains(a, "\"audio\"") || strings.Contains(a, "\"voice\""):
		return "[Аудио]"
	case strings.Contains(a, "\"video\""):
		return "[Видео]"
	case strings.Contains(a, "\"photo\""):
		return "[Фото]"
	case strings.Contains(a, "\"sticker\""):
		return "[Стикер]"
	case strings.Contains(a, "\"file\""):
		return "[Файл]"
	default:
		return "[Вложение]"
	}
}

func truncate(s string, w int) string {
	if w <= 0 {
		return ""
	}
	r := []rune(s)
	if len(r) <= w {
		return s
	}
	if w <= 1 {
		return string(r[:w])
	}
	return string(r[:w-1]) + "…"
}

func truncateLeft(s string, w int) string {
	if w <= 0 {
		return ""
	}
	r := []rune(s)
	if len(r) <= w {
		return s
	}
	if w <= 1 {
		return string(r[len(r)-w:])
	}
	return "…" + string(r[len(r)-(w-1):])
}

func truncateStyled(s string, w int) string {
	if w <= 0 || lipgloss.Width(s) <= w {
		return s
	}
	// Simple ANSI-aware truncation
	var out strings.Builder
	visible := 0
	inEsc := false
	for _, r := range s {
		if r == '\x1b' {
			inEsc = true
			out.WriteRune(r)
			continue
		}
		if inEsc {
			out.WriteRune(r)
			if (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') {
				inEsc = false
			}
			continue
		}
		if visible >= w {
			break
		}
		out.WriteRune(r)
		visible++
	}
	out.WriteString("\x1b[0m")
	return out.String()
}

func wrapText(s string, width int) []string {
	if width <= 0 {
		width = 10
	}
	var lines []string
	for _, paragraph := range strings.Split(s, "\n") {
		paragraph = strings.ReplaceAll(paragraph, "\r", "")
		if paragraph == "" {
			lines = append(lines, "")
			continue
		}
		words := strings.Fields(paragraph)
		if len(words) == 0 {
			lines = append(lines, "")
			continue
		}
		cur := words[0]
		for _, w := range words[1:] {
			if len([]rune(cur))+1+len([]rune(w)) <= width {
				cur += " " + w
			} else {
				lines = append(lines, splitLongWord(cur, width)...)
				cur = w
			}
		}
		lines = append(lines, splitLongWord(cur, width)...)
	}
	if len(lines) == 0 {
		return []string{""}
	}
	return lines
}

func splitLongWord(s string, width int) []string {
	r := []rune(s)
	if len(r) <= width {
		return []string{s}
	}
	var out []string
	for len(r) > width {
		out = append(out, string(r[:width]))
		r = r[width:]
	}
	if len(r) > 0 {
		out = append(out, string(r))
	}
	return out
}

func fitLines(content string, w, h int) string {
	_ = w
	lines := strings.Split(content, "\n")
	if len(lines) > h {
		lines = lines[:h]
	}
	for len(lines) < h {
		lines = append(lines, "")
	}
	return strings.Join(lines, "\n")
}

func centerText(s string, w int) string {
	r := []rune(s)
	if len(r) >= w {
		return s
	}
	pad := (w - len(r)) / 2
	return strings.Repeat(" ", pad) + s
}
