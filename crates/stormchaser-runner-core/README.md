# stormchaser-runner-core

What the Docker and Kubernetes runners share: the runner's life (NATS connection, session key, registration, orphan adoption, heartbeat, task and cancellation consumers, shutdown) around a `Backend` trait each runner implements, the encryption of step definitions kept on containers and Jobs, and reading the agent's marked log lines.
