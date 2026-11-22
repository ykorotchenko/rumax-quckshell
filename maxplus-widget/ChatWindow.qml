import QtQuick 2.15
import org.kde.quick.shell 2.15

// Pop-up chat window component
// Connects to maxplus-daemon via D-Bus for message sending and history

Dialog {
    id: chatDialog
    title: "Max+"
    visible: false
    modal: false
    width: 600
    height: 400
    minimumWidth: 400
    minimumHeight: 300
    
    // Set Qt background color
    background: Rectangle {
        color: "#F8F8F8"
        radius: 8
    }
    
    // Header with contact info and close button
    HeaderBar {
        id: header
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        Height: 32
        BackgroundRole: null
        
        // Contact name label
        Text {
            id: contactName
            text: "Loading..."
            anchorsVerticalCenter: header.verticalCenter
            leftPadding: 8
            font.pointSize: 13
            font.weight: Font.Bold
        }
        
        // Close button
        Button {
            anchorsRight: header.right
            anchorsVerticalCenter: header.verticalCenter
            text: "Close"
            onClicked: {
                chatDialog.visible = false;
            }
        }
    }
    
    // Chat message list
    ScrollView {
        anchors.top: header.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: inputRow.top
        clip: true
        
        Flickable.flickableDirection: Flickable.Vich
        
        // Message container
        Column {
            id: messagesColumn
            spacing: 4
            padding: 4
            implicitHeight: Math.max(200, parent.height)
            // Add stretching at bottom
            Layout.fillHeight: true
        }
    }
    
    // Load initial messages when shown
    onVisibleChanged: {
        if (visible) {
            // Fetch chat info via D-Bus
            var process = new Process();
            var output = process.start("dbus-send", ["--session", "--dest=org.maxplus.Client", "--print-reply", 
                "/org/maxplus/Client", "org.maxplus.Client.GetChatInfo", "uint32:0"]);
            
            // Fetch message history
            var msgOutput = process.start("dbus-send", ["--session", "--dest=org.maxplus.Client", "--print-reply", 
                "/org/maxplus/Client", "org.maxplus.Client.GetMessages", "uint32:0", "uint32:50"]);
        }
    }
    
    // Input area at bottom
    Item {
        id: inputRow
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        Height: 50
        
        Rectangle {
            anchors.fill: parent
            color: "#FFFFFF"
            radius: 8
            // Shadow effect
            Behavior on color { Duration: 0 }
        }
        
        // Message input field
        TextField {
            id: messageInput
            anchors.fill: parent
            leftPadding: 12
            rightPadding: 12
            placeholderText: "Type a message..."
            returnKeyReturnLabel: Qt.Submit
            onSubmitted: {
                sendMessage();
            }
            
            // Send on Enter key (Shift+Enter for new line)
            onTextChanged: {
                if (messageInput.text.endsWith("\n") && messageInput.text.length > 1) {
                    // Remove the newline and send
                    messageInput.text = messageInput.text.remove(messageInput.text.length - 1, 1);
                    sendMessage();
                }
            }
        }
        
        // Send button
        Button {
            rightPadding: 12
            anchorsVerticalCenter: messageInput.verticalCenter
            text: "Send"
            onClicked: {
                sendMessage();
            }
        }
    }
    
    // ==========================================
    // Send message function via D-Bus
    // ==========================================
    function sendMessage() {
        const text = messageInput.text.trim();
        if (text.length === 0) return;
        
        // Send message via D-Bus using dbus-send
        var dbusCmd = "dbus-send --session --dest=org.maxplus.Client --print-reply " +
            "/org/maxplus/Client org.maxplus.Client.SendMessage string:" + text + " int32:" + currentChatId;
        var process = new Process();
        var output = process.start("dbus-send", ["--session", "--dest=org.maxplus.Client", "--print-reply", 
            "/org/maxplus/Client", "org.maxplus.Client.SendMessage", "string:" + text, "int32:" + currentChatId]);
        
        // Add local message preview immediately
        addLocalMessage(text, true);
        
        // Clear input
        messageInput.clear();
    }
    
    // ==========================================
    // Add message to UI
    // ==========================================
    function addLocalMessage(text: String, isOutgoing: bool) {
        const msgItem = {
            text: text,
            isOutgoing: isOutgoing,
            time: new Date()
        };
        
        // Create message rectangle
        const msgRect = {
            id: "msg_" + Date.now(),
            text: text,
            isOutgoing: isOutgoing,
            color: isOutgoing ? "#0084FF" : "#717171",
            padding: 8
        };
        
        // Add to message column
        messagesColumn.children.push(msgRect);
        
        // Scroll to bottom
        // messagesColumn.forceLayout();
    }
    
    // ==========================================
    // Process helper class for dbus-send
    // ==========================================
    function Process() {
        this._process = new QProcess();
        
        this.start = function(command, args) {
            this._process.clearArguments();
            this._process.clearArgs();
            this._process.setCommand(command);
            if (args) {
                for (var i = 0; i < args.length; i++) {
                    this._process.addArgument(args[i]);
                }
            }
            this._process.start();
            this._process.waitForFinished(5000);
            var output = QString::fromLocal8Bit(this._process.readAllStandardOutput());
            var exitCode = this._process.exitCode();
            return { exitCode: exitCode, output: output };
        }
    }
}
}otype>