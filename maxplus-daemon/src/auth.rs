mod auth;
mod storage;
mod dbus_servicer;

pub use dbus_servicer::MaxPlusDbusServicer;

const DBUS_SERVICE: &str = "org.maxplus.Client";
const DBUS_PATH: &str = "/org/maxplus/Client";
const DBUS_INTERFACE: &str = "org.maxplus.Client";

/// Authentication status states
#[derive(Debug, Clone, PartialEq)]
pub enum AuthStatus {
    Unauthenticated,
    Authenticating,
    Authenticated,
    VerificationRequired,
}

/// Configuration for the daemon
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub database_path: Option<String>,
    pub phone_number: Option<String>,
    pub auth_token: Option<String>,
}

/// Main daemon struct
pub struct MaxPlusDaemon {
    client: rumax::MaxClient,
    storage: storage::MaxPlusStorage,
    dbus_servicer: dbus_servicer::MaxPlusDbusServicer,
    dbus_connection: Option<String>,
    auth_status: AuthStatus,
    phone_number: Option<String>,
    auth_token: Option<String>,
}

impl MaxPlusDaemon {
    pub fn new(config: DaemonConfig) -> anyhow::Result<Self> {
        env_logger::init();

        let client = rumax::MaxClient::new();
        let storage = storage::MaxPlusStorage::new(&config.database_path);
        let dbus_servicer = dbus_servicer::MaxPlusDbusServicer::new(&client, &storage);

        Ok(MaxPlusDaemon {
            client,
            storage,
            dbus_servicer,
            dbus_connection: None,
            auth_status: AuthStatus::Unauthenticated,
            phone_number: config.phone_number,
            auth_token: config.auth_token,
        })
    }

    /// Start the authentication flow
    /// In a real implementation, this would:
    /// 1. Send the phone number to the Max server for verification
    /// 2. Receive a verification code via SMS
    /// 3. Prompt the user to enter the code
    /// 3. Validate the code with the server
    pub async fn authenticate(&mut self, phone_number: String) -> anyhow::Result<()> {
        self.phone_number = Some(phone_number.clone());
        self.auth_status = AuthStatus::Authenticating;

        // Placeholder: In real implementation, this would:
        // 1. Send the phone number to Max's authentication server
        // 2. Trigger an SMS with verification code to the phone
        // 3. Return a verification ID that the user needs to enter

        // For now, simulate the flow with a verification code entry
        println!("\n=== Max+ Authentication ===");
        println!("Phone number: {}", phone_number);
        println!("\nA verification code has been sent via SMS to your phone.");
        println!("Please enter the verification code you received:");

        // Read verification code from stdin
        let mut code = String::new();
        std::io::stdin().read_line(&mut code).map_err(|e| anyhow::anyhow!("Failed to read input: {}", e))?;
        let code = code.trim();

        // Validate the verification code (placeholder - in real impl, send to server)
        if self.verify_verification_code(&code).await? {
            self.auth_status = AuthStatus::Authenticated;
            self.auth_token = Some(generate_auth_token());
            println!("\n✅ Authentication successful!");
            println!("Welcome to Max+!");
            println!("Your auth token: {}", self.auth_token.as_ref().unwrap());
        } else {
            self.auth_status = AuthStatus::VerificationRequired;
            println!("\n❌ Invalid verification code. Authentication failed.");
            println!("Please try again with the correct code.");
        }

        Ok(())
    }

    /// Verify the verification code with the Max server
    async fn verify_verification_code(&self, code: &str) -> anyhow::Result<bool> {
        // Placeholder implementation
        // In real application, this would send the code to Max's server
        // and check if it's valid

        // For demonstration, accept code "123456" as valid
        if code == "123456" {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get the current authentication status
    pub fn auth_status(&self) -> &AuthStatus {
        &self.auth_status
    }

    /// Check if the daemon is authenticated
    pub fn is_authenticated(&self) -> bool {
        self.auth_status == AuthStatus::Authenticated
    }

    /// Get the auth token
    pub fn auth_token(&self) -> Option<&String> {
        self.auth_token.as_ref()
    }

    /// Generate a simple auth token
    fn generate_auth_token() -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let token: String = (0..32)
            .map(|_| {
                let n = rng.gen_range(0..36);
                if n < 10 {
                    char::from_digit(n, 10).unwrap()
                } else {
                    char::from_u32(n - 10 + b'a' as u32).unwrap()
                }
            })
            .collect();
        token
    }
}