use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions, LogOutput,
    LogsOptions, RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
    WaitContainerOptions,
};
use bollard::errors::Error;
use bollard::image::CreateImageOptions;
use bollard::models::{
    ContainerCreateResponse, ContainerInspectResponse, ContainerSummary, ContainerWaitResponse,
    CreateImageInfo, Volume, VolumeListResponse,
};
use bollard::volume::{CreateVolumeOptions, ListVolumesOptions, RemoveVolumeOptions};
use bollard::Docker;
use bytes::Bytes;
use futures::{stream::BoxStream, StreamExt};

#[cfg(test)]
use mockall::predicate::*;

#[cfg(test)]
mockall::mock! {
    pub ContainerRuntime {}

    #[async_trait]
    impl ContainerRuntime for ContainerRuntime {
        async fn create_container(
            &self,
            options: Option<CreateContainerOptions<String>>,
            config: Config<String>,
        ) -> Result<ContainerCreateResponse, Error>;

        async fn start_container(
            &self,
            container_name: &str,
            options: Option<StartContainerOptions<String>>,
        ) -> Result<(), Error>;

        async fn stop_container(
            &self,
            container_name: &str,
            options: Option<StopContainerOptions>,
        ) -> Result<(), Error>;

        async fn remove_container(
            &self,
            container_name: &str,
            options: Option<RemoveContainerOptions>,
        ) -> Result<(), Error>;

        fn wait_container(
            &self,
            container_name: &str,
            options: Option<WaitContainerOptions<String>>,
        ) -> BoxStream<'static, Result<ContainerWaitResponse, Error>>;

        async fn create_volume(
            &self,
            config: CreateVolumeOptions<String>,
        ) -> Result<Volume, Error>;

        async fn remove_volume(
            &self,
            name: &str,
            options: Option<RemoveVolumeOptions>,
        ) -> Result<(), Error>;

        fn create_image(
            &self,
            options: Option<CreateImageOptions<'static, String>>,
            root_fs: Option<Bytes>,
            credentials: Option<bollard::auth::DockerCredentials>,
        ) -> BoxStream<'static, Result<CreateImageInfo, Error>>;

        fn logs(
            &self,
            container_name: &str,
            options: Option<LogsOptions<String>>,
        ) -> BoxStream<'static, Result<LogOutput, Error>>;

        async fn create_exec(
            &self,
            container_name: &str,
            config: bollard::exec::CreateExecOptions<String>,
        ) -> Result<bollard::exec::CreateExecResults, Error>;

        async fn start_exec(
            &self,
            exec_id: &str,
            config: Option<bollard::exec::StartExecOptions>,
        ) -> Result<bollard::exec::StartExecResults, Error>;

        async fn inspect_exec(
            &self,
            exec_id: &str,
        ) -> Result<bollard::models::ExecInspectResponse, Error>;

        async fn inspect_container(
            &self,
            container_name: &str,
            options: Option<InspectContainerOptions>,
        ) -> Result<ContainerInspectResponse, Error>;

        async fn list_containers(
            &self,
            options: Option<ListContainersOptions<String>>,
        ) -> Result<Vec<ContainerSummary>, Error>;

        async fn list_volumes(
            &self,
            options: Option<ListVolumesOptions<String>>,
        ) -> Result<VolumeListResponse, Error>;
    }

    impl Clone for ContainerRuntime {
        fn clone(&self) -> Self;
    }
}

/// A trait abstraction over Docker container runtime interactions.
#[async_trait]
pub trait ContainerRuntime: Send + Sync + Clone + 'static {
    /// Creates a new container.
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions<String>>,
        config: Config<String>,
    ) -> Result<ContainerCreateResponse, Error>;

    /// Starts an existing container.
    async fn start_container(
        &self,
        container_name: &str,
        options: Option<StartContainerOptions<String>>,
    ) -> Result<(), Error>;

    /// Stops a running container.
    async fn stop_container(
        &self,
        container_name: &str,
        options: Option<StopContainerOptions>,
    ) -> Result<(), Error>;

    /// Removes a container.
    async fn remove_container(
        &self,
        container_name: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), Error>;

    /// Waits for a container to finish executing.
    fn wait_container(
        &self,
        container_name: &str,
        options: Option<WaitContainerOptions<String>>,
    ) -> BoxStream<'static, Result<ContainerWaitResponse, Error>>;

    /// Creates a volume.
    async fn create_volume(&self, config: CreateVolumeOptions<String>) -> Result<Volume, Error>;

    /// Removes a volume.
    async fn remove_volume(
        &self,
        name: &str,
        options: Option<RemoveVolumeOptions>,
    ) -> Result<(), Error>;

    /// Creates an image.
    fn create_image(
        &self,
        options: Option<CreateImageOptions<'static, String>>,
        root_fs: Option<Bytes>,
        credentials: Option<bollard::auth::DockerCredentials>,
    ) -> BoxStream<'static, Result<CreateImageInfo, Error>>;

    /// Retrieves logs for a container.
    fn logs(
        &self,
        container_name: &str,
        options: Option<LogsOptions<String>>,
    ) -> BoxStream<'static, Result<LogOutput, Error>>;

    async fn create_exec(
        &self,
        container_name: &str,
        config: bollard::exec::CreateExecOptions<String>,
    ) -> Result<bollard::exec::CreateExecResults, Error>;

    async fn start_exec(
        &self,
        exec_id: &str,
        config: Option<bollard::exec::StartExecOptions>,
    ) -> Result<bollard::exec::StartExecResults, Error>;

    async fn inspect_exec(
        &self,
        exec_id: &str,
    ) -> Result<bollard::models::ExecInspectResponse, Error>;

    /// Inspects a container's details.
    async fn inspect_container(
        &self,
        container_name: &str,
        options: Option<InspectContainerOptions>,
    ) -> Result<ContainerInspectResponse, Error>;

    /// Lists containers.
    async fn list_containers(
        &self,
        options: Option<ListContainersOptions<String>>,
    ) -> Result<Vec<ContainerSummary>, Error>;

    /// Lists volumes.
    async fn list_volumes(
        &self,
        options: Option<ListVolumesOptions<String>>,
    ) -> Result<VolumeListResponse, Error>;
}

