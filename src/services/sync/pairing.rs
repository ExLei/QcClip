use crypto_box::{
    PublicKey, SalsaBox, SecretKey,
    aead::{Aead, AeadCore, OsRng, rand_core::RngCore},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{
    Result,
    config::{DavConfig, PairingCode, Profile},
    fail, protocol, webdav,
};

const TTL: u64 = 300;
const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

#[derive(Clone)]
pub struct Candidate {
    pub device: PairingCode,
    pub(super) invitation: Option<Invitation>,
    pub(super) host: bool,
    pub(super) dav: Option<DavConfig>,
}

impl Candidate {
    pub fn manual(device: PairingCode) -> Self {
        Self {
            device,
            invitation: None,
            host: false,
            dav: None,
        }
    }
    pub fn verification(&self, local: &Profile) -> String {
        let mut keys = [local.key, self.device.profile.key];
        keys.sort();
        let mut hash = Sha256::new();
        hash.update(b"QcClip/pair-verification/v1");
        hash.update(keys[0]);
        hash.update(keys[1]);
        let bytes = hash.finalize();
        let number = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
        format!("{:03} {:03}", number / 1000, number % 1000)
    }
    pub fn remote(&self) -> bool {
        self.invitation.is_some()
    }
    pub fn hosting(&self) -> bool {
        self.host
    }
}

#[derive(Clone)]
pub struct Invitation {
    code: Zeroizing<String>,
    offer: Offer,
}

#[derive(Clone)]
pub struct InvitationStatus {
    pub code: String,
    pub expires: u64,
    pub candidate: Option<Candidate>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Offer {
    profile: Profile,
    expires: u64,
    challenge: String,
}

#[derive(Serialize, Deserialize)]
struct Acceptance {
    host: String,
    challenge: String,
    profile: Profile,
}

#[derive(Serialize, Deserialize)]
struct Sealed {
    key: [u8; 32],
    nonce: [u8; 24],
    #[serde(with = "protocol::bytes")]
    ciphertext: Vec<u8>,
}

fn normalize(code: &str) -> Result<Zeroizing<String>> {
    if code.len() > 64 {
        return fail("连接码格式无效，请输入四组字符");
    }
    let code = Zeroizing::new(
        code.chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>(),
    );
    if code.len() != 16 || !code.bytes().all(|c| ALPHABET.contains(&c)) {
        return fail("连接码格式无效，请输入四组字符");
    }
    Ok(code)
}

fn secret(code: &str, purpose: &str) -> SecretKey {
    let mut hash = Sha256::new();
    hash.update(b"QcClip/invitation/v1/");
    hash.update(purpose);
    hash.update(code);
    SecretKey::from(<[u8; 32]>::from(hash.finalize()))
}

fn path(code: &str, kind: &str) -> String {
    let hash = protocol::hash(format!("QcClip/invitation/route/v1/{code}").as_bytes());
    let route: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("pairing/{route}-{kind}")
}

pub(super) fn seal<T: Serialize>(code: &str, kind: &str, payload: &T) -> Result<Vec<u8>> {
    let recipient = secret(code, kind);
    let ephemeral = SecretKey::generate(&mut OsRng);
    let cipher = SalsaBox::new(&recipient.public_key(), &ephemeral);
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let plain = Zeroizing::new(serde_json::to_vec(payload)?);
    let ciphertext = cipher
        .encrypt(&nonce, plain.as_slice())
        .map_err(|_| super::Error::Message("连接资料加密失败".into()))?;
    Ok(serde_json::to_vec(&Sealed {
        key: *ephemeral.public_key().as_bytes(),
        nonce: nonce.into(),
        ciphertext,
    })?)
}

pub(super) fn open<T: DeserializeOwned>(code: &str, kind: &str, bytes: &[u8]) -> Result<T> {
    let envelope: Sealed = serde_json::from_slice(bytes)?;
    let cipher = SalsaBox::new(&PublicKey::from(envelope.key), &secret(code, kind));
    let plain = Zeroizing::new(
        cipher
            .decrypt((&envelope.nonce).into(), envelope.ciphertext.as_slice())
            .map_err(|_| super::Error::Message("连接资料验证失败，请重新生成连接码".into()))?,
    );
    Ok(serde_json::from_slice(&plain)?)
}

impl Invitation {
    pub fn create(client: &webdav::Client, profile: Profile) -> Result<Self> {
        let mut random = [0u8; 16];
        OsRng.fill_bytes(&mut random);
        let code = Zeroizing::new(
            random
                .iter()
                .map(|b| ALPHABET[(b & 31) as usize] as char)
                .collect::<String>(),
        );
        let offer = Offer {
            profile,
            expires: protocol::now() + TTL,
            challenge: uuid::Uuid::new_v4().to_string(),
        };
        let invitation = Self { code, offer };
        client.initialize()?;
        client.mkdir("pairing/")?;
        client.put(
            &path(&invitation.code, "offer"),
            &seal(&invitation.code, "offer", &invitation.offer)?,
        )?;
        Ok(invitation)
    }
    pub fn join(client: &webdav::Client, raw: &str) -> Result<Candidate> {
        let code = normalize(raw)?;
        let bytes = client.get(&path(&code, "offer"), 16384)?.ok_or_else(|| {
            super::Error::Message(
                "未找到连接码，请确认双方使用同一 WebDAV 目录，或在另一台设备重新生成".into(),
            )
        })?;
        let offer: Offer = open(&code, "offer", &bytes)?;
        offer.profile.validate()?;
        let invitation = Self { code, offer };
        invitation.validate()?;
        Ok(Candidate {
            device: PairingCode {
                profile: invitation.offer.profile.clone(),
                address: String::new(),
            },
            invitation: Some(invitation),
            host: false,
            dav: None,
        })
    }
    fn validate(&self) -> Result<()> {
        if self.offer.expires <= protocol::now()
            || self.offer.expires > protocol::now() + TTL
            || uuid::Uuid::parse_str(&self.offer.challenge).is_err()
        {
            return fail("连接码已过期，请在另一台设备重新生成");
        }
        Ok(())
    }
    pub fn status(&self, client: Option<&webdav::Client>) -> Result<InvitationStatus> {
        self.validate()?;
        let mut candidate = None;
        if let Some(client) = client
            && let Some(bytes) = client.get(&path(&self.code, "reply"), 16384)?
        {
            let acceptance: Acceptance = open(&self.code, "reply", &bytes)?;
            acceptance.profile.validate()?;
            if acceptance.host != self.offer.profile.id
                || acceptance.challenge != self.offer.challenge
                || acceptance.profile.id == self.offer.profile.id
                || acceptance.profile.key == self.offer.profile.key
            {
                return fail("连接请求与当前设备不匹配");
            }
            candidate = Some(Candidate {
                device: PairingCode {
                    profile: acceptance.profile,
                    address: String::new(),
                },
                invitation: Some(self.clone()),
                host: true,
                dav: None,
            });
        }
        Ok(InvitationStatus {
            candidate,
            ..self.display()
        })
    }
    pub fn display(&self) -> InvitationStatus {
        InvitationStatus {
            code: self
                .code
                .as_bytes()
                .chunks(4)
                .map(|s| std::str::from_utf8(s).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("-"),
            expires: self.offer.expires,
            candidate: None,
        }
    }
    pub fn respond(&self, client: &webdav::Client, profile: Profile) -> Result<()> {
        self.validate()?;
        let bytes = client
            .get(&path(&self.code, "offer"), 16384)?
            .ok_or_else(|| super::Error::Message("连接码已取消，请在另一台设备重新生成".into()))?;
        let current: Offer = open(&self.code, "offer", &bytes)?;
        if current.challenge != self.offer.challenge
            || current.profile != self.offer.profile
            || current.expires != self.offer.expires
        {
            return fail("连接资料发生变化，请重新检查设备");
        }
        let acceptance = Acceptance {
            host: self.offer.profile.id.clone(),
            challenge: self.offer.challenge.clone(),
            profile,
        };
        client.put(
            &path(&self.code, "reply"),
            &seal(&self.code, "reply", &acceptance)?,
        )
    }
    pub fn matches(&self, other: &Self) -> bool {
        self.offer.challenge == other.offer.challenge && self.code == other.code
    }
    pub fn expired(&self) -> bool {
        self.offer.expires <= protocol::now()
    }
    pub fn cleanup(&self, client: &webdav::Client) -> Result<()> {
        client.delete(&path(&self.code, "offer"))?;
        client.delete(&path(&self.code, "reply"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_codes_normalize_and_authenticate_encrypted_identity() {
        let code = normalize("2345-6789-abcd-efgh").unwrap();
        assert_eq!(&*code, "23456789ABCDEFGH");
        assert!(normalize("123456").is_err());
        let profile = super::super::config::Config::fresh().profile;
        let bytes = seal(&code, "offer", &profile).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(&profile.id));
        assert_eq!(open::<Profile>(&code, "offer", &bytes).unwrap(), profile);
        assert!(open::<Profile>(&code, "reply", &bytes).is_err());
        assert!(open::<Profile>("ABCDEFGH23456789", "offer", &bytes).is_err());
        let mut tampered = bytes;
        tampered[20] ^= 1;
        assert!(open::<Profile>(&code, "offer", &tampered).is_err());
    }
    #[test]
    fn expired_or_future_invitation_is_rejected() {
        let mut invitation = Invitation {
            code: normalize("23456789ABCDEFGH").unwrap(),
            offer: Offer {
                profile: super::super::config::Config::fresh().profile,
                expires: protocol::now() - 1,
                challenge: uuid::Uuid::new_v4().to_string(),
            },
        };
        assert!(invitation.validate().is_err());
        invitation.offer.expires = protocol::now() + TTL + 10;
        assert!(invitation.validate().is_err());
    }
}
