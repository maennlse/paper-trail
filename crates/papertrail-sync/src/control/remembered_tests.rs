// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use papertrail_crypto::Account;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Default)]
struct MemoryStore {
    value: Arc<Mutex<Option<Zeroizing<String>>>>,
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
}
impl MemoryStore {
    fn check(&self) -> io::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            Err(error("Test keyring unavailable"))
        } else {
            Ok(())
        }
    }
}
impl credentials::Store for MemoryStore {
    fn load(&self, _: &Config) -> io::Result<Option<Zeroizing<String>>> {
        self.check()?;
        Ok(self.value.lock().unwrap().clone())
    }
    fn save(&self, _: &Config, value: &str) -> io::Result<()> {
        self.check()?;
        *self.value.lock().unwrap() = Some(Zeroizing::new(value.to_owned()));
        Ok(())
    }
    fn delete(&self, _: &Config) -> io::Result<()> {
        self.check()?;
        *self.value.lock().unwrap() = None;
        Ok(())
    }
}

const PASSWORD: &str = "long password with \"quotes\", \\slashes and 日本語";

struct Fixture {
    _temp: tempfile::TempDir,
    service: Service,
    config: Config,
    account: Account,
    vault: Vault,
    store: MemoryStore,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new(&temp.path().join("settings.json"));
        let config = Config {
            server: "https://sync.example".into(),
            issuer: "https://issuer.example".into(),
            client_id: "native".into(),
            folders: vec![temp.path().join("notes")],
            ..Config::default()
        };
        service.configure(config.clone()).unwrap();
        let account = Account {
            iss: config.issuer.clone(),
            sub: "alice".into(),
        };
        let (vault, _, _) = Vault::create(&account, PASSWORD).unwrap();
        let store = MemoryStore::default();
        service.control.lock().unwrap().secrets = Box::new(store.clone());
        Self {
            _temp: temp,
            service,
            config,
            account,
            vault,
            store,
        }
    }
    fn engine(&self) -> Engine {
        Engine::open(self.service.root.join("state.json")).unwrap()
    }
}

#[test]
fn passwords_round_trip_but_cannot_cross_accounts_vaults_or_generations() {
    let f = Fixture::new();
    let encoded = Remembered::encode(&f.account, &f.vault, PASSWORD).unwrap();
    assert_eq!(
        Remembered::decode(&encoded, &f.account, &f.vault)
            .unwrap()
            .as_str(),
        PASSWORD
    );
    let mut other = f.account.clone();
    other.sub = "bob".into();
    assert!(Remembered::decode(&encoded, &other, &f.vault).is_err());
    let mut changed = f.vault.clone();
    changed.id = papertrail_crypto::opaque_id();
    assert!(Remembered::decode(&encoded, &f.account, &changed).is_err());
    changed = f.vault.clone();
    changed.generation += 1;
    assert!(Remembered::decode(&encoded, &f.account, &changed).is_err());
    assert!(Remembered::decode("invalid secret JSON", &f.account, &f.vault).is_err());
}

#[test]
fn only_successful_unlocks_save_and_unchecked_unlocks_forget() {
    let f = Fixture::new();
    let mut c = f.service.control.lock().unwrap();
    let mut engine = f.engine();
    assert!(c
        .unlock_vault(
            &f.account,
            &f.vault,
            &mut engine,
            "wrong password",
            Some((&f.config, true))
        )
        .is_err());
    assert_eq!(f.store.calls.load(Ordering::SeqCst), 0);
    assert!(c.key.is_none());
    engine.state.account = Some(Account {
        iss: f.account.iss.clone(),
        sub: "bob".into(),
    });
    assert!(c
        .unlock_vault(
            &f.account,
            &f.vault,
            &mut engine,
            PASSWORD,
            Some((&f.config, true))
        )
        .is_err());
    assert_eq!(f.store.calls.load(Ordering::SeqCst), 0);
    engine.state.account = None;
    c.unlock_vault(
        &f.account,
        &f.vault,
        &mut engine,
        PASSWORD,
        Some((&f.config, true)),
    )
    .unwrap();
    assert!(c.key.is_some());
    let value = f.store.value.lock().unwrap();
    assert_eq!(
        Remembered::decode(value.as_ref().unwrap(), &f.account, &f.vault)
            .unwrap()
            .as_str(),
        PASSWORD
    );
    drop(value);
    let config = std::fs::read_to_string(f.service.root.join("config.json")).unwrap();
    let journal = std::fs::read_to_string(f.service.root.join("state.json")).unwrap();
    assert!(!config.contains("long password"));
    assert!(!journal.contains("long password"));
    c.unlock_vault(
        &f.account,
        &f.vault,
        &mut engine,
        PASSWORD,
        Some((&f.config, false)),
    )
    .unwrap();
    assert!(f.store.value.lock().unwrap().is_none());
}

