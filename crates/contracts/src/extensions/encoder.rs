//! Declarations for host-approved encoder choices in the existing Video slot.
//!
//! These records can describe dropdown choices. They do not register a native
//! service, approve a publisher, grant a capability, or probe an encoder. The
//! signed component request, host grant ceiling, effective runtime grants and
//! the live `EncoderApi` table must all agree before Video may bind a choice.

use super::{ExtensionVersion, configuration::ExtensionConfigurationRef};
use crate::{Grants, encoder::VideoCodec, valid_name};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_TABLE_SIZE: u32 = 1_048_576;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderProviderDescriptor {
    pub version: ExtensionVersion,
    pub provider_id: String,
    pub component_id: String,
    /// Exact native `EncoderApi` service already bound by Video.
    pub service: String,
    pub interface_major: u32,
    pub interface_minor: u32,
    pub declared_table_size: u32,
    /// A claim only; actual encoder enumeration and open remain authoritative.
    pub codecs: BTreeSet<VideoCodec>,
    pub configuration: Option<ExtensionConfigurationRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderProviderApproval {
    pub provider_id: String,
    pub component_id: String,
    pub service: String,
    /// Host-owned ordering; a module cannot choose its own priority.
    pub priority: i16,
    pub interface_major: u32,
    pub minimum_interface_minor: u32,
    pub minimum_table_size: u32,
    pub allowed_codecs: BTreeSet<VideoCodec>,
    /// None forbids extension-owned settings for this choice.
    pub option_namespace: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderProviderRecord {
    pub descriptor: EncoderProviderDescriptor,
    pub approval: EncoderProviderApproval,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EncoderExtensionSelection {
    #[default]
    Builtin,
    Exact {
        provider_id: String,
    },
    Automatic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderProviderPolicy {
    pub selection: EncoderExtensionSelection,
    pub candidates: Vec<EncoderProviderRecord>,
}

pub struct EncoderCandidate<'a> {
    pub descriptor: &'a EncoderProviderDescriptor,
    pub approval: &'a EncoderProviderApproval,
    pub provider_grants: &'a Grants,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncoderRequirement {
    pub codec: VideoCodec,
    pub interface_major: u32,
    pub minimum_interface_minor: u32,
    /// Derived from the consumer's actual C table prefix.
    pub minimum_table_size: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderBindPlan {
    pub provider_id: String,
    pub component_id: String,
    pub service: String,
    pub interface_major: u32,
    pub minimum_interface_minor: u32,
    pub minimum_table_size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncoderPlanError {
    InvalidDescriptor,
    DuplicateProvider,
    PermissionDenied,
    NoCompatibleProvider,
}

/// Select one approved service. No probe, load, fallback or configuration write
/// occurs here. In particular, an explicit choice never silently falls back.
pub fn plan_encoder_provider(
    selection: &EncoderExtensionSelection,
    requirement: EncoderRequirement,
    candidates: &[EncoderCandidate<'_>],
    consumer_grants: &Grants,
) -> Result<Option<EncoderBindPlan>, EncoderPlanError> {
    if matches!(selection, EncoderExtensionSelection::Builtin) {
        return Ok(None);
    }
    if requirement.interface_major != 1
        || requirement.minimum_table_size == 0
        || requirement.minimum_table_size > MAX_TABLE_SIZE
    {
        return Err(EncoderPlanError::InvalidDescriptor);
    }
    let mut seen = BTreeSet::new();
    let mut compatible = Vec::new();
    for candidate in candidates {
        // Unselected identities cannot block an explicit choice. Automatic
        // selection checks every candidate before its deterministic ordering.
        if let EncoderExtensionSelection::Exact { provider_id } = selection
            && &candidate.descriptor.provider_id != provider_id
        {
            continue;
        }
        validate_pair(candidate)?;
        if !seen.insert(candidate.descriptor.provider_id.as_str()) {
            return Err(EncoderPlanError::DuplicateProvider);
        }
        let service = candidate.descriptor.service.as_str();
        if !candidate.provider_grants.native_provide.contains(service)
            || !candidate.provider_grants.provide.contains(service)
            || !consumer_grants.native_use.contains(service)
            || !consumer_grants.call.contains(service)
        {
            return Err(EncoderPlanError::PermissionDenied);
        }
        if candidate.descriptor.interface_minor < requirement.minimum_interface_minor
            || candidate.descriptor.declared_table_size < requirement.minimum_table_size
            || !candidate.descriptor.codecs.contains(&requirement.codec)
        {
            continue;
        }
        compatible.push(candidate);
    }
    compatible.sort_by(|left, right| {
        right
            .approval
            .priority
            .cmp(&left.approval.priority)
            .then_with(|| left.approval.provider_id.cmp(&right.approval.provider_id))
    });
    let selected = compatible
        .into_iter()
        .next()
        .ok_or(EncoderPlanError::NoCompatibleProvider)?;
    Ok(Some(EncoderBindPlan {
        provider_id: selected.approval.provider_id.clone(),
        component_id: selected.approval.component_id.clone(),
        service: selected.approval.service.clone(),
        interface_major: 1,
        minimum_interface_minor: requirement
            .minimum_interface_minor
            .max(selected.approval.minimum_interface_minor),
        minimum_table_size: requirement
            .minimum_table_size
            .max(selected.approval.minimum_table_size),
    }))
}

fn validate_pair(candidate: &EncoderCandidate<'_>) -> Result<(), EncoderPlanError> {
    let descriptor = candidate.descriptor;
    let approval = candidate.approval;
    if !descriptor.version.is_supported()
        || !external_id(&descriptor.provider_id)
        || !external_id(&descriptor.component_id)
        || !external_service(&descriptor.service)
        || descriptor.provider_id != approval.provider_id
        || descriptor.component_id != approval.component_id
        || descriptor.service != approval.service
        || descriptor.interface_major != 1
        || descriptor.interface_major != approval.interface_major
        || descriptor.interface_minor < approval.minimum_interface_minor
        || descriptor.declared_table_size < approval.minimum_table_size
        || descriptor.declared_table_size == 0
        || descriptor.declared_table_size > MAX_TABLE_SIZE
        || approval.minimum_table_size == 0
        || approval.minimum_table_size > MAX_TABLE_SIZE
        || descriptor.codecs.is_empty()
        || !descriptor.codecs.is_subset(&approval.allowed_codecs)
    {
        return Err(EncoderPlanError::InvalidDescriptor);
    }
    match (&descriptor.configuration, &approval.option_namespace) {
        (None, None) => Ok(()),
        (Some(configuration), Some(namespace))
            if namespace == &configuration.namespace
                && configuration.validate_for(&descriptor.component_id).is_ok() =>
        {
            Ok(())
        }
        _ => Err(EncoderPlanError::InvalidDescriptor),
    }
}

fn external_id(id: &str) -> bool {
    valid_name(id) && id.contains('.') && !id.contains('/') && !id.starts_with("amaterasu.")
}

fn external_service(service: &str) -> bool {
    service.strip_suffix("/1").is_some_and(external_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        id: &str,
        priority: i16,
    ) -> (EncoderProviderDescriptor, EncoderProviderApproval, Grants) {
        let service = format!("{id}/1");
        let codecs = BTreeSet::from([VideoCodec::H264]);
        (
            EncoderProviderDescriptor {
                version: ExtensionVersion::CURRENT,
                provider_id: id.into(),
                component_id: id.into(),
                service: service.clone(),
                interface_major: 1,
                interface_minor: 1,
                declared_table_size: 128,
                codecs: codecs.clone(),
                configuration: None,
            },
            EncoderProviderApproval {
                provider_id: id.into(),
                component_id: id.into(),
                service: service.clone(),
                priority,
                interface_major: 1,
                minimum_interface_minor: 0,
                minimum_table_size: 64,
                allowed_codecs: codecs,
                option_namespace: None,
            },
            Grants {
                native_provide: BTreeSet::from([service.clone()]),
                provide: BTreeSet::from([service]),
                ..Grants::default()
            },
        )
    }

    fn requirement() -> EncoderRequirement {
        EncoderRequirement {
            codec: VideoCodec::H264,
            interface_major: 1,
            minimum_interface_minor: 0,
            minimum_table_size: 64,
        }
    }

    #[test]
    fn exact_selection_ignores_an_ungranted_unselected_candidate() {
        let (a, aa, ag) = candidate("example.encoder-a", 1);
        let (b, ba, bg) = candidate("example.encoder-b", 2);
        let candidates = [
            EncoderCandidate {
                descriptor: &a,
                approval: &aa,
                provider_grants: &ag,
            },
            EncoderCandidate {
                descriptor: &b,
                approval: &ba,
                provider_grants: &bg,
            },
        ];
        let grants = Grants {
            native_use: BTreeSet::from([a.service.clone()]),
            call: BTreeSet::from([a.service.clone()]),
            ..Grants::default()
        };
        let plan = plan_encoder_provider(
            &EncoderExtensionSelection::Exact {
                provider_id: a.provider_id.clone(),
            },
            requirement(),
            &candidates,
            &grants,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.service, a.service);
        assert_eq!(plan.minimum_table_size, 64);
        assert_eq!(
            plan_encoder_provider(
                &EncoderExtensionSelection::Automatic,
                requirement(),
                &candidates,
                &grants
            ),
            Err(EncoderPlanError::PermissionDenied)
        );
    }

    #[test]
    fn auto_orders_approved_providers_and_never_selects_an_unsupported_codec() {
        let (a, aa, ag) = candidate("example.encoder-a", 1);
        let (b, ba, bg) = candidate("example.encoder-b", 2);
        let candidates = [
            EncoderCandidate {
                descriptor: &a,
                approval: &aa,
                provider_grants: &ag,
            },
            EncoderCandidate {
                descriptor: &b,
                approval: &ba,
                provider_grants: &bg,
            },
        ];
        let grants = Grants {
            native_use: BTreeSet::from([a.service.clone(), b.service.clone()]),
            call: BTreeSet::from([a.service.clone(), b.service.clone()]),
            ..Grants::default()
        };
        let plan = plan_encoder_provider(
            &EncoderExtensionSelection::Automatic,
            requirement(),
            &candidates,
            &grants,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.provider_id, b.provider_id);
        assert_eq!(
            plan_encoder_provider(
                &EncoderExtensionSelection::Automatic,
                EncoderRequirement {
                    codec: VideoCodec::Hevc,
                    ..requirement()
                },
                &candidates,
                &grants
            ),
            Err(EncoderPlanError::NoCompatibleProvider)
        );
        assert_eq!(
            plan_encoder_provider(
                &EncoderExtensionSelection::Builtin,
                requirement(),
                &[],
                &Grants::default()
            ),
            Ok(None)
        );
    }

    #[test]
    fn descriptor_must_match_approval_abi_codecs_and_option_namespace() {
        let (mut descriptor, mut approval, provider_grants) = candidate("example.encoder-a", 1);
        let grants = Grants {
            native_use: BTreeSet::from([descriptor.service.clone()]),
            call: BTreeSet::from([descriptor.service.clone()]),
            ..Grants::default()
        };
        let plan = |descriptor: &EncoderProviderDescriptor, approval: &EncoderProviderApproval| {
            plan_encoder_provider(
                &EncoderExtensionSelection::Automatic,
                requirement(),
                &[EncoderCandidate {
                    descriptor,
                    approval,
                    provider_grants: &provider_grants,
                }],
                &grants,
            )
        };
        descriptor.configuration = Some(ExtensionConfigurationRef {
            version: ExtensionVersion::CURRENT,
            caller_component: descriptor.component_id.clone(),
            namespace: descriptor.component_id.clone(),
        });
        assert_eq!(
            plan(&descriptor, &approval),
            Err(EncoderPlanError::InvalidDescriptor)
        );
        approval.option_namespace = Some(descriptor.component_id.clone());
        assert!(plan(&descriptor, &approval).is_ok());
        descriptor.version.major += 1;
        assert_eq!(
            plan(&descriptor, &approval),
            Err(EncoderPlanError::InvalidDescriptor)
        );
        descriptor.version = ExtensionVersion::CURRENT;
        descriptor.codecs.insert(VideoCodec::Hevc);
        assert_eq!(
            plan(&descriptor, &approval),
            Err(EncoderPlanError::InvalidDescriptor)
        );
    }

    #[test]
    fn native_grants_without_control_calls_are_not_selectable() {
        let (descriptor, approval, mut provider) = candidate("example.encoder", 1);
        let consumer = Grants {
            native_use: BTreeSet::from([descriptor.service.clone()]),
            ..Grants::default()
        };
        let candidate = EncoderCandidate {
            descriptor: &descriptor,
            approval: &approval,
            provider_grants: &provider,
        };
        assert_eq!(
            plan_encoder_provider(
                &EncoderExtensionSelection::Automatic,
                requirement(),
                &[candidate],
                &consumer
            ),
            Err(EncoderPlanError::PermissionDenied)
        );
        provider.provide.clear();
        let consumer = Grants {
            call: BTreeSet::from([descriptor.service.clone()]),
            native_use: BTreeSet::from([descriptor.service.clone()]),
            ..Grants::default()
        };
        assert_eq!(
            plan_encoder_provider(
                &EncoderExtensionSelection::Automatic,
                requirement(),
                &[EncoderCandidate {
                    descriptor: &descriptor,
                    approval: &approval,
                    provider_grants: &provider,
                }],
                &consumer
            ),
            Err(EncoderPlanError::PermissionDenied)
        );
    }
}
