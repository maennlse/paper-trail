// SPDX-License-Identifier: GPL-3.0-or-later
//! Portable v1 envelopes. Never add Debug/Serialize to secret key types.
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const FORMAT: u32 = 1;
pub const FOLDER_FORMAT: u32 = 2;
pub const MAX_BLOB: usize = 16 * 1024 * 1024;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Unsupported encryption or KDF format")]
    Version,
    #[error("Invalid encrypted data or KDF parameters")]
    Format,
    #[error("Could not authenticate encrypted data (wrong key/passphrase or altered data)")]
    Authentication,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Secret(pub [u8; 32]);
impl Secret {
    pub fn random() -> Self {
        let mut bytes = [0; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }
    pub fn encode(&self) -> Zeroizing<String> {
        Zeroizing::new(B64.encode(self.0))
    }
    pub fn decode(s: &str) -> Result<Self> {
        let bytes = Zeroizing::new(B64.decode(s.trim()).map_err(|_| Error::Format)?);
        Ok(Self(
            bytes.as_slice().try_into().map_err(|_| Error::Format)?,
        ))
    }
}
pub fn opaque_id() -> String {
    let mut bytes = [0; 16];
    OsRng.fill_bytes(&mut bytes);
    B64.encode(bytes)
}
pub fn valid_id(s: &str) -> bool {
    B64.decode(s).is_ok_and(|b| b.len() == 16) && s.len() == 22
}
pub fn digest(bytes: &[u8]) -> String {
    B64.encode(Sha256::digest(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub iss: String,
    pub sub: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Blob {
    pub version: u32,
    pub nonce: String,
    pub ciphertext: String,
}
impl Blob {
    pub fn validate(&self) -> Result<()> {
        if self.version != FORMAT {
            return Err(Error::Version);
        }
        if B64.decode(&self.nonce).map_err(|_| Error::Format)?.len() != 24
            || self.ciphertext.len() > MAX_BLOB * 4 / 3 + 64
        {
            return Err(Error::Format);
        }
        let c = B64.decode(&self.ciphertext).map_err(|_| Error::Format)?;
        if c.len() < 16 || c.len() > MAX_BLOB + 16 {
            return Err(Error::Format);
        }
        Ok(())
    }
}
fn encrypt(key: &Secret, aad: &[u8], plaintext: &[u8]) -> Result<Blob> {
    if plaintext.len() > MAX_BLOB {
        return Err(Error::Format);
    }
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = XChaCha20Poly1305::new((&key.0).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| Error::Authentication)?;
    Ok(Blob {
        version: FORMAT,
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ciphertext),
    })
}
fn decrypt(key: &Secret, aad: &[u8], blob: &Blob) -> Result<Zeroizing<Vec<u8>>> {
    blob.validate()?;
    let nonce = B64.decode(&blob.nonce).map_err(|_| Error::Format)?;
    let ciphertext = B64.decode(&blob.ciphertext).map_err(|_| Error::Format)?;
    XChaCha20Poly1305::new((&key.0).into())
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Error::Authentication)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Kdf {
    pub version: u32,
    pub algorithm: String,
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
    pub salt: String,
}
impl Kdf {
    pub fn fresh() -> Self {
        let mut salt = [0; 16];
        OsRng.fill_bytes(&mut salt);
        Self {
            version: 19,
            algorithm: "argon2id".into(),
            memory_kib: 65536,
            passes: 3,
            lanes: 4,
            salt: B64.encode(salt),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 19 || self.algorithm != "argon2id" {
            return Err(Error::Version);
        }
        if !(65536..=262144).contains(&self.memory_kib)
            || !(3..=10).contains(&self.passes)
            || !(1..=4).contains(&self.lanes)
            || B64.decode(&self.salt).map_err(|_| Error::Format)?.len() != 16
        {
            return Err(Error::Format);
        }
        Ok(())
    }
    fn derive(&self, passphrase: &str) -> Result<Secret> {
        self.validate()?;
        let salt = B64.decode(&self.salt).map_err(|_| Error::Format)?;
        let params = Params::new(self.memory_kib, self.passes, self.lanes, Some(32))
            .map_err(|_| Error::Format)?;
        let mut key = Secret([0; 32]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(passphrase.as_bytes(), &salt, &mut key.0)
            .map_err(|_| Error::Format)?;
        Ok(key)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Vault {
    pub version: u32,
    pub id: String,
    pub generation: u64,
    pub kdf: Kdf,
    pub passphrase: Blob,
    pub recovery: Blob,
}
impl Vault {
    fn aad(&self, account: &Account, purpose: &str) -> Vec<u8> {
        // Fixed-order JSON arrays, not maps; no cross-language key-order dependency.
        serde_json::to_vec(&(
            "papertrail/key",
            FORMAT,
            account,
            &self.id,
            self.generation,
            purpose,
            &self.kdf,
        ))
        .expect("serializable")
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != FORMAT {
            return Err(Error::Version);
        }
        if !valid_id(&self.id) || self.generation == 0 {
            return Err(Error::Format);
        }
        self.kdf.validate()?;
        self.passphrase.validate()?;
        self.recovery.validate()
    }
    pub fn create(account: &Account, passphrase: &str) -> Result<(Self, Secret, Secret)> {
        let key = Secret::random();
        let recovery = Secret::random();
        let empty = Blob {
            version: FORMAT,
            nonce: String::new(),
            ciphertext: String::new(),
        };
        let mut v = Self {
            version: FORMAT,
            id: opaque_id(),
            generation: 1,
            kdf: Kdf::fresh(),
            passphrase: empty.clone(),
            recovery: empty,
        };
        v.wrap(account, passphrase, &key, &recovery)?;
        Ok((v, key, recovery))
    }
    fn wrap(
        &mut self,
        account: &Account,
        passphrase: &str,
        key: &Secret,
        recovery: &Secret,
    ) -> Result<()> {
        self.passphrase = encrypt(
            &self.kdf.derive(passphrase)?,
            &self.aad(account, "passphrase"),
            &key.0,
        )?;
        self.recovery = encrypt(recovery, &self.aad(account, "recovery"), &key.0)?;
        Ok(())
    }
    pub fn unlock(&self, account: &Account, passphrase: &str) -> Result<Secret> {
        self.validate()?;
        let bytes = decrypt(
            &self.kdf.derive(passphrase)?,
            &self.aad(account, "passphrase"),
            &self.passphrase,
        )?;
        Ok(Secret(
            bytes.as_slice().try_into().map_err(|_| Error::Format)?,
        ))
    }
    pub fn recover(&self, account: &Account, recovery: &Secret) -> Result<Secret> {
        self.validate()?;
        let bytes = decrypt(recovery, &self.aad(account, "recovery"), &self.recovery)?;
        Ok(Secret(
            bytes.as_slice().try_into().map_err(|_| Error::Format)?,
        ))
    }
    /// Both wrappers authenticate their generation. Rewrapping requires the recovery
    /// secret too; the UI rotates it and requires saving the replacement.
    pub fn rewrap(
        &self,
        account: &Account,
        passphrase: &str,
        key: &Secret,
    ) -> Result<(Self, Secret)> {
        let mut v = self.clone();
        v.generation = v.generation.checked_add(1).ok_or(Error::Format)?;
        v.kdf = Kdf::fresh();
        let recovery = Secret::random();
        v.wrap(account, passphrase, key, &recovery)?;
        Ok((v, recovery))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub version: u32,
    pub vault_id: String,
    pub note_id: String,
    pub id: String,
    pub parent: Option<String>,
    pub deleted: bool,
    pub blob: Blob,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NoteData {
    pub version: u32,
    pub filename: String,
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    pub body: String,
    pub modified_ms: i64,
    pub pinned: bool,
    pub language: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FolderData {
    pub name: String,
    pub color: String,
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FolderCatalog {
    pub version: u32,
    pub folders: std::collections::BTreeMap<String, FolderData>,
    pub legacy_notes: std::collections::BTreeMap<String, String>,
}
impl FolderCatalog {
    pub fn validate(&self) -> Result<()> {
        if self.version != FOLDER_FORMAT {
            return Err(Error::Version);
        }
        if self.folders.iter().any(|(id, f)| {
            !valid_id(id) || f.name.trim().is_empty() || f.name.len() > 1024 || f.color.len() > 64
        }) || self
            .legacy_notes
            .iter()
            .any(|(id, folder)| !valid_id(id) || !self.folders.contains_key(folder))
        {
            return Err(Error::Format);
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Record {
    Note(NoteData),
    FolderCatalog(FolderCatalog),
    DeletedNote,
}
impl Revision {
    fn aad(&self, a: &Account) -> Vec<u8> {
        serde_json::to_vec(&(
            "papertrail/note",
            self.version,
            a,
            &self.vault_id,
            &self.note_id,
            &self.id,
            &self.parent,
            self.deleted,
        ))
        .expect("serializable")
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != FORMAT && self.version != FOLDER_FORMAT {
            return Err(Error::Version);
        }
        if ![&self.vault_id, &self.note_id, &self.id]
            .iter()
            .all(|s| valid_id(s))
            || self.parent.as_ref().is_some_and(|s| !valid_id(s))
        {
            return Err(Error::Format);
        }
        self.blob.validate()
    }
    pub fn seal(
        key: &Secret,
        account: &Account,
        vault_id: &str,
        note_id: &str,
        parent: Option<String>,
        data: Option<&NoteData>,
    ) -> Result<Self> {
        if data.is_some_and(|n| n.version == FOLDER_FORMAT) {
            return Self::seal_record(
                key,
                account,
                vault_id,
                note_id,
                parent,
                &Record::Note(data.unwrap().clone()),
            );
        }
        let mut r = Self {
            version: FORMAT,
            vault_id: vault_id.into(),
            note_id: note_id.into(),
            id: opaque_id(),
            parent,
            deleted: data.is_none(),
            blob: Blob {
                version: FORMAT,
                nonce: String::new(),
                ciphertext: String::new(),
            },
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&data).map_err(|_| Error::Format)?);
        r.blob = encrypt(key, &r.aad(account), &bytes)?;
        r.validate()?;
        Ok(r)
    }
    pub fn open(&self, key: &Secret, account: &Account) -> Result<Option<NoteData>> {
        match self.open_record(key, account)? {
            Record::Note(note) => Ok(Some(note)),
            Record::DeletedNote => Ok(None),
            Record::FolderCatalog(_) => Err(Error::Format),
        }
    }
    pub fn seal_record(
        key: &Secret,
        account: &Account,
        vault_id: &str,
        note_id: &str,
        parent: Option<String>,
        record: &Record,
    ) -> Result<Self> {
        match record {
            Record::Note(n) => {
                if n.version != FOLDER_FORMAT {
                    return Err(Error::Version);
                }
                if !n.folder_id.as_ref().is_some_and(|id| valid_id(id)) || note_id == vault_id {
                    return Err(Error::Format);
                }
            }
            Record::FolderCatalog(catalog) => {
                catalog.validate()?;
                if note_id != vault_id {
                    return Err(Error::Format);
                }
            }
            Record::DeletedNote => {
                if note_id == vault_id {
                    return Err(Error::Format);
                }
            }
        }
        let mut r = Self {
            version: FOLDER_FORMAT,
            vault_id: vault_id.into(),
            note_id: note_id.into(),
            id: opaque_id(),
            parent,
            deleted: matches!(record, Record::DeletedNote),
            blob: Blob {
                version: FORMAT,
                nonce: String::new(),
                ciphertext: String::new(),
            },
        };
        let bytes = Zeroizing::new(serde_json::to_vec(record).map_err(|_| Error::Format)?);
        r.blob = encrypt(key, &r.aad(account), &bytes)?;
        r.validate()?;
        Ok(r)
    }
    pub fn open_record(&self, key: &Secret, account: &Account) -> Result<Record> {
        self.validate()?;
        let bytes = decrypt(key, &self.aad(account), &self.blob)?;
        if self.version == FOLDER_FORMAT {
            let record: Record = serde_json::from_slice(&bytes).map_err(|_| Error::Format)?;
            if self.deleted != matches!(record, Record::DeletedNote) {
                return Err(Error::Format);
            }
            match &record {
                Record::Note(note) => {
                    if note.version != FOLDER_FORMAT
                        || !note.folder_id.as_ref().is_some_and(|id| valid_id(id))
                        || self.note_id == self.vault_id
                    {
                        return Err(Error::Format);
                    }
                }
                Record::FolderCatalog(catalog) => {
                    catalog.validate()?;
                    if self.note_id != self.vault_id {
                        return Err(Error::Format);
                    }
                }
                Record::DeletedNote => {
                    if self.note_id == self.vault_id {
                        return Err(Error::Format);
                    }
                }
            }
            return Ok(record);
        }
        let data: Option<NoteData> = serde_json::from_slice(&bytes).map_err(|_| Error::Format)?;
        if self.deleted != data.is_none() {
            return Err(Error::Format);
        }
        if data.as_ref().is_some_and(|n| n.version != FORMAT) {
            return Err(Error::Version);
        }
        Ok(data.map(Record::Note).unwrap_or(Record::DeletedNote))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub sequence: u64,
    pub revision: Revision,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Changes {
    pub through: u64,
    pub next: u64,
    pub changes: Vec<Change>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decrypts_libsodium_combined_mode_vector() {
        // Produced independently by libsodium's
        // crypto_aead_xchacha20poly1305_ietf_encrypt(), key bytes 0..31.
        let key = Secret(std::array::from_fn(|i| i as u8));
        let blob = Blob {
            version: 1,
            nonce: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYX".into(),
            ciphertext: "zqN_GuLy2dxSLUruqD3FmCozQceUunSfBmn527sybzOYJsfMoZ1E0RU".into(),
        };
        assert_eq!(
            decrypt(&key, b"papertrail/test/v1", &blob)
                .unwrap()
                .as_slice(),
            b"Paper Trail compatibility"
        );
    }
    #[test]
    fn argon2_rfc9106_vector() {
        // The RFC vector additionally supplies Argon2 associated data.
        // Use ParamsBuilder to match it exactly.
        let mut builder = argon2::ParamsBuilder::new();
        builder
            .m_cost(32)
            .t_cost(3)
            .p_cost(4)
            .output_len(32)
            .data(argon2::AssociatedData::new(&[4; 12]).unwrap());
        let argon = Argon2::new_with_secret(
            &[3; 8],
            Algorithm::Argon2id,
            Version::V0x13,
            builder.build().unwrap(),
        )
        .unwrap();
        let mut output = [0; 32];
        argon
            .hash_password_into(&[1; 32], &[2; 16], &mut output)
            .unwrap();
        assert_eq!(
            output,
            [
                0x0d, 0x64, 0x0d, 0xf5, 0x8d, 0x78, 0x76, 0x6c, 0x08, 0xc0, 0x37, 0xa3, 0x4a, 0x8b,
                0x53, 0xc9, 0xd0, 0x1e, 0xf0, 0x45, 0x2d, 0x75, 0xb6, 0x5e, 0xb5, 0x25, 0x20, 0xe9,
                0x6b, 0x01, 0xe6, 0x59
            ]
        );
    }
}
