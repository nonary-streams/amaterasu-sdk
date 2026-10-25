//! Capture replacement planning over the existing native service ABI.
//!
//! A plan is only a candidate for `NativeBind`; the bound C table's header,
//! size, callbacks and actual frame delivery remain authoritative. A module
//! descriptor cannot grant itself a service, priority, option namespace or GPU
//! support. All approval inputs must come from verified host/package policy.

use super::{ExtensionVersion, configuration::ExtensionConfigurationRef};
use crate::{Grants, valid_name};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_DECLARED_TABLE_SIZE: u32 = 1_048_576;

/// Existing video provider slots. Replacing a slot does not change the video
/// pipeline's capture state machine or implicitly enable a new source type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSlot {
    Dxgi,
    Wgc,
    Pipewire,
    Vkms,
    X11,
    Kms,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureCapability {
    CpuBgra,
    CpuNv12,
    DesktopGeometry,
    SourceFreshness,
    SourceTimestamp,
    /// Borrowed DMA-BUF descriptors through CaptureApi v1.4 or later. This
    /// does not prove an end-to-end zero-copy encode path.
    BorrowedDmaBuf,
    LatestPresentation,
}

impl CaptureCapability {
    pub const fn minimum_interface_minor(self) -> u32 {
        match self {
            Self::CpuBgra | Self::CpuNv12 => 0,
            Self::DesktopGeometry => 1,
            Self::SourceFreshness => 2,
            Self::SourceTimestamp => 3,
            Self::BorrowedDmaBuf => 4,
            Self::LatestPresentation => 5,
        }
    }
}

/// Publisher-declared capabilities; untrusted until an exact signed approval,
/// grants and a successful native bind validate them. No pointer or FD is
/// serialized here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureProviderDescriptor {
    pub version: ExtensionVersion,
    pub provider_id: String,
    pub component_id: String,
    pub service: String,
    pub slot: CaptureSlot,
    pub interface_major: u32,
    pub interface_minor: u32,
    pub declared_table_size: u32,
    pub capabilities: BTreeSet<CaptureCapability>,
    pub configuration: Option<ExtensionConfigurationRef>,
}

