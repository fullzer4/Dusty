use std::error::Error;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tokio::signal::unix::{signal, SignalKind};
use tokio::select;
use zbus::{Connection, interface};
use log::{info, warn, error, debug, LevelFilter};
use chrono::Local;
use env_logger::Builder;
use std::io::Write;

#[derive(Debug, Clone)]
struct Notification {
    id: u32,
    app_name: String,
    summary: String,
    body: String,
    icon: String,
    expire_timeout: i32,
}

#[derive(Clone)]
struct NotificationDaemon {
    notifications: Arc<Mutex<HashMap<u32, Notification>>>,
    next_id: Arc<Mutex<u32>>,
}

impl NotificationDaemon {
    fn new() -> Self {
        Self {
            notifications: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(1)),
        }
    }

    fn next_id(&self) -> u32 {
        let mut id = self.next_id.lock().unwrap();
        let current_id = *id;
        *id = id.wrapping_add(1);
        if *id == 0 { *id = 1; }
        current_id
    }
    
    fn get_stats(&self) -> (usize, u32) {
        let notifications = self.notifications.lock().unwrap();
        let count = notifications.len();
        let next_id = *self.next_id.lock().unwrap();
        (count, next_id)
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationDaemon {
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<&str>,
        hints: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id > 0 { replaces_id } else { self.next_id() };
        
        let notification = Notification {
            id,
            app_name: app_name.to_string(),
            summary: summary.to_string(),
            body: body.to_string(),
            icon: app_icon.to_string(),
            expire_timeout,
        };
        
        info!("Notification #{} from {}: {} - {}", id, app_name, summary, body);
        
        if !actions.is_empty() {
            debug!("Actions available for notification #{}: {:?}", id, actions);
        }
        
        for (key, value) in hints {
            if key == "urgency" {
                if let Ok(urgency) = value.downcast_ref::<u8>() {
                    debug!("Urgency for notification #{}: {}", id, urgency);
                }
            }
        }
        
        self.notifications.lock().unwrap().insert(id, notification);
        
        id
    }

    async fn get_capabilities(&self) -> Vec<String> {
        debug!("Client requested capabilities");
        vec![
            "body".to_string(),
            "body-markup".to_string(),
            "actions".to_string(),
            "persistence".to_string(),
        ]
    }

    async fn close_notification(&self, id: u32) {
        let notification_closed = {
            let mut notifications = self.notifications.lock().unwrap();
            if let Some(notification) = notifications.remove(&id) {
                info!("Closing notification #{}: {}", id, notification.summary);
                true
            } else {
                warn!("Attempt to close nonexistent notification: #{}", id);
                false
            }
        };
        
        if notification_closed {
            debug!("Emitted NotificationClosed signal for #{}", id);
        }
    }

    async fn get_server_information(&self) -> (String, String, String, String) {
        debug!("Client requested server information");
        (
            "Dusty".to_string(),
            "fullzer4".to_string(),
            "0.1.0".to_string(),
            "1.2".to_string(),
        )
    }
}

fn setup_logger() -> Result<(), log::SetLoggerError> {
    Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{}] - {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .filter(None, LevelFilter::Info)
        .init();
    Ok(())
}

async fn run_daemon() -> Result<(), Box<dyn Error + Send + Sync>> {
    let daemon = NotificationDaemon::new();
    
    let daemon_stats = daemon.clone();

    let connection = Connection::session().await?;
    
    match connection.request_name("org.freedesktop.Notifications").await {
        Ok(_) => {
            info!("Successfully acquired D-Bus name");
        },
        Err(e) => {
            error!("Could not acquire D-Bus name: {}", e);
            error!("Another notification daemon is likely already running");
            error!("Please stop any existing notification daemon (like Dunst) before running Dusty");
            error!("You can typically do this with: killall dunst");
            return Err(e.into());
        }
    }
    
    connection
        .object_server()
        .at("/org/freedesktop/Notifications", daemon)
        .await?;

    info!("Dusty notification daemon is running");
    info!("You can test it by running: notify-send 'Test' 'This is a test notification'");
    
    let mut interval_counter = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        
        interval_counter += 1;
        if interval_counter % 10 == 0 {
            let (count, next_id) = daemon_stats.get_stats();
            info!("Status: {} active notifications, next ID: {}", count, next_id);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if let Err(e) = setup_logger() {
        eprintln!("Failed to set up logger: {}", e);
    }
    
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    
    info!("Starting Dusty notification daemon");
    
    let daemon_handle = tokio::spawn(async {
        if let Err(e) = run_daemon().await {
            error!("Error running daemon: {}", e);
            return false;
        }
        true
    });
    
    select! {
        _ = sigint.recv() => {
            info!("Received SIGINT signal, shutting down");
        },
        _ = sigterm.recv() => {
            info!("Received SIGTERM signal, shutting down");
        },
        result = daemon_handle => {
            match result {
                Ok(true) => info!("Daemon terminated normally"),
                Ok(false) => error!("Daemon terminated with error"),
                Err(e) => error!("Error joining daemon task: {}", e),
            }
            return Ok(());
        }
    }
    
    info!("Daemon successfully shut down");
    Ok(())
}