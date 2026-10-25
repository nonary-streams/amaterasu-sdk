//! Safe dropdown data derived from the existing host-approved provider plans.
//!
//! This is deliberately not another registry. Package verification must first
//! bind each record to signed component requests and effective host grants.
//! A management adapter may then use these functions to show only compatible
//! choices. Selecting one still requires the exact package and native bind.

use super::capture::{
    CaptureCandidate, CapturePlanError, CaptureRequirement, CaptureSelection, plan_capture_provider,
};
use super::encoder::{
    EncoderCandidate, EncoderExtensionSelection, EncoderPlanError, EncoderRequirement,
    plan_encoder_provider,
};
use crate::Grants;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_CHOICES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderChoiceKind {
    Capture,
    Encoder,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderChoice {
    pub kind: ProviderChoiceKind,
    pub provider_id: String,
    pub component_id: String,
    pub service: String,
    /// Exact caller-scoped configuration namespace, when approved.
    pub option_namespace: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChoiceError {
    TooMany,
    Duplicate,
    Invalid,
}

/// List compatible capture providers using the same exact planner as binding.
/// Entries lacking effective grants are not selectable and are omitted.
pub fn capture_dropdown_options(
    requirement: &CaptureRequirement,
    candidates: &[CaptureCandidate<'_>],
    consumer_grants: &Grants,
) -> Result<Vec<ProviderChoice>, ChoiceError> {
    if candidates.len() > MAX_CHOICES {
        return Err(ChoiceError::TooMany);
    }
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for candidate in candidates {
        if !seen.insert(candidate.descriptor.provider_id.as_str()) {
            return Err(ChoiceError::Duplicate);
        }
        let service = &candidate.descriptor.service;
        if !candidate.provider_grants.provide.contains(service)
            || !consumer_grants.call.contains(service)
        {
            continue;
        }
        let selection = CaptureSelection::Exact {
            provider_id: candidate.descriptor.provider_id.clone(),
        };
        match plan_capture_provider(
            &selection,
            requirement,
            std::slice::from_ref(candidate),
            consumer_grants,
        ) {
            Ok(Some(plan)) => output.push(ProviderChoice {
                kind: ProviderChoiceKind::Capture,
                provider_id: plan.provider_id,
                component_id: plan.component_id,
                service: plan.service,
                option_namespace: candidate.approval.option_namespace.clone(),
            }),
            Ok(None)
            | Err(CapturePlanError::NoCompatibleProvider | CapturePlanError::PermissionDenied) => {}
            Err(CapturePlanError::InvalidDescriptor | CapturePlanError::DuplicateProvider) => {
                return Err(ChoiceError::Invalid);
            }
        }
    }
    output.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
    Ok(output)
}

/// List compatible encoders without probing hardware or claiming an available
/// codec. The live encoder's own enumeration/open still determines availability.
pub fn encoder_dropdown_options(
    requirement: EncoderRequirement,
    candidates: &[EncoderCandidate<'_>],
    consumer_grants: &Grants,
) -> Result<Vec<ProviderChoice>, ChoiceError> {
    if candidates.len() > MAX_CHOICES {
        return Err(ChoiceError::TooMany);
    }
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for candidate in candidates {
        if !seen.insert(candidate.descriptor.provider_id.as_str()) {
            return Err(ChoiceError::Duplicate);
        }
        let selection = EncoderExtensionSelection::Exact {
            provider_id: candidate.descriptor.provider_id.clone(),
        };
        match plan_encoder_provider(
            &selection,
            requirement,
            std::slice::from_ref(candidate),
            consumer_grants,
        ) {
            Ok(Some(plan)) => output.push(ProviderChoice {
                kind: ProviderChoiceKind::Encoder,
                provider_id: plan.provider_id,
                component_id: plan.component_id,
                service: plan.service,
                option_namespace: candidate.approval.option_namespace.clone(),
            }),
            Ok(None)
            | Err(EncoderPlanError::NoCompatibleProvider | EncoderPlanError::PermissionDenied) => {}
            Err(EncoderPlanError::InvalidDescriptor | EncoderPlanError::DuplicateProvider) => {
                return Err(ChoiceError::Invalid);
            }
        }
    }
    output.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::super::{
        ExtensionVersion,
        capture::{
            CaptureCapability, CaptureProviderApproval, CaptureProviderDescriptor, CaptureSlot,
        },
        encoder::{EncoderProviderApproval, EncoderProviderDescriptor},
    };
    use super::*;
    use crate::encoder::VideoCodec;

    #[test]
    fn dropdown_omits_ungranted_encoder_and_preserves_namespaced_settings() {
        let first = EncoderProviderDescriptor {
            version: ExtensionVersion::CURRENT,
            provider_id: "example.encoder-a".into(),
            component_id: "example.encoder-a".into(),
            service: "example.encoder-a/1".into(),
            interface_major: 1,
            interface_minor: 0,
            declared_table_size: 64,
            codecs: BTreeSet::from([VideoCodec::H264]),
            configuration: Some(super::super::configuration::ExtensionConfigurationRef {
                version: ExtensionVersion::CURRENT,
                caller_component: "example.encoder-a".into(),
                namespace: "example.encoder-a".into(),
            }),
        };
        let approved = EncoderProviderApproval {
            provider_id: first.provider_id.clone(),
            component_id: first.component_id.clone(),
            service: first.service.clone(),
            priority: 1,
            interface_major: 1,
            minimum_interface_minor: 0,
            minimum_table_size: 64,
            allowed_codecs: first.codecs.clone(),
            option_namespace: Some(first.component_id.clone()),
        };
        let provider_grants = Grants {
            provide: BTreeSet::from([first.service.clone()]),
            native_provide: BTreeSet::from([first.service.clone()]),
            ..Grants::default()
        };
        let candidate = || EncoderCandidate {
            descriptor: &first,
            approval: &approved,
            provider_grants: &provider_grants,
        };
        let requirement = EncoderRequirement {
            codec: VideoCodec::H264,
            interface_major: 1,
            minimum_interface_minor: 0,
            minimum_table_size: 64,
        };
        assert!(
            encoder_dropdown_options(requirement, &[candidate()], &Grants::default())
                .unwrap()
                .is_empty()
        );
        let granted = Grants {
            call: BTreeSet::from([first.service.clone()]),
            native_use: BTreeSet::from([first.service.clone()]),
            ..Grants::default()
        };
        let choice = encoder_dropdown_options(requirement, &[candidate()], &granted)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(
            choice.option_namespace.as_deref(),
            Some("example.encoder-a")
        );
        assert_eq!(choice.kind, ProviderChoiceKind::Encoder);
        assert_eq!(choice.service, "example.encoder-a/1");
    }

    #[test]
    fn capture_choice_uses_the_existing_x11_planner_and_exact_effective_grants() {
        let service = "example.capture/1";
        let caps = BTreeSet::from([CaptureCapability::CpuBgra]);
        let descriptor = CaptureProviderDescriptor {
            version: ExtensionVersion::CURRENT,
            provider_id: "example.capture".into(),
            component_id: "example.capture".into(),
            service: service.into(),
            slot: CaptureSlot::X11,
            interface_major: 1,
            interface_minor: 0,
            declared_table_size: 64,
            capabilities: caps.clone(),
            configuration: None,
        };
        let approval = CaptureProviderApproval {
            provider_id: descriptor.provider_id.clone(),
            component_id: descriptor.component_id.clone(),
            service: service.into(),
            slot: CaptureSlot::X11,
            priority: 0,
            interface_major: 1,
            minimum_interface_minor: 0,
            minimum_table_size: 64,
            allowed_capabilities: caps.clone(),
            option_namespace: None,
        };
        let provider_grants = Grants {
            provide: BTreeSet::from([service.into()]),
            native_provide: BTreeSet::from([service.into()]),
            ..Grants::default()
        };
        let requirement = CaptureRequirement {
            slot: CaptureSlot::X11,
            interface_major: 1,
            minimum_interface_minor: 0,
            minimum_table_size: 64,
            capabilities: caps,
        };
        let candidate = || CaptureCandidate {
            descriptor: &descriptor,
            approval: &approval,
            provider_grants: &provider_grants,
        };
        assert!(
            capture_dropdown_options(&requirement, &[candidate()], &Grants::default())
                .unwrap()
                .is_empty()
        );
        let grants = Grants {
            call: BTreeSet::from([service.into()]),
            native_use: BTreeSet::from([service.into()]),
            ..Grants::default()
        };
        let choice = capture_dropdown_options(&requirement, &[candidate()], &grants).unwrap();
        assert_eq!(choice.len(), 1);
        assert_eq!(choice[0].kind, ProviderChoiceKind::Capture);
        assert_eq!(choice[0].provider_id, "example.capture");
        assert_eq!(
            capture_dropdown_options(&requirement, &[candidate(), candidate()], &grants),
            Err(ChoiceError::Duplicate)
        );
    }
}
