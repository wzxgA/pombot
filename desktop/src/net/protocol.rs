use anyhow::{Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::{
    contract::{Envelope, Error, Type},
    store::now_millis,
};

pub const MAX_SKEW_MILLIS: i64 = 120_000;

pub fn new_envelope<T: Serialize>(kind: Type, payload: T) -> Result<Envelope> {
    Ok(Envelope {
        v: 1,
        id: Uuid::new_v4().to_string(),
        envelope_type: kind,
        ts: now_millis(),
        nonce: Uuid::new_v4().to_string(),
        payload: Some(serde_json::to_value(payload)?),
    })
}

pub fn error_envelope(message: impl Into<String>, request_id: Option<String>) -> Envelope {
    new_envelope(
        Type::Error,
        Error {
            code: None,
            message: message.into(),
            request_id,
        },
    )
    .unwrap_or_else(|_| Envelope {
        v: 1,
        id: Uuid::new_v4().to_string(),
        envelope_type: Type::Error,
        ts: now_millis(),
        nonce: Uuid::new_v4().to_string(),
        payload: None,
    })
}

pub fn payload<T: DeserializeOwned>(envelope: &Envelope) -> Result<T> {
    let value = envelope.payload.clone().unwrap_or(serde_json::Value::Null);
    serde_json::from_value(value).map_err(Into::into)
}

pub fn validate(envelope: &Envelope) -> Result<()> {
    if envelope.v != 1 {
        bail!("不支持的协议版本 {}", envelope.v);
    }
    Uuid::parse_str(&envelope.id)?;
    if envelope.nonce.len() < 8 || envelope.nonce.len() > 128 {
        bail!("nonce 长度无效");
    }
    if (now_millis() - envelope.ts).abs() > MAX_SKEW_MILLIS {
        bail!("消息时间戳超出允许窗口，请校准设备时间");
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct EmptyPayload {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_old_and_replayed_shaped_messages() {
        let mut envelope = new_envelope(Type::Ping, serde_json::json!({})).expect("envelope");
        assert!(validate(&envelope).is_ok());
        envelope.ts -= MAX_SKEW_MILLIS + 1;
        assert!(validate(&envelope).is_err());
    }
}
