import QtQuick 2.15
import org.kde.quick.shell 2.15

// Pop-up chat window component
// Can be invoked from the main widget or via D-Bus

Dialog {
    id: chatDialog
    title: "Max+"
    visible: false
    modal: false
    width: 600
    height: 400
    
    // Header with contact info and close button
    HeaderBar {
        id: header
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        Height: 32
        
        Text {
            id: contactName
            text: "Loading..."
            anchorsVerticalCenter: header.verticalCenter
            leftPadding: 8
        }
        
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
        anchors.bottom: bottomButton.top
        clip: true
        
        Flickable.flickableDirection: Flickable.Vich
        
        // Message container
        Column {
            id: messagesColumn
            spacing: 4
            padding: 4
            implicitHeight: Math.max(200, parent.height)
        }
    }
    
    // Input area at bottom
    Item {
        id: bottomButton
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        Height: 40
        
        Rectangle {
            anchors.fill: parent
            color: "#F0F0F0"
            radius: 8
        }
        
        TextField {
            id: messageInput
            anchors.fill: parent
            leftPadding: 8
            rightPadding: 8
            placeholderText: "Type a message..."
            returnKeyReturnLabel: Qt.Submit
            onSubmitted: {
                if (messageInput.text.length > 0) {
                    // Send message via D-Bus
                    invokeMethod("org.maxplus.client", "SendMessage",
                        QtVariantVariant {
                            args: [currentChatId, messageInput.text]
                        });
                    messageInput.clear();
                }
            }
        }
        
        Button {
            rightPadding: 8
            anchorsVerticalCenter: messageInput.verticalCenter
            text: "Send"
            onClicked: {
                if (messageInput.text.length > 0) {
                    invokeMethod("org.maxplus.client", "SendMessage",
                        QtVariantVariant {
                            args: [currentChatId, messageInput.text]
                        });
                    messageInput.clear();
                }
            }
        }
    }
    
    // Load chat data when shown
    onVisibleChanged: {
        if (visible) {
            // Fetch chat info via D-Bus
            invokeMethod("org.maxplus.client", "GetChatInfo",
                QtVariantVariant {
                    args: [currentChatId]
                });
            
            // Fetch message history
            invokeMethod("org.maxplus.client", "GetMessages",
                QtVariantVariant {
                    args: [currentChatId, 50]  // last 50 messages
                });
        }
    }
}