#[async_trait]
impl ContainerRuntime for Docker {
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions<String>>,
        config: Config<String>,
    ) -> Result<ContainerCreateResponse, Error> {
        self.create_container(options, config).await
    }

    async fn start_container(
        &self,
        container_name: &str,
        options: Option<StartContainerOptions<String>>,
    ) -> Result<(), Error> {
        self.start_container(container_name, options).await
    }

    async fn stop_container(
        &self,
        container_name: &str,
        options: Option<StopContainerOptions>,
    ) -> Result<(), Error> {
        self.stop_container(container_name, options).await
    }

    async fn remove_container(
        &self,
        container_name: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), Error> {
        self.remove_container(container_name, options).await
    }

    fn wait_container(
        &self,
        container_name: &str,
        options: Option<WaitContainerOptions<String>>,
    ) -> BoxStream<'static, Result<ContainerWaitResponse, Error>> {
        self.wait_container(container_name, options).boxed()
    }

    async fn create_volume(&self, config: CreateVolumeOptions<String>) -> Result<Volume, Error> {
        self.create_volume(config).await
    }

    async fn remove_volume(
        &self,
        name: &str,
        options: Option<RemoveVolumeOptions>,
    ) -> Result<(), Error> {
        self.remove_volume(name, options).await
    }

    fn create_image(
        &self,
        options: Option<CreateImageOptions<'static, String>>,
        root_fs: Option<Bytes>,
        credentials: Option<bollard::auth::DockerCredentials>,
    ) -> BoxStream<'static, Result<CreateImageInfo, Error>> {
        self.create_image(options, root_fs, credentials).boxed()
    }

    fn logs(
        &self,
        container_name: &str,
        options: Option<LogsOptions<String>>,
    ) -> BoxStream<'static, Result<LogOutput, Error>> {
        self.logs(container_name, options).boxed()
    }

    async fn create_exec(
        &self,
        container_name: &str,
        config: bollard::exec::CreateExecOptions<String>,
    ) -> Result<bollard::exec::CreateExecResults, Error> {
        self.create_exec(container_name, config).await
    }

    async fn start_exec(
        &self,
        exec_id: &str,
        config: Option<bollard::exec::StartExecOptions>,
    ) -> Result<bollard::exec::StartExecResults, Error> {
        self.start_exec(exec_id, config).await
    }

    async fn inspect_exec(
        &self,
        exec_id: &str,
    ) -> Result<bollard::models::ExecInspectResponse, Error> {
        self.inspect_exec(exec_id).await
    }

    async fn inspect_container(
        &self,
        container_name: &str,
        options: Option<InspectContainerOptions>,
    ) -> Result<ContainerInspectResponse, Error> {
        self.inspect_container(container_name, options).await
    }

    async fn list_containers(
        &self,
        options: Option<ListContainersOptions<String>>,
    ) -> Result<Vec<ContainerSummary>, Error> {
        self.list_containers(options).await
    }

    async fn list_volumes(
        &self,
        options: Option<ListVolumesOptions<String>>,
    ) -> Result<VolumeListResponse, Error> {
        self.list_volumes(options).await
    }
}

#[cfg(test)]
mockall::mock! {
    pub MessageBus {}

    #[async_trait]
    impl MessageBus for MessageBus {
        async fn publish(
            &self,
            subject: String,
            payload: Bytes,
        ) -> Result<(), async_nats::PublishError>;
        async fn request(
            &self,
            subject: String,
            payload: Bytes,
        ) -> Result<async_nats::Message, async_nats::RequestError>;
    }

    impl Clone for MessageBus {
        fn clone(&self) -> Self;
    }
}

/// A trait abstraction over message bus (e.g. NATS) interactions.
#[async_trait]
pub trait MessageBus: Send + Sync + Clone + 'static {
    /// Publishes a message to a given subject.
    async fn publish(
        &self,
        subject: String,
        payload: Bytes,
    ) -> Result<(), async_nats::PublishError>;
    /// Sends a request message to a given subject and waits for a response.
    async fn request(
        &self,
        subject: String,
        payload: Bytes,
    ) -> Result<async_nats::Message, async_nats::RequestError>;
    // subscribe returning BoxStream might be tough to mock properly without dealing with lifetimes
    // we'll just define publish and request for now, since those are what scan_for_orphans uses.
}

#[async_trait]
impl MessageBus for async_nats::Client {
    async fn publish(
        &self,
        subject: String,
        payload: Bytes,
    ) -> Result<(), async_nats::PublishError> {
        self.publish(subject, payload).await
    }
    async fn request(
        &self,
        subject: String,
        payload: Bytes,
    ) -> Result<async_nats::Message, async_nats::RequestError> {
        self.request(subject, payload).await
    }
}
