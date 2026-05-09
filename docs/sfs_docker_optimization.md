# Shared File System (SFS) Optimization for Docker Runner

The Stormchaser Docker Runner (`stormchaser-runner-docker`) supports a high-performance optimization for the Shared File System (SFS) using Host Bind Mounts.

By default, Stormchaser's SFS works by "parking" (uploading) files to an S3-compatible backend at the end of a step and "unparking" (downloading) them at the start of the next step. While this ensures perfect data isolation and portability across different runners, it can be slow for workflows that pass large amounts of data between steps.

To solve this, the Docker runner can be configured to use a host directory via bind mounts to instantly share data between steps on the same host without needing to upload or download files from S3.

## Configuration

To enable this feature, provide the `STORMCHASER_SFS_HOST_PATH` environment variable to the Docker Runner, pointing to a directory on the host machine.

```bash
STORMCHASER_SFS_HOST_PATH=/var/lib/stormchaser/sfs
```

## Data Isolation

While the runner uses a single global host directory, **the data inside it is strictly isolated per workflow run.**

When the Docker runner builds the container specification for a workflow step, it dynamically creates a path for the run and mount using the format `{STORMCHASER_SFS_HOST_PATH}/{run_id}/{mount_name}` and binds it into the container.

### Example Scenario

1. You deploy the Runner with `STORMCHASER_SFS_HOST_PATH=/var/lib/stormchaser/sfs`.
2. Workflow Run `A` (UUID: `550e8400-e29b-41d4-a716-446655440000`) starts. Its "workspace" storage is bound from `/var/lib/stormchaser/sfs/550e8400-e29b-41d4-a716-446655440000/workspace/`.
3. Workflow Run `B` (UUID: `123e4567-e89b-12d3-a456-426614174000`) starts concurrently. Its "workspace" storage is bound from `/var/lib/stormchaser/sfs/123e4567-e89b-12d3-a456-426614174000/workspace/`.
4. Run `A` Step 1 writes files. Run `A` Step 2 mounts the same bind path and sees the files instantly.
5. Run `B` cannot see Run `A`'s files, ensuring isolation.

## Provisioning Support

If a workflow step requests `provision` URLs (to download external seed data into the volume before the step starts), the Docker runner still fully supports this. It will automatically download and extract the provisioned data into the host bind mount before the main step container starts.

S3 "unparking" (restoring previous step state) and "parking" (saving step state) are automatically bypassed when this feature is enabled, as the state is already preserved on the host disk between steps.

## Garbage Collection and Cleanup

Unlike Docker named volumes, which the runner can automatically reap, **the lifecycle of the host directory is left to the host system.** Because the runner manages steps independently, it does not inherently know when an entire workflow is "complete" across all runners.

You must implement a host-level cleanup mechanism (e.g., a simple cron job or systemd timer) to remove directories older than a certain threshold to prevent disk exhaustion.

Example cron job (runs hourly, deletes directories older than 24 hours):

```bash
0 * * * * find /var/lib/stormchaser/sfs -mindepth 1 -maxdepth 1 -type d -mmin +1440 -exec rm -rf {} +
```
