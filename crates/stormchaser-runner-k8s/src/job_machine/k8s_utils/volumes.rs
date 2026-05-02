use crate::job_machine::JobMetadata;
use k8s_openapi::api::core::v1::{ConfigMapVolumeSource, SecretVolumeSource, Volume, VolumeMount};

use super::*;

pub(crate) fn build_k8s_volumes(
    agent_image_present: bool,
    sfs_pvc_name: Option<&str>,
    metadata: &JobMetadata,
    step_spec: &StepSpec,
) -> (Vec<Volume>, Vec<VolumeMount>) {
    let mut volumes = Vec::new();
    let mut volume_mounts = Vec::new();

    // Add empty dir for agent binary
    if agent_image_present {
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

        let (volume, sub_path) = if let Some(pvc) = sfs_pvc_name {
            (
                Volume {
                    name: vol_name.clone(),
                    persistent_volume_claim: Some(
                        k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                            claim_name: pvc.to_string(),
                            ..Default::default()
                        },
                    ),
                    ..Default::default()
                },
                Some(format!("{}/{}", metadata.run_id, mount.name)),
            )
        } else {
            (
                Volume {
                    name: vol_name.clone(),
                    empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                    ..Default::default()
                },
                None,
            )
        };

        volumes.push(volume);
        volume_mounts.push(VolumeMount {
            name: vol_name,
            mount_path: mount.mount_path.clone(),
            read_only: mount.read_only,
            sub_path,
            ..Default::default()
        });
    }

    (volumes, volume_mounts)
}
