// SPDX-License-Identifier: GPL-3.0-or-later
//! Vault passwords live only in the platform credential store, never in settings.
use crate::{error, Config};
use papertrail_crypto::{Account, Vault};
use serde::{Deserialize, Serialize};
use std::io;
use zeroize::Zeroizing;

pub(crate) trait Store: Send {
    fn load(&self, config: &Config) -> io::Result<Option<Zeroizing<String>>>;
    fn save(&self, config: &Config, value: &str) -> io::Result<()>;
    fn delete(&self, config: &Config) -> io::Result<()>;
}

pub(crate) struct Platform;

/// Secret Service's default label includes the opaque lookup username. Change
/// only presentation metadata, leaving service/username lookup attributes intact.
/// Cosmetic failures must never prevent saving or using valid credentials.
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
))]
pub(crate) fn set_label(entry: &keyring::Entry, config: &Config, purpose: &str) {
    #[cfg(target_os = "linux")]
    {
        let label = display_label(config, purpose);
        let _ = entry.update_attributes(&std::collections::HashMap::from([(
            "label",
            label.as_str(),
        )]));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (entry, config, purpose);
}

#[cfg(target_os = "linux")]
fn display_label(config: &Config, purpose: &str) -> String {
    let server = reqwest::Url::parse(&config.server)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?;
            let port = url
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            Some(format!("{host}{port}{}", url.path().trim_end_matches('/')))
        })
        .unwrap_or_else(|| "configured server".into());
    format!("Paper Trail — {purpose} ({server})")
}

pub(crate) struct Remembered;

impl Remembered {
    pub fn encode(
        account: &Account,
        vault: &Vault,
        passphrase: &str,
    ) -> io::Result<Zeroizing<String>> {
        // Serialize a borrowed password so the serializer never creates an
        // unprotected owned copy of the secret.
        #[derive(Serialize)]
        struct Record<'a> {
            account: &'a Account,
            vault_id: &'a str,
            generation: u64,
            passphrase: &'a str,
        }
        serde_json::to_string(&Record {
            account,
            vault_id: &vault.id,
            generation: vault.generation,
            passphrase,
        })
        .map(Zeroizing::new)
        .map_err(|_| error("Could not encode remembered vault credentials"))
    }

    pub fn decode(value: &str, account: &Account, vault: &Vault) -> io::Result<Zeroizing<String>> {
        fn password<'de, D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Zeroizing<String>, D::Error> {
            String::deserialize(deserializer).map(Zeroizing::new)
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Record {
            account: Account,
            vault_id: String,
            generation: u64,
            #[serde(deserialize_with = "password")]
            passphrase: Zeroizing<String>,
        }
        let record: Record = serde_json::from_str(value)
            .map_err(|_| error("Invalid saved vault password; unlock manually"))?;
        if record.account != *account
            || record.vault_id != vault.id
            || record.generation != vault.generation
        {
            return Err(error(
                "Saved vault password is for a different account or vault version; unlock manually",
            ));
        }
        Ok(record.passphrase)
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
))]
fn entry(config: &Config) -> io::Result<keyring::Entry> {
    let id = papertrail_crypto::digest(
        format!("{}\n{}\n{}", config.server, config.issuer, config.client_id).as_bytes(),
    );
    keyring::Entry::new("io.github.maennlse.paper-trail.sync.vault", &id)
        .map_err(|_| error("Secure vault password storage unavailable"))
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
))]
impl Store for Platform {
    fn load(&self, config: &Config) -> io::Result<Option<Zeroizing<String>>> {
        let entry = entry(config)?;
        match entry.get_password() {
            Ok(value) => {
                let value = Zeroizing::new(value);
                set_label(&entry, config, "Encryption password");
                Ok(Some(value))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(error("Cannot access saved vault password; unlock the keyring or unlock the vault manually")),
        }
    }
    fn save(&self, config: &Config, value: &str) -> io::Result<()> {
        let entry = entry(config)?;
        entry.set_password(value).map_err(|_| {
            error("Could not remember vault password; unlocked for this session only")
        })?;
        set_label(&entry, config, "Encryption password");
        Ok(())
    }
    fn delete(&self, config: &Config) -> io::Result<()> {
        match entry(config)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(error("Could not remove saved vault password")),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod label_tests {
    use super::*;

    #[test]
    fn labels_explain_purpose_without_lookup_ids_or_url_credentials() {
        let config = Config {
            server: "https://sync.example".into(),
            ..Config::default()
        };
        assert_eq!(
            display_label(&config, "Sync login"),
            "Paper Trail — Sync login (sync.example)"
        );
        assert_eq!(
            display_label(&config, "Encryption password"),
            "Paper Trail — Encryption password (sync.example)"
        );
        let config = Config {
            server: "https://user:password@sync.example:8443/team/?token=private#private".into(),
            ..Config::default()
        };
        assert_eq!(
            display_label(&config, "Sync login"),
            "Paper Trail — Sync login (sync.example:8443/team)"
        );
        let config = Config {
            server: "invalid".into(),
            ..Config::default()
        };
        assert_eq!(
            display_label(&config, "Sync login"),
            "Paper Trail — Sync login (configured server)"
        );
    }

    #[test]
    fn updating_a_label_preserves_secret_and_lookup_attributes() {
        use keyring::credential::CredentialApi;
        use std::{any::Any, collections::HashMap, sync::Mutex};
        struct Credential(Mutex<HashMap<String, String>>);
        impl CredentialApi for Credential {
            fn set_secret(&self, _: &[u8]) -> keyring::Result<()> {
                panic!("Renaming must not rewrite secrets")
            }
            fn get_secret(&self) -> keyring::Result<Vec<u8>> {
                panic!("Renaming must not read secrets")
            }
            fn delete_credential(&self) -> keyring::Result<()> {
                panic!("Renaming must not delete secrets")
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn update_attributes(&self, attributes: &HashMap<&str, &str>) -> keyring::Result<()> {
                assert_eq!(attributes.len(), 1);
                assert!(attributes.contains_key("label"));
                self.0.lock().unwrap().extend(
                    attributes
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.to_string())),
                );
                Ok(())
            }
        }
        let entry =
            keyring::Entry::new_with_credential(Box::new(Credential(Mutex::new(HashMap::from([
                (
                    "service".into(),
                    "io.github.maennlse.paper-trail.sync".into(),
                ),
                ("username".into(), "opaque-lookup-id".into()),
                ("label".into(), "old cryptic label".into()),
            ])))));
        let config = Config {
            server: "https://sync.example".into(),
            ..Config::default()
        };
        set_label(&entry, &config, "Sync login");
        let credential = entry.get_credential().downcast_ref::<Credential>().unwrap();
        let attributes = credential.0.lock().unwrap();
        assert_eq!(
            attributes["label"],
            "Paper Trail — Sync login (sync.example)"
        );
        assert_eq!(attributes["username"], "opaque-lookup-id");
        assert_eq!(attributes["service"], "io.github.maennlse.paper-trail.sync");
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
)))]
impl Store for Platform {
    fn load(&self, _: &Config) -> io::Result<Option<Zeroizing<String>>> {
        Err(error("Secure vault password storage unavailable"))
    }
    fn save(&self, _: &Config, _: &str) -> io::Result<()> {
        Err(error("Secure vault password storage unavailable"))
    }
    fn delete(&self, _: &Config) -> io::Result<()> {
        Ok(())
    }
}
