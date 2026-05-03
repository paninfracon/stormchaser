use crate::container_machine::{state, ContainerState, DockerContainerMachine};

impl DockerContainerMachine<state::Finished> {
    /// Consumes the state machine, returning the final `ContainerState` result.
    pub fn into_result(self) -> ContainerState {
        self.state.result
    }
}
