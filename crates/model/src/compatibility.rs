//! Compatibility facts shared by format readers, save policy and front ends.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompatibilityEffect {
    Omitted,
    Static,
    ReferenceUnadjusted,
    EncryptionRemoved,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Editability {
    Editable,
    PreservedOnly,
    Unavailable,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityIssue {
    pub code: String,
    pub feature: String,
    pub location: Option<String>,
    pub effect: CompatibilityEffect,
    pub editability: Editability,
    pub description: String,
}

impl CompatibilityIssue {
    pub fn new(
        feature: impl Into<String>,
        location: Option<String>,
        effect: CompatibilityEffect,
        editability: Editability,
        description: impl Into<String>,
    ) -> Self {
        let code = match effect {
            CompatibilityEffect::Omitted => "feature.omitted",
            CompatibilityEffect::Static => "feature.static",
            CompatibilityEffect::ReferenceUnadjusted => "reference.unadjusted",
            CompatibilityEffect::EncryptionRemoved => "encryption.removed",
            CompatibilityEffect::Unknown => "package.unknown",
        };
        Self { code: code.into(), feature: feature.into(), location, effect, editability, description: description.into() }
    }
}
