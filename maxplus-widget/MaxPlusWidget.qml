import QtQuick 2.15
import org.kde.quick.shell 2.15
import Qt.labs.platform 1.0

// Top-right corner widget for Max+ Messenger
// Shows icon with unread count, opens chat on click

Rectangle {
    id: root
    property int unreadCount: 0
    property string avatarUrl: ""
    property u64 currentChatId: 0
    
    // Don't steal focus unless clicked
    focus: false
    
    // Compact mode - just shows icon with count
    width: themes.iconSizeSmall
    height: themes.iconSizeSmall
    
    // Tooltip with status info
    ToolTipText {
        text: root.unreadCount > 0 ? `Max+: ${root.unreadCount} unread` : "Max+"
    }
    
    // Main icon area
    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHand
        onClicked: {
            // Toggle between panel view and chat view
            if (root.currentChatId > 0) {
                // Open specific chat
                invokeMethod("org.maxplus.client", "OpenChat", 
                    QtVariantVariant { args: [root.currentChatId] });
            } else {
                // Show/hide panel
                invokeMethod("org.maxplus.client", "TogglePanel", 
                    QtVariantVariant { args: [] });
            }
        }
    }
    
    // Avatar display
    Image {
        id: avatarImage
        source: root.avatarUrl
        anchors.left: parent.left
        anchors.leftMargin: 4
        anchors.verticalCenter: parent.verticalCenter
        width: 32
        height: 32
        radius: 8
        smooth: true
        
        // Placeholder if no avatar
        fallback.width: 32
        fallback.height: 32
    }
    
    // Unread count badge
    Item {
        id: countBadge
        visible: root.unreadCount > 0
        anchors.right: parent.right
        anchors.rightMargin: 2
        anchors.verticalCenter: parent.verticalCenter
        
        Rectangle {
            width: 20
            height: 20
            radius: 10
            color: "#FF3B30"
            anchors.centerIn: parent
            
            Text {
                anchors.centerIn: parent
                text: root.unreadCount
                color: "white"
                font.pixelSize: 12
                font.bold: true
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }
    }
    
    // Status indicator dot
    Item {
        id: statusDot
        visible: true
        anchors.left: parent.left
        anchors.leftMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        
        Rectangle {
            width: 8
            height: 8
            radius: 4
            color: root.unreadCount > 0 ? "#FF3B30" : "#4CAF50"
        }
    }
    
    // Animation when count changes
    NumberAnimation on unreadCount {
        duration: 200
    }
}