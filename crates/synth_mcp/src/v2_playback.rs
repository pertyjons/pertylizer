//! Last GUI observation of the experimental V2 output. This is not a project revision.
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
#[error("invalid V2 observation identity")]
pub struct InvalidObservationIdentity;

macro_rules! identity {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(try_from = "String", into = "String")]
        #[must_use]
        pub struct $name(String);
        impl $name {
            pub fn new(value: std::num::NonZeroU64) -> Self {
                Self(format!("{}{}", $prefix, value))
            }
        }
        impl TryFrom<String> for $name {
            type Error = InvalidObservationIdentity;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                let digits = value
                    .strip_prefix($prefix)
                    .ok_or(InvalidObservationIdentity)?;
                let number = digits
                    .parse::<std::num::NonZeroU64>()
                    .map_err(|_| InvalidObservationIdentity)?;
                if digits != number.to_string() {
                    return Err(InvalidObservationIdentity);
                }
                Ok(Self(value))
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
identity!(SwitchScope, "switch-");
identity!(RenderedInstall, "install-");
identity!(RenderedPlan, "plan-");
identity!(PublicationId, "publication-");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[must_use]
pub struct ServiceInstance(String);
impl ServiceInstance {
    pub fn new(nonce: [u64; 2]) -> Self {
        Self(format!("instance-{:016x}{:016x}", nonce[0], nonce[1]))
    }
}
impl TryFrom<String> for ServiceInstance {
    type Error = InvalidObservationIdentity;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let digits = value
            .strip_prefix("instance-")
            .ok_or(InvalidObservationIdentity)?;
        if !is_hex(digits, 32) {
            return Err(InvalidObservationIdentity);
        }
        Ok(Self(value))
    }
}
impl From<ServiceInstance> for String {
    fn from(value: ServiceInstance) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[must_use]
pub struct LoweringDigest(String);
impl TryFrom<String> for LoweringDigest {
    type Error = InvalidObservationIdentity;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !is_hex(&value, 64) {
            return Err(InvalidObservationIdentity);
        }
        Ok(Self(value))
    }
}
impl From<LoweringDigest> for String {
    fn from(value: LoweringDigest) -> Self {
        value.0
    }
}
fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum PlaybackSchema {
    #[serde(rename = "pertylizer.v2-playback/1")]
    V1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum LoweringProjection {
    #[serde(rename = "sha256:pertylizer.v2-lowering-input/1")]
    V1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    NotCompiled,
    NoGui,
    NotPublished,
    NoPublisher,
    InvalidObservation,
    IdentityExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudiblePlan {
    pub install: RenderedInstall,
    pub plan: RenderedPlan,
    pub projection: LoweringProjection,
    /// Opaque equality token for the captured lowering input, including cosmetic fields.
    /// Compare only within one service instance. It is neither a hash of current
    /// edits nor reproducible from a saved project.
    pub digest: LoweringDigest,
    pub omissions: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlaybackState {
    Inactive {
        releasing: bool,
    },
    Refused {
        scope: SwitchScope,
    },
    Active {
        scope: SwitchScope,
        requested_playing: bool,
        edit_pending: bool,
        #[serde(deserialize_with = "required_option")]
        #[schemars(required)]
        audible: Option<AudiblePlan>,
    },
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlaybackObservation {
    Unavailable {
        reason: UnavailableReason,
    },
    Published {
        instance: ServiceInstance,
        publication: PublicationId,
        age_ms: u64,
        state: PlaybackState,
    },
}
/// The single owned GUI view used by both its badge and this read-only MCP reply.
/// A published plan completed a successful callback; age measures the GUI poll,
/// not a callback heartbeat or device liveness. Queued plans are never audible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct V2PlaybackStatus {
    pub schema: PlaybackSchema,
    pub observation: PlaybackObservation,
}
impl V2PlaybackStatus {
    #[must_use]
    pub fn unavailable(reason: UnavailableReason) -> Self {
        Self {
            schema: PlaybackSchema::V1,
            observation: PlaybackObservation::Unavailable { reason },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn active() -> Value {
        json!({"schema":"pertylizer.v2-playback/1", "observation": {
            "status":"published", "instance":"instance-00000000000000010000000000000002",
            "publication":"publication-18446744073709551615", "age_ms":0,
            "state": {"mode":"active", "scope":"switch-1", "requested_playing":true,
                "edit_pending":false, "audible": {
                    "install":"install-9007199254740993", "plan":"plan-18446744073709551615",
                    "projection":"sha256:pertylizer.v2-lowering-input/1",
                    "digest":"a".repeat(64), "omissions":[]
                }}
        }})
    }
    #[test]
    fn round_trip_preserves_large_identities_as_strings_and_explicit_null() {
        for mut value in [active(), active()] {
            if value["observation"]["state"]["audible"].is_object() {
                let status: V2PlaybackStatus = serde_json::from_value(value.clone()).unwrap();
                assert_eq!(serde_json::to_value(status).unwrap(), value);
            }
            value["observation"]["state"]["audible"] = Value::Null;
            let status: V2PlaybackStatus = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(status).unwrap(), value);
        }
        for value in [
            json!({"schema":"pertylizer.v2-playback/1","observation":{"status":"unavailable","reason":"not_compiled"}}),
            json!({"schema":"pertylizer.v2-playback/1","observation":{"status":"published","instance":"instance-00000000000000010000000000000002","publication":"publication-1","age_ms":4,"state":{"mode":"inactive","releasing":true}}}),
            json!({"schema":"pertylizer.v2-playback/1","observation":{"status":"published","instance":"instance-00000000000000010000000000000002","publication":"publication-1","age_ms":4,"state":{"mode":"refused","scope":"switch-2"}}}),
        ] {
            let status: V2PlaybackStatus = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(status).unwrap(), value);
        }
    }
    #[test]
    fn every_object_rejects_unknown_and_missing_fields() {
        for pointer in [
            "",
            "/observation",
            "/observation/state",
            "/observation/state/audible",
        ] {
            let mut value = active();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("typo".to_owned(), json!(true));
            assert!(
                serde_json::from_value::<V2PlaybackStatus>(value).is_err(),
                "{pointer}"
            );
            let keys = active()
                .pointer(pointer)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            for key in keys {
                let mut value = active();
                value
                    .pointer_mut(pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(&key);
                assert!(
                    serde_json::from_value::<V2PlaybackStatus>(value).is_err(),
                    "{pointer}/{key}"
                );
            }
        }
    }
    #[test]
    fn malformed_identities_versions_and_numeric_values_are_rejected() {
        for (pointer, replacement) in [
            ("/schema", json!("pertylizer.v2-playback/0")),
            ("/observation/instance", json!("instance-deadbeef")),
            ("/observation/publication", json!("publication-0")),
            ("/observation/publication", json!("publication-01")),
            ("/observation/age_ms", json!(-1)),
            ("/observation/state/scope", json!("plan-1")),
            ("/observation/state/audible/install", json!(42)),
            (
                "/observation/state/audible/plan",
                json!("plan-18446744073709551616"),
            ),
            ("/observation/state/audible/digest", json!("A".repeat(64))),
            (
                "/observation/state/audible/projection",
                json!("sha256:canonical-project/1"),
            ),
        ] {
            let mut value = active();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                serde_json::from_value::<V2PlaybackStatus>(value).is_err(),
                "{pointer}"
            );
        }
    }
}
