import re
import os

with open('crates/stormchaser-runner-docker/src/container_machine/transitions.rs', 'r') as f:
    content = f.read()

# Define the boundaries based on patterns
initialized_start = content.find("impl DockerContainerMachine<state::Initialized>")
running_start = content.find("impl DockerContainerMachine<state::Running>")
finished_start = content.find("impl DockerContainerMachine<state::Finished>")
tests_start = content.find("#[cfg(test)]\nmod tests {")

# Extract blocks
initialized_block = content[initialized_start:running_start]
running_block = content[running_start:finished_start]
finished_block = content[finished_start:tests_start]
tests_block = content[tests_start:]

# Create output dir
out_dir = 'crates/stormchaser-runner-docker/src/container_machine/transitions'
os.makedirs(out_dir, exist_ok=True)

# Write mod.rs
with open(os.path.join(out_dir, 'mod.rs'), 'w') as f:
    f.write('''pub mod initialized;
pub mod running;
pub mod finished;

#[cfg(test)]
mod tests;
''')

# Write initialized.rs
with open(os.path.join(out_dir, 'initialized.rs'), 'w') as f:
    f.write('''use crate::container_machine::{state, DockerContainerMachine, StartResult};
use anyhow::{Context, Result};
use bollard::container::{Config, CreateContainerOptions, StartContainerOptions, WaitContainerOptions};
use bollard::image::CreateImageOptions;
use bollard::service::{HostConfig, Mount, MountTypeEnum};
use bollard::volume::CreateVolumeOptions;
use chrono::Utc;
use futures::StreamExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use stormchaser_model::dsl::CommonContainerSpec;
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

''')
    f.write(initialized_block.strip() + '\n')

# Write running.rs
with open(os.path.join(out_dir, 'running.rs'), 'w') as f:
    f.write('''use crate::container_machine::{state, ContainerMetrics, ContainerState, DockerContainerMachine};
use anyhow::Result;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, StartContainerOptions, WaitContainerOptions};
use bollard::service::{HostConfig, Mount};
use chrono::Utc;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_model::dsl::CommonContainerSpec;
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

''')
    f.write(running_block.strip() + '\n')

# Write finished.rs
with open(os.path.join(out_dir, 'finished.rs'), 'w') as f:
    f.write('''use crate::container_machine::{state, ContainerState, DockerContainerMachine};

''')
    f.write(finished_block.strip() + '\n')

# Write tests.rs
tests_block_content = tests_block.replace("#[cfg(test)]\nmod tests {", "").strip()
tests_block_content = tests_block_content[:-1] # remove last brace

with open(os.path.join(out_dir, 'tests.rs'), 'w') as f:
    f.write(tests_block_content)

print("Files written successfully")