#[test]
fn remembered_unlock_waits_for_recovery_confirmation_and_publication() {
    let f = Fixture::new();
    let mut c = f.service.control.lock().unwrap();
    let mut engine = f.engine();
    let key = f.vault.unlock(&f.account, PASSWORD).unwrap();
    c.pending = Some(PendingVault {
        vault: f.vault.clone(),
        key,
        expected: 0,
        passphrase: Some(Zeroizing::new(PASSWORD.into())),
    });
    assert!(c
        .confirm_recovery(&f.config, &f.account, &mut engine, |_, _| Err(error(
            "offline"
        )))
        .is_err());
    assert!(c.pending.is_some());
    assert!(
        c.known_vault.is_none(),
        "failed publication is not a created vault"
    );
    assert!(f.store.value.lock().unwrap().is_none());
    c.confirm_recovery(&f.config, &f.account, &mut engine, |_, expected| {
        assert_eq!(expected, 0);
        Ok(())
    })
    .unwrap();
    assert!(c.pending.is_none());
    assert_eq!(c.known_vault, Some((f.account.clone(), true)));
    let new_password = "a different strong passphrase";
    let (updated, _) = f
        .vault
        .rewrap(&f.account, new_password, c.key.as_ref().unwrap())
        .unwrap();
    c.pending = Some(PendingVault {
        vault: updated.clone(),
        key: Secret(c.key.as_ref().unwrap().0),
        expected: f.vault.generation,
        passphrase: Some(Zeroizing::new(new_password.into())),
    });
    c.confirm_recovery(&f.config, &f.account, &mut engine, |_, expected| {
        assert_eq!(expected, f.vault.generation);
        Ok(())
    })
    .unwrap();
    c.key = None;
    assert!(c
        .restore_vault(&f.config, &f.account, &updated, &mut engine)
        .unwrap());
    assert!(c.key.is_some());
    assert!(Remembered::decode(
        f.store.value.lock().unwrap().as_ref().unwrap(),
        &f.account,
        &f.vault
    )
    .is_err());
}

#[test]
fn lock_blocks_restore_until_restart_and_cancels_pending_passwords() {
    let f = Fixture::new();
    let mut c = f.service.control.lock().unwrap();
    let mut engine = f.engine();
    c.unlock_vault(
        &f.account,
        &f.vault,
        &mut engine,
        PASSWORD,
        Some((&f.config, true)),
    )
    .unwrap();
    c.pending = Some(PendingVault {
        vault: f.vault.clone(),
        key: Secret(c.key.as_ref().unwrap().0),
        expected: 0,
        passphrase: Some(Zeroizing::new("never saved".into())),
    });
    let calls = f.store.calls.load(Ordering::SeqCst);
    assert!(!c.execute(Command::Lock).unwrap().unlocked);
    assert!(c.pending.is_none());
    assert!(!c
        .restore_vault(&f.config, &f.account, &f.vault, &mut engine)
        .unwrap());
    assert!(!c.execute(Command::Restore).unwrap().unlocked);
    assert_eq!(f.store.calls.load(Ordering::SeqCst), calls);
    // A fresh process/service starts with auto-unlock allowed.
    let fresh = Service::new(&c.settings_path);
    let mut restarted = fresh.control.lock().unwrap();
    restarted.secrets = Box::new(f.store.clone());
    assert!(restarted
        .restore_vault(&f.config, &f.account, &f.vault, &mut engine)
        .unwrap());
}

#[test]
fn unavailable_storage_keeps_manual_unlock_session_only() {
    let f = Fixture::new();
    f.store.fail.store(true, Ordering::SeqCst);
    let mut c = f.service.control.lock().unwrap();
    let mut engine = f.engine();
    let warning = c
        .unlock_vault(
            &f.account,
            &f.vault,
            &mut engine,
            PASSWORD,
            Some((&f.config, true)),
        )
        .unwrap();
    assert!(warning.unwrap().contains("keyring unavailable"));
    assert!(c.key.is_some());
    c.key = None;
    assert!(c
        .restore_vault(&f.config, &f.account, &f.vault, &mut engine)
        .is_err());
    assert!(c.key.is_none());
    c.memory_only_tokens = true;
    let calls = f.store.calls.load(Ordering::SeqCst);
    c.unlock_vault(
        &f.account,
        &f.vault,
        &mut engine,
        PASSWORD,
        Some((&f.config, true)),
    )
    .unwrap();
    c.remember_password(&f.config, &f.account, &f.vault, None)
        .unwrap();
    assert!(!c
        .restore_vault(&f.config, &f.account, &f.vault, &mut engine)
        .unwrap());
    assert_eq!(f.store.calls.load(Ordering::SeqCst), calls);
}

#[test]
fn signout_deletes_both_credentials_and_attempts_both_even_on_failure() {
    let f = Fixture::new();
    let mut c = f.service.control.lock().unwrap();
    c.remember_password(&f.config, &f.account, &f.vault, Some(PASSWORD))
        .unwrap();
    let deleted_login = AtomicBool::new(false);
    c.forget_credentials(&f.config, |_| {
        deleted_login.store(true, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    assert!(deleted_login.load(Ordering::SeqCst));
    assert!(f.store.value.lock().unwrap().is_none());
    f.store.fail.store(true, Ordering::SeqCst);
    deleted_login.store(false, Ordering::SeqCst);
    let err = c
        .forget_credentials(&f.config, |_| {
            deleted_login.store(true, Ordering::SeqCst);
            Err(error("Test login removal failed"))
        })
        .unwrap_err();
    assert!(deleted_login.load(Ordering::SeqCst));
    assert!(err.to_string().contains("keyring unavailable"));
    assert!(err.to_string().contains("login removal failed"));
    c.memory_only_tokens = true;
    let calls = f.store.calls.load(Ordering::SeqCst);
    c.forget_credentials(&f.config, |_| {
        panic!("Headless logout touched login storage")
    })
    .unwrap();
    assert_eq!(f.store.calls.load(Ordering::SeqCst), calls);
}
