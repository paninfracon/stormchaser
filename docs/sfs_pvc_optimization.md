# Shared File System (SFS) PVC Optimization for Kubernetes Runner

The Stormchaser Kubernetes Runner (`stormchaser-runner-k8s`) supports a high-performance optimization for the Shared File System (SFS) using Kubernetes PersistentVolumeClaims (PVCs).

By default, Stormchaser's SFS works by "parking" (uploading) files to an S3-compatible backend at the end of a step and "unparking" (downloading) them at the start of the next step. While this ensures perfect data isolation and portability across different runners (e.g., mixing Docker and Kubernetes runners), it can be slow for workflows that pass large amounts of data between steps.

To solve this, the Kubernetes runner can be configured to use a single, shared `ReadWriteMany` PVC to instantly share data between steps without needing to upload or download files from S3.

## Configuration

The PVC optimization is configured globally at the **runner level**, not per-workflow.

To enable this feature, provide the `STORMCHASER_SFS_PVC_NAME` environment variable to the Kubernetes Runner deployment, pointing to an existing `ReadWriteMany` PVC in the same namespace.

```yaml
env:
  - name: STORMCHASER_SFS_PVC_NAME
    value: "stormchaser-shared-storage"
```

## Data Isolation

While the entire runner deployment uses a single global PVC, **the data inside the PVC is strictly isolated per workflow run.**

When the Kubernetes runner builds the Pod specification for a workflow step, it mounts the global PVC but leverages the Kubernetes `subPath` feature. The sub-path is dynamically generated using the unique UUID of the Workflow Run and the name of the storage mount:

```text
{run_id}/{storage_name}
```

### Example Scenario

1. You deploy the Runner with `STORMCHASER_SFS_PVC_NAME=stormchaser-shared-storage`.
2. Workflow Run `A` (UUID: `550e8400-e29b-41d4-a716-446655440000`) starts. Its "workspace" storage is mounted into the step's container from the PVC at the sub-path: `550e8400-e29b-41d4-a716-446655440000/workspace/`.
3. Workflow Run `B` (UUID: `123e4567-e89b-12d3-a456-426614174000`) starts concurrently. Its "workspace" storage is mounted from the *same* PVC but at the sub-path: `123e4567-e89b-12d3-a456-426614174000/workspace/`.
4. Run `A` Step 1 writes files. Run `A` Step 2 mounts `550e8400-e29b-41d4-a716-446655440000/workspace/` and sees the files instantly.
5. Run `B` cannot see Run `A`'s files, ensuring perfect isolation.

## Provisioning Support

If a workflow step requests `provision` URLs (to download external seed data into the volume before the step starts), the Kubernetes runner still fully supports this. It will dynamically inject Alpine-based init containers into the Pod to download and extract the provisioned data into the PVC sub-path before the main step container starts.

S3 "unparking" (restoring previous step state) is automatically bypassed when the PVC is enabled, as the state is already present on the disk.
