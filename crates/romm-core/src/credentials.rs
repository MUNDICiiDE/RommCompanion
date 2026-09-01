use thiserror::Error;

pub const CLIENT_TOKEN_LOCATOR: &str = "system:romm-client-token";
const CREDENTIAL_SERVICE: &str = "app.romm-companion.desktop";
const CREDENTIAL_ACCOUNT: &str = "romm-client-token";

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("the operating-system credential store is unavailable: {0}")]
    Unavailable(String),
    #[error("no saved RomM credential exists")]
    Missing,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCredentialStore;

impl SystemCredentialStore {
    pub fn save(&self, token: &str) -> Result<&'static str, CredentialError> {
        platform_entry()?
            .set_password(token)
            .map_err(safe_keyring_error)?;
        Ok(CLIENT_TOKEN_LOCATOR)
    }

    pub fn load(&self, locator: &str) -> Result<String, CredentialError> {
        if locator != CLIENT_TOKEN_LOCATOR {
            return Err(CredentialError::Missing);
        }
        platform_entry()?
            .get_password()
            .map_err(|error| match error {
                keyring::Error::NoEntry => CredentialError::Missing,
                other => safe_keyring_error(other),
            })
    }

    pub fn delete(&self, locator: &str) -> Result<(), CredentialError> {
        if locator != CLIENT_TOKEN_LOCATOR {
            return Ok(());
        }
        match platform_entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(safe_keyring_error(error)),
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn platform_entry() -> Result<keyring::Entry, CredentialError> {
    keyring::Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT).map_err(safe_keyring_error)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_entry() -> Result<keyring::Entry, CredentialError> {
    Err(CredentialError::Unavailable(
        "this operating system is not supported in v1".to_owned(),
    ))
}

fn safe_keyring_error(error: keyring::Error) -> CredentialError {
    CredentialError::Unavailable(error.to_string())
}
