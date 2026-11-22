use log::{info, error, debug};
use std::sync::Arc;
use dbus::{
    Path,
    SignalReceiver,
    Methods,
    Properties,
    Connection,
};
use dbus::message::Signal;
use rumax::MaxClient;
use std::path::PathBuf;

mod dbus_servicer;
mod storage;

pub use dbus_servicer::MaxPlusDbusServicer;
pub use storage::MaxPlusStorage;

const DBUS_SERVICE: &str = "org.maxplus.Client";
const DBUS_PATH: &str = "/org/maxplus/Client";
const DBUS_INTERFACE: &str = "org.maxplus.Client";

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub database_path: Option<PathBuf>,
    pub log_level: String,
}

pub struct MaxPlusDaemon {
    client: MaxClient,
    dbus_servicer: MaxPlusDbusServicer,
    storage: MaxPlusStorage,
    dbus_connection: Option<Connection>,
}

impl MaxPlusDaemon {
    pub fn new(config: DaemonConfig) -> anyhow::Result<Self> {
        env_logger::init();
        
        let client = MaxClient::new();
        let storage = MaxPlusStorage::new(config.database_path.clone());
        let dbus_servicer = MaxPlusDbusServicer::new(&client, &storage);
        
        Ok(MaxPlusDaemon {
            client,
            dbus_servicer,
            storage,
            dbus_connection: None,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        info!("Starting maxplus-daemon...");
        
        // Connect to system D-Bus
        let connection = dbus::BusConnection::session()?;
        self.dbus_connection = Some(connection.clone());
        
        // Register the D-Bus service
        let name = dbus::Name::BusName(DBUS_SERVICE.to_string());
        connection.request_name(&name, dbus::NameFlags::REPLACE_EXISTING | dbus::NameFlags::DO_NOT_QUEUE)?;
        
        // Register object at our path
        let object = dbus::ObjectServer::builder()
            .name(&DBUS_SERVICE)?
            .path(DBUS_PATH)?
            .interface(DBUS_INTERFACE)?
            .add_method("GetStatus", get_status_method)?;
        // ... more methods
        
        connection.add_object(DBUS_PATH, DBUS_INTERFACE, object)?;
        
        info!("D-Bus service registered: {}@{}", DBUS_SERVICE, DBUS_PATH);
        
        // Start listening for events from rumax
        let event_rx = self.client.subscribe();
        
        // Process events
        for event in event_rx {
            // Handle events from rumax
            debug!("Received event: {:?}", event);
            // Forward to D-Bus subscribers, etc.
        }
        
        Ok(())
    }
}

fn get_status_method(_: dbus::method::Call<dbus::server::MethodCall>) -> anyhow::Result<dbus::method::Response> {
    Ok(dbus::method::Response::new_empty())
}