/// Host-derived signed approval. Priority is never taken from the module's
/// descriptor. Merely constructing this Rust value does not authenticate it;
/// the caller must obtain it from verified package policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureProviderApproval {
    pub provider_id: String,
    pub component_id: String,
    pub service: String,
    pub slot: CaptureSlot,
    pub priority: i16,
    pub interface_major: u32,
    pub minimum_interface_minor: u32,
    pub minimum_table_size: u32,
    pub allowed_capabilities: BTreeSet<CaptureCapability>,
    /// None forbids extension-owned settings. Some requires the exact caller-
    /// scoped configuration identity approved for this component.
    pub option_namespace: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureSelection {
    /// Default: retain the existing built-in slot provider.
    #[default]
    Builtin,
    /// An explicit provider never silently falls back to another extension.
    Exact { provider_id: String },
    /// Choose the highest signed priority, then lexical identity as a tie break.
    Automatic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureRequirement {
    pub slot: CaptureSlot,
    pub interface_major: u32,
    pub minimum_interface_minor: u32,
    /// Derived by the consumer from the exact CaptureApi C table it will read.
    pub minimum_table_size: u32,
    pub capabilities: BTreeSet<CaptureCapability>,
}

pub struct CaptureCandidate<'a> {
    pub descriptor: &'a CaptureProviderDescriptor,
    pub approval: &'a CaptureProviderApproval,
    pub provider_grants: &'a Grants,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureBindPlan {
    pub provider_id: String,
    pub component_id: String,
    pub service: String,
    pub slot: CaptureSlot,
    pub interface_major: u32,
    pub minimum_interface_minor: u32,
    pub minimum_table_size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturePlanError {
    InvalidDescriptor,
    DuplicateProvider,
    PermissionDenied,
    NoCompatibleProvider,
}

/// Plan an exact native bind. `Builtin` returns None before inspecting any
/// extension; it has no hot-path cost or change to existing provider behavior.
/// An approved plan still needs `NativeBind`, whose runtime checks effective
/// grants and whose provider must return an ABI-compatible live interface.
pub fn plan_capture_provider(
    selection: &CaptureSelection,
    requirement: &CaptureRequirement,
    candidates: &[CaptureCandidate<'_>],
    consumer_grants: &Grants,
) -> Result<Option<CaptureBindPlan>, CapturePlanError> {
    if matches!(selection, CaptureSelection::Builtin) {
        return Ok(None);
    }
    if requirement.interface_major != 1
        || requirement.minimum_table_size == 0
        || requirement.minimum_table_size > MAX_DECLARED_TABLE_SIZE
    {
        return Err(CapturePlanError::InvalidDescriptor);
    }
    let required_minor = requirement
        .capabilities
        .iter()
        .map(|capability| capability.minimum_interface_minor())
        .max()
        .unwrap_or(0)
        .max(requirement.minimum_interface_minor);
    let mut seen = BTreeSet::new();
    let mut compatible = Vec::new();
    for candidate in candidates {
        validate_pair(candidate)?;
        if !seen.insert(candidate.approval.provider_id.as_str()) {
            return Err(CapturePlanError::DuplicateProvider);
        }
        if candidate.descriptor.slot != requirement.slot {
            continue;
        }
        if let CaptureSelection::Exact { provider_id } = selection
            && &candidate.approval.provider_id != provider_id
        {
            continue;
        }
        let service = candidate.approval.service.as_str();
        if !candidate.provider_grants.native_provide.contains(service)
            || !consumer_grants.native_use.contains(service)
        {
            return Err(CapturePlanError::PermissionDenied);
        }
        if candidate.descriptor.interface_minor < required_minor
            || candidate.descriptor.declared_table_size < requirement.minimum_table_size
            || !requirement
                .capabilities
                .is_subset(&candidate.descriptor.capabilities)
        {
            continue;
        }
        compatible.push(candidate);
    }
    let selected = match selection {
        CaptureSelection::Builtin => unreachable!(),
        CaptureSelection::Exact { provider_id } => compatible
            .into_iter()
            .find(|candidate| &candidate.approval.provider_id == provider_id),
        CaptureSelection::Automatic => {
            compatible.sort_by(|left, right| {
                right
                    .approval
                    .priority
                    .cmp(&left.approval.priority)
                    .then_with(|| left.approval.provider_id.cmp(&right.approval.provider_id))
            });
            compatible.into_iter().next()
        }
    }
    .ok_or(CapturePlanError::NoCompatibleProvider)?;
    Ok(Some(CaptureBindPlan {
        provider_id: selected.approval.provider_id.clone(),
        component_id: selected.approval.component_id.clone(),
        service: selected.approval.service.clone(),
        slot: requirement.slot,
        interface_major: requirement.interface_major,
        minimum_interface_minor: required_minor.max(selected.approval.minimum_interface_minor),
        minimum_table_size: requirement
            .minimum_table_size
            .max(selected.approval.minimum_table_size),
    }))
}

fn validate_pair(candidate: &CaptureCandidate<'_>) -> Result<(), CapturePlanError> {
    let descriptor = candidate.descriptor;
    let approval = candidate.approval;
    if !descriptor.version.is_supported()
        || !valid_name(&descriptor.provider_id)
        || !descriptor.provider_id.contains('.')
        || descriptor.provider_id.starts_with("amaterasu.")
        || !valid_name(&descriptor.component_id)
        || descriptor.component_id.starts_with("amaterasu.")
        || !valid_name(&descriptor.service)
        || descriptor.service.starts_with("amaterasu.")
        || !descriptor.service.ends_with("/1")
        || descriptor.provider_id != approval.provider_id
        || descriptor.component_id != approval.component_id
        || descriptor.service != approval.service
        || descriptor.slot != approval.slot
        || descriptor.interface_major != 1
        || descriptor.interface_major != approval.interface_major
        || descriptor.interface_minor < approval.minimum_interface_minor
        || descriptor.declared_table_size < approval.minimum_table_size
        || approval.minimum_table_size == 0
        || approval.minimum_table_size > MAX_DECLARED_TABLE_SIZE
        || descriptor.declared_table_size == 0
        || descriptor.declared_table_size > MAX_DECLARED_TABLE_SIZE
        || !descriptor
            .capabilities
            .is_subset(&approval.allowed_capabilities)
        || !descriptor.capabilities.iter().any(|capability| {
            matches!(
                capability,
                CaptureCapability::CpuBgra
                    | CaptureCapability::CpuNv12
                    | CaptureCapability::BorrowedDmaBuf
            )
        })
        || descriptor
            .capabilities
            .iter()
            .any(|capability| capability.minimum_interface_minor() > descriptor.interface_minor)
    {
        return Err(CapturePlanError::InvalidDescriptor);
    }
    match (&descriptor.configuration, &approval.option_namespace) {
        (None, None) => {}
        (Some(configuration), Some(namespace))
            if namespace == &configuration.namespace
                && configuration.validate_for(&descriptor.component_id).is_ok() => {}
        _ => return Err(CapturePlanError::InvalidDescriptor),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        id: &str,
        priority: i16,
    ) -> (CaptureProviderDescriptor, CaptureProviderApproval, Grants) {
        let service = format!("{id}/1");
        let capabilities = [
            CaptureCapability::CpuBgra,
            CaptureCapability::DesktopGeometry,
        ]
        .into_iter()
        .collect();
        (
            CaptureProviderDescriptor {
                version: ExtensionVersion::CURRENT,
                provider_id: id.into(),
                component_id: id.into(),
                service: service.clone(),
                slot: CaptureSlot::Pipewire,
                interface_major: 1,
                interface_minor: 1,
                declared_table_size: 48,
                capabilities,
                configuration: None,
            },
            CaptureProviderApproval {
                provider_id: id.into(),
                component_id: id.into(),
                service: service.clone(),
                slot: CaptureSlot::Pipewire,
                priority,
                interface_major: 1,
                minimum_interface_minor: 1,
                minimum_table_size: 48,
                allowed_capabilities: [
                    CaptureCapability::CpuBgra,
                    CaptureCapability::DesktopGeometry,
                ]
                .into_iter()
                .collect(),
                option_namespace: None,
            },
            Grants {
                native_provide: [service].into(),
                ..Grants::default()
            },
        )
    }

    fn requirement() -> CaptureRequirement {
        CaptureRequirement {
            slot: CaptureSlot::Pipewire,
            interface_major: 1,
            minimum_interface_minor: 1,
            minimum_table_size: 48,
            capabilities: [CaptureCapability::CpuBgra].into(),
        }
    }

    #[test]
    fn builtin_is_default_and_does_not_touch_extension_candidates() {
        let invalid = candidate("amaterasu.invalid", 0);
        let result = plan_capture_provider(
            &CaptureSelection::default(),
            &requirement(),
            &[CaptureCandidate {
                descriptor: &invalid.0,
                approval: &invalid.1,
                provider_grants: &invalid.2,
            }],
            &Grants::default(),
        );
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn exact_grants_and_signed_identity_plan_existing_native_bind() {
        let provider = candidate("org.example.capture", 9);
        let consumer = Grants {
            native_use: [provider.0.service.clone()].into(),
            ..Grants::default()
        };
        let plan = plan_capture_provider(
            &CaptureSelection::Exact {
                provider_id: provider.0.provider_id.clone(),
            },
            &requirement(),
            &[CaptureCandidate {
                descriptor: &provider.0,
                approval: &provider.1,
                provider_grants: &provider.2,
            }],
            &consumer,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.service, provider.0.service);
        assert_eq!(plan.minimum_interface_minor, 1);
    }

    #[test]
    fn automatic_uses_approved_priority_and_stable_identity_tie_break() {
        let a = candidate("org.example.a", 5);
        let b = candidate("org.example.b", 5);
        let consumer = Grants {
            native_use: [a.0.service.clone(), b.0.service.clone()].into(),
            ..Grants::default()
        };
        let choices = [
            CaptureCandidate {
                descriptor: &b.0,
                approval: &b.1,
                provider_grants: &b.2,
            },
            CaptureCandidate {
                descriptor: &a.0,
                approval: &a.1,
                provider_grants: &a.2,
            },
        ];
        let plan = plan_capture_provider(
            &CaptureSelection::Automatic,
            &requirement(),
            &choices,
            &consumer,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.provider_id, "org.example.a");
    }

    #[test]
    fn exact_selection_ignores_an_unselected_providers_missing_consumer_grant() {
        let selected = candidate("org.example.selected", 1);
        let unselected = candidate("org.example.unselected", 9);
        let choices = [
            CaptureCandidate {
                descriptor: &selected.0,
                approval: &selected.1,
                provider_grants: &selected.2,
            },
            CaptureCandidate {
                descriptor: &unselected.0,
                approval: &unselected.1,
                provider_grants: &unselected.2,
            },
        ];
        let consumer = Grants {
            native_use: [selected.0.service.clone()].into(),
            ..Grants::default()
        };
        let plan = plan_capture_provider(
            &CaptureSelection::Exact {
                provider_id: selected.0.provider_id.clone(),
            },
            &requirement(),
            &choices,
            &consumer,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.provider_id, selected.0.provider_id);
        assert_eq!(
            plan_capture_provider(
                &CaptureSelection::Automatic,
                &requirement(),
                &choices,
                &consumer,
            ),
            Err(CapturePlanError::PermissionDenied),
            "automatic selection still checks every same-slot candidate"
        );
    }

    #[test]
    fn mismatched_service_and_missing_consumer_grant_fail_closed() {
        let mut provider = candidate("org.example.capture", 0);
        provider.0.service = "org.example.other/1".into();
        assert_eq!(
            plan_capture_provider(
                &CaptureSelection::Automatic,
                &requirement(),
                &[CaptureCandidate {
                    descriptor: &provider.0,
                    approval: &provider.1,
                    provider_grants: &provider.2,
                }],
                &Grants::default(),
            ),
            Err(CapturePlanError::InvalidDescriptor)
        );
        provider.0.service = provider.1.service.clone();
        provider.0.service = "amaterasu.capture-pipewire/1".into();
        provider.1.service = provider.0.service.clone();
        assert_eq!(
            plan_capture_provider(
                &CaptureSelection::Automatic,
                &requirement(),
                &[CaptureCandidate {
                    descriptor: &provider.0,
                    approval: &provider.1,
                    provider_grants: &provider.2,
                }],
                &Grants::default(),
            ),
            Err(CapturePlanError::InvalidDescriptor)
        );
        provider.0.service = "org.example.capture/1".into();
        provider.1.service = provider.0.service.clone();
        assert_eq!(
            plan_capture_provider(
                &CaptureSelection::Automatic,
                &requirement(),
                &[CaptureCandidate {
                    descriptor: &provider.0,
                    approval: &provider.1,
                    provider_grants: &provider.2,
                }],
                &Grants::default(),
            ),
            Err(CapturePlanError::PermissionDenied)
        );
    }

    #[test]
    fn dmabuf_claim_requires_v14_but_does_not_prove_gpu_compatibility() {
        let mut provider = candidate("org.example.capture", 0);
        provider
            .0
            .capabilities
            .insert(CaptureCapability::BorrowedDmaBuf);
        provider
            .1
            .allowed_capabilities
            .insert(CaptureCapability::BorrowedDmaBuf);
        let consumer = Grants {
            native_use: [provider.0.service.clone()].into(),
            ..Grants::default()
        };
        assert_eq!(
            plan_capture_provider(
                &CaptureSelection::Automatic,
                &requirement(),
                &[CaptureCandidate {
                    descriptor: &provider.0,
                    approval: &provider.1,
                    provider_grants: &provider.2,
                }],
                &consumer,
            ),
            Err(CapturePlanError::InvalidDescriptor)
        );
        provider.0.interface_minor = 4;
        let mut gpu_requirement = requirement();
        gpu_requirement
            .capabilities
            .insert(CaptureCapability::BorrowedDmaBuf);
        gpu_requirement.minimum_table_size = 72; // CaptureApiV1_4 on 64-bit.
        provider.0.declared_table_size = 72;
        let plan = plan_capture_provider(
            &CaptureSelection::Automatic,
            &gpu_requirement,
            &[CaptureCandidate {
                descriptor: &provider.0,
                approval: &provider.1,
                provider_grants: &provider.2,
            }],
            &consumer,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.minimum_interface_minor, 4);
        assert_eq!(plan.minimum_table_size, 72);
    }
}
