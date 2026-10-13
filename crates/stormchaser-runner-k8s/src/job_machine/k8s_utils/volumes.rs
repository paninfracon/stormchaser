use crate::job_machine::JobMetadata;
use k8s_openapi::api::core::v1::{ConfigMapVolumeSource, SecretVolumeSource, Volume, VolumeMount};

use super::*;

/// Builds the pod's volumes, and the mounts of the step's own container.
///
/// `in_step_agent` is true when the agent binary is copied into the step's container to
/// collect its test reports.
pub(crate) fn build_k8s_volumes(
    in_step_agent: bool,
    sfs_pvc_name: Option<&str>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
) -> (Vec<Volume>, Vec<VolumeMount>) {
    let mut volumes = Vec::new();
    let mut volume_mounts = Vec::new();

    // Add empty dir for agent binary
    if in_step_agent {
        volumes.push(Volume {
            name: "storm-agent".to_string(),
            empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: "storm-agent".to_string(),
            mount_path: "/stormchaser/agent".to_string(),
            ..Default::default()
        });
    }

    // Handle secret mounts
    for sm in &step_spec.secret_mounts {
        let vol_name = normalize_resource_name(&sm.name, "sec");
        volumes.push(Volume {
            name: vol_name.clone(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(sm.name.clone()),
                ..Default::default()
            }),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: vol_name,
            mount_path: sm.mount_path.clone(),
            ..Default::default()
        });
    }

    // Handle config map mounts
    for cm in &step_spec.config_map_mounts {
        let vol_name = normalize_resource_name(&cm.name, "cm");
        volumes.push(Volume {
            name: vol_name.clone(),
            config_map: Some(ConfigMapVolumeSource {
                name: cm.name.clone(),
                ..Default::default()
            }),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: vol_name,
            mount_path: cm.mount_path.clone(),
            ..Default::default()
        });
    }

    // Handle storage mounts
    for mount in &step_spec.storage_mounts {
        let vol_name = normalize_resource_name(&mount.name, "sfs");

        let volume = if let Some(pvc) = sfs_pvc_name {
            Volume {
                name: vol_name.clone(),
                persistent_volume_claim: Some(
                    k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                        claim_name: pvc.to_string(),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            }
        } else {
            Volume {
                name: vol_name.clone(),
                empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                ..Default::default()
            }
        };

        volumes.push(volume);
    }
    volume_mounts.extend(storage_volume_mounts(sfs_pvc_name, metadata, step_spec));

    (volumes, volume_mounts)
}

/// The mounts of a step's storage volumes, as both the step's container and the agent
/// sidecar see them.
pub(crate) fn storage_volume_mounts(
    sfs_pvc_name: Option<&str>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
) -> Vec<VolumeMount> {
    step_spec
        .storage_mounts
        .iter()
        .map(|mount| VolumeMount {
            name: normalize_resource_name(&mount.name, "sfs"),
            mount_path: mount.mount_path.clone(),
            read_only: mount.read_only,
            sub_path: sfs_pvc_name.map(|_| format!("{}/{}", metadata.run_id, mount.name)),
            ..Default::default()
        })
        .collect()
}
