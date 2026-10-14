# NATS: Quickstart and Production

NATS JetStream carries every task Stormchaser hands to a runner and every
result a runner reports back. Anyone who can publish to it can schedule
containers and report on other users' steps, so how NATS is secured decides how
far a compromised step or runner can reach.

Stormchaser bundles NATS in two places so that you can evaluate it without
provisioning anything: `docker-compose.yml` and the umbrella Helm chart
(`deploy/charts/stormchaser`). **The bundled server is a quickstart, not a
production broker.** For production, run NATS yourself (or use a managed
service) and configure it as described [below](#production-requirements).

## What the quickstart gives you

| | Quickstart (compose and umbrella chart) | Production requirement |
| --- | --- | --- |
| Authentication | Four users with generated passwords: engine, API, query service, runners | Yes, per role |
| Permissions | Runners, the API and the query service restricted to the subjects below; the engine unrestricted | Yes |
| TLS | None; traffic is plaintext | TLS on every link, verified |
| Nodes | 1 | 3 or more, clustered |
| Stream replicas | 1 | 3 |
| Network isolation | Steps on a separate Docker network or behind a NetworkPolicy; **off** when started by `scripts/setup.sh` | Step containers cannot reach NATS |
| Monitoring port (8222) | Open; published on the host by compose | Restricted |
| Storage | 10Gi PVC (Helm); container filesystem (compose) | Sized, persistent, encrypted |

The quickstart requires a password because evaluation installs have a habit of
becoming permanent. That makes it safe against a stray client on the same
network. It does not make it production-ready: passwords and every message
travel in plaintext, and a single node loses in-flight work when it restarts.

### Quickstart credentials

| User | Used by | Where the password lives |
| --- | --- | --- |
| `stormchaser` | Engine | compose: `STORMCHASER_NATS_PASSWORD` in `.env`; Helm: Secret `stormchaser-nats-control` |
| `stormchaser-api` | API | compose: `STORMCHASER_NATS_API_PASSWORD` in `.env`; Helm: Secret `stormchaser-nats-api` |
| `stormchaser-query` | Query service | compose: `STORMCHASER_NATS_QUERY_PASSWORD` in `.env`; Helm: Secret `stormchaser-nats-query` |
| `stormchaser-runner` | Docker and Kubernetes runners | compose: `STORMCHASER_NATS_RUNNER_PASSWORD` in `.env`; Helm: Secret `stormchaser-nats-runner` |

`scripts/setup.sh` generates the compose passwords. The umbrella chart
generates the Helm Secrets on first install and keeps them across upgrades
(`natsAuth.generate`). The server configuration is in
`deploy/nats/nats-server.conf` for compose and under `nats.config.merge` in the
umbrella chart's `values.yaml`; keep the two in step.

To talk to the quickstart server yourself, pass the engine's credentials, which are
the only unrestricted ones:

```bash
nats --server nats://localhost:4222 \
  --user stormchaser --password "$STORMCHASER_NATS_PASSWORD" stream ls
```

## How components authenticate

Every component reads its NATS credentials from the environment, in this order
of preference:

| Variable | Meaning |
| --- | --- |
| `NATS_CREDS_FILE` | Path to a NATS `.creds` file (JWT and NKey). Preferred for production. |
| `NATS_USER` / `NATS_PASSWORD` | Username and password. |
| `NATS_TOKEN` | Shared token. NATS documents this as suitable for quick internal setups only. |
| `NATS_CA_FILE` | Root certificate used to verify the NATS server. |
| `NATS_TLS_REQUIRED` | `true` refuses a server that does not offer TLS. |

A component that finds no credentials logs a warning and connects anonymously.
Credentials embedded in `NATS_URL` (`nats://user:pass@host`) are **not** used.

In Helm, `global.nats.authSecret` (orchestration chart) and
`config.natsAuthSecret` (runner charts) name a Secret with `user` and
`password` keys. To use a `.creds` file or TLS settings, mount the file and set
the variables above on the deployment.

The API and engine also present their mTLS client certificate
(`TLS_CERT_PATH` / `TLS_KEY_PATH`) to NATS. It is only checked if the server
sets `verify: true`. The runners do not present a client certificate; they
authenticate with the credentials above.

## What Stormchaser needs from NATS

- JetStream enabled.
- One stream, `stormchaser`, capturing `stormchaser.v1.>` and nothing wider.
  The engine creates it if it does not exist; you can create it yourself (with
  the replica count you want) before first start. Requests to the engine
  (`stormchaser.rpc.>`) must stay outside it: the stream acknowledges what it
  stores to the message's reply subject, so a request it captured would get that
  acknowledgement instead of the engine's answer. The engine narrows a stream
  that captures `stormchaser.>`, as earlier runners created it.
- Durable pull consumers, created by the components that use them:
  `orchestration-engine-global`, `orchestration-engine-shard-<n>` (one per
  engine shard), `orchestration-engine-legacy-run`,
  `orchestration-engine-legacy-runner`, `orchestration-engine-legacy-step`,
  `runner-container-tasks` (`RunContainer` tasks, shared by Docker and Kubernetes
  runners so that each task goes to one runner), `runner-k8s-job-tasks`
  (`RunK8sJob` tasks), and one ephemeral consumer per running runner named
  `runner-<type>-<runner id>`. Earlier releases used `docker-runner` and
  `k8s-runner`, which nothing reads any more; remove them with
  `nats consumer rm stormchaser docker-runner` and `nats consumer rm stormchaser k8s-runner`.
- A key-value bucket for API rate limiting (`api_rate_limit_buckets`), created by the API.

## Dedicated or shared NATS

Whether Stormchaser gets its own NATS cluster or uses one you already run is
your decision. It matters more than usual here, because of what travels over
NATS:

- **Task messages contain secrets.** The engine resolves a step before
  dispatching it, so the message a runner receives holds any secret the
  workflow passes to a container, image pull credentials, and storage access
  URLs, as plain JSON. The credentials of an OCI artifact registry are not in
  it: a runner asks for a short-lived token when it needs to push, and the
  answer goes to its reply inbox, which the stream does not store.
- **A task message is an instruction to run a container.** Whoever can publish
  one on the right subject can make a runner execute it.
- **Stormchaser does not encrypt or sign these messages itself.** It relies on
  NATS for confidentiality (TLS, authentication, permissions) and on the
  filesystem for encryption at rest.
- **Task messages are erased when their step is over.** The engine erases a
  step's task message from the stream within about a minute of the step
  finishing, and the stream has a maximum age as a backstop (see
  [Availability and storage](#4-availability-and-storage)). Until then the
  message is readable through the NATS API by anyone with access to the stream.

So the question to answer is: *who, other than Stormchaser, can read or
publish on this NATS server, and do you trust them with every secret your
workflows use and with the ability to run containers on your runners?*

| | Dedicated cluster | Shared cluster, own NATS account | Shared cluster, shared account |
| --- | --- | --- | --- |
| Who can read task messages and their secrets | Stormchaser components and your NATS administrators | Stormchaser components and the cluster's operators. Other tenants cannot: accounts have separate subject spaces and separate JetStream storage | Anyone in the account whose permissions cover `stormchaser.>` or the JetStream API, plus the cluster's operators |
| Who can publish a task to your runners | The engine's user only | The control-plane user, and a cluster operator acting inside your account | Any user in the account without an explicit deny, plus operators |
| Is the documented runner permission list enough | Yes | Yes, applied to users inside your account | Only if every other user in the account is denied `stormchaser.>`, `$JS.API.*.stormchaser.>` and `$JS.ACK.stormchaser.>`. That is a deny list you must maintain as others add users |
| Name collisions | None | None | The stream is always named `stormchaser` and consumers have fixed names; another application could clash |
| TLS, client verification and encryption at rest | You set them | Set by the cluster's operators; check they meet the requirements below | Set by the cluster's operators |
| Retention, backups and mirrors of the stream | Under your control | Limits are yours within the account; backups and mirrors are the operators' and will contain task secrets | Same, and other users may be able to read or copy the stream |
| Availability | Yours alone to size | Shared hardware; ask for account-level JetStream limits so another tenant cannot exhaust storage | Shared hardware and shared limits |
| NATS version and upgrades | You choose. Stormchaser's permissions were checked against NATS 2.10 | Operators choose; confirm the version supports filtered consumer-create subjects (2.9 or later) | Operators choose |
| Operating cost | A cluster to run, monitor and upgrade | Low: an account and two or three users | Lowest |
| Fits when | Workflows handle production credentials, or you cannot vouch for everyone on an existing cluster | You already run NATS as a platform and trust its operators with these secrets | Evaluation, or a cluster where every user is already trusted with everything Stormchaser handles |

Things to weigh:

- **A shared account is the one to avoid for production.** Isolation then
  rests on deny lists in other people's user definitions.
- **A dedicated account on a shared cluster isolates you from other tenants,
  not from the operators.** Anyone who administers the cluster can read the
  stream and publish into your account. If they are outside the group you
  would trust with your deployment credentials, use a dedicated cluster.
- **If you must use a server you do not fully trust,** keep long-lived secrets
  out of task messages: have steps fetch what they need at run time (for
  example from Vault with a short-lived token) instead of passing secrets in
  step environment variables, and set a short maximum age on the stream.
  Stormchaser has no payload encryption or message signing to fall back on.
- Running Stormchaser inside a NATS account (rather than the default global
  account) has not been tested by the project. It uses only standard
  JetStream APIs, so it is expected to work; verify it in your environment.

Whichever you choose, the requirements in the next section apply.

## Production requirements

### 1. Separate users with scoped permissions

Give the engine, the API, the query service and the runners an identity each.
NATS treats a user with no `permissions` block as unrestricted, and a grant of
`>` includes the JetStream control plane under `$JS.API.>`, so every user but
the engine's must have an explicit allow list.

**Engine**: publish and subscribe on `stormchaser.>`, `$JS.API.>`,
`$JS.ACK.>` and `_INBOX_stormchaser.>`, and publish on the other users' inboxes
(`_INBOX_stormchaser-runner.>` and so on) to answer them. It owns the stream, and it should be the only
identity that can publish a task (`stormchaser.v1.*.step.scheduled.>`): a task
tells a runner which container to start, with which secrets, and the engine
publishes one only after its policy check.

**API**: exactly the list below. The API faces users, so it is the component
most likely to be compromised, and with the engine's identity it could
schedule containers on runners directly and read the resolved secrets in task
messages. With this list it can queue runs, report the outcome of approvals
and waits (which the engine accepts only with the step's completion token),
watch run events for its event stream, and keep its rate-limit buckets.

```text
publish allow:
  stormchaser.v1.*.run.queued
  stormchaser.v1.run.direct
  stormchaser.v1.*.step.completed
  stormchaser.v1.*.step.failed
  $JS.API.INFO
  $JS.API.STREAM.CREATE.KV_api_rate_limit_buckets
  $JS.API.STREAM.INFO.KV_api_rate_limit_buckets
  $JS.API.DIRECT.GET.KV_api_rate_limit_buckets.>
  $JS.API.STREAM.MSG.GET.KV_api_rate_limit_buckets
  $KV.api_rate_limit_buckets.>

subscribe allow:
  _INBOX_stormchaser-api.>
  stormchaser.v1.run.>
  stormchaser.v1.*.run.>
```

**Query service**: it connects and does nothing else, so deny it all
publishing and allow it to subscribe to `_INBOX_stormchaser-query.>` only.

The API and query lists are the ones the quickstart ships and have been
checked against a NATS 2.10 server with the configuration from both the
compose file and the umbrella chart: the API's and the query service's whole
test suites pass under their own users, and the API is refused when it tries
to publish a task or a run's completion.

In the orchestration chart, give the API and the query service their Secrets
with `global.nats.apiAuthSecret` and `global.nats.queryAuthSecret`; left empty,
each falls back to the engine's `global.nats.authSecret`.

**Runners**: exactly the list below. It is the one the quickstart ships, and it
has been checked against a NATS 2.10 server: a runner starts, registers,
creates its consumers and pulls work with no permission errors, and is refused
when it tries to subscribe to `stormchaser.>`, create a consumer on any other
subject, pull from an engine consumer, publish a `run.*` or `step.scheduled.*`
event, or delete or read the stream directly.

```text
publish allow:
  stormchaser.v1.runner.register
  stormchaser.v1.global.runner.heartbeat
  stormchaser.v1.global.runner.offline
  stormchaser.v1.*.step.initializing
  stormchaser.v1.*.step.running
  stormchaser.v1.*.step.completed
  stormchaser.v1.*.step.failed
  stormchaser.v1.step.unpacking_sfs
  stormchaser.v1.step.packing_sfs
  stormchaser.rpc.step.query
  stormchaser.rpc.step.artifact_auth
  stormchaser.rpc.runner.session
  $JS.API.STREAM.INFO.stormchaser
  $JS.API.STREAM.CREATE.stormchaser
  $JS.API.CONSUMER.INFO.stormchaser.*
  $JS.API.CONSUMER.CREATE.stormchaser.*.stormchaser.v1.*.step.scheduled.>
  $JS.API.CONSUMER.CREATE.stormchaser.*.stormchaser.v1.runner.>
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.*
  $JS.ACK.stormchaser.>

publish deny (the engine's own consumers; one shard entry per engine shard):
  $JS.API.CONSUMER.CREATE.stormchaser.orchestration-engine-global.>
  $JS.API.CONSUMER.CREATE.stormchaser.orchestration-engine-legacy-run.>
  $JS.API.CONSUMER.CREATE.stormchaser.orchestration-engine-legacy-runner.>
  $JS.API.CONSUMER.CREATE.stormchaser.orchestration-engine-legacy-step.>
  $JS.API.CONSUMER.CREATE.stormchaser.orchestration-engine-shard-0.>
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-global
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-legacy-run
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-legacy-runner
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-legacy-step
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-shard-0
  $JS.ACK.stormchaser.orchestration-engine-global.>
  $JS.ACK.stormchaser.orchestration-engine-legacy-run.>
  $JS.ACK.stormchaser.orchestration-engine-legacy-runner.>
  $JS.ACK.stormchaser.orchestration-engine-legacy-step.>
  $JS.ACK.stormchaser.orchestration-engine-shard-0.>

subscribe allow:
  _INBOX_stormchaser-runner.>
```

`stormchaser.v1.runner.cancel` is deliberately missing from the runner's publish
list. The engine publishes there to stop the steps of a run that has ended, and
runners act on whatever arrives on it, so it must stay writable by the engine's
account alone. Runners read it through a consumer of their own, which the
`stormchaser.v1.runner.>` consumer entry already permits.

If you create the stream yourself, drop `$JS.API.STREAM.CREATE.stormchaser`
from the runner list. If you run more than one engine shard
(`STORMCHASER_TOTAL_SHARDS`), add the three deny entries for each shard number.

#### Inboxes

Replies (request answers, publish acknowledgements and pull-consumer
deliveries) arrive on an inbox subject. Every component uses an inbox prefix of
its own, `_INBOX_<NATS_USER>` (or `NATS_INBOX_PREFIX` when set, which you need
with a credentials file or a token, since those carry no user name), and each
user may subscribe only to its own prefix. Do not grant `_INBOX.>`: it holds the
engine's pull deliveries, which carry every runner's reports, and the answers
the engine sends each runner, so any user allowed to read it could read them
all. Runners share one user and so one prefix; the one answer that matters
between them, a step's registry push token, is sealed with the asking runner's
own key.

Prefer NKeys or operator-mode `.creds` files to passwords: no secret crosses
the wire. If you use passwords, store bcrypt hashes in the server
configuration and require TLS, because the client still sends the password
itself.

**What these permissions do not stop.** Runners share one credential, so NATS
cannot tell them apart. The engine does check the runner named in each report:
a step's result is accepted only from the runner the step is with, a step is
taken over from a runner only once that runner has missed a heartbeat, and a
step's registry token goes only to its own runner. That stops a runner that
does not know which runner holds a step. A compromised runner that learns
another runner's id can still name it in a report, because the name is part of
the message and not something NATS vouches for. A runner credential can also
read tasks scheduled for other runners of any type. It cannot finish steps the engine handles itself, such as
approvals: those need a token that never leaves the database. Treat every
runner host as trusted to that extent.

### 2. TLS on every link

- Enable TLS for client connections and, separately, for cluster routes.
  Enabling it for clients leaves routes in plaintext until the `cluster` block
  is configured too.
- Set `verify: true` on the client block if you want NATS to require client
  certificates. Without it, any client that trusts your CA connects without
  presenting one.
- Set `NATS_TLS_REQUIRED=true` and, for a private CA, `NATS_CA_FILE` on every
  Stormchaser component, and use a `tls://` URL.

### 3. Keep workflow steps away from NATS

Workflow authors choose the images that steps run. A step that can open a
connection to NATS can attack it directly, so the NATS client port must not be
reachable from step containers:

- **Kubernetes**: steps run in their own namespace, and the runner chart
  applies a NetworkPolicy there (`networkPolicy.enabled`, on by default). It
  denies all ingress and allows egress only to cluster DNS and to addresses
  outside the private, carrier-grade NAT and link-local ranges, so NATS,
  PostgreSQL, the Kubernetes API and the cloud metadata address
  (`169.254.169.254`) are out of reach. Check three things in your cluster:
  that the CNI enforces NetworkPolicy; that `networkPolicy.blockedCidrs`
  covers your pod and service ranges; and that cluster DNS is not served from
  a link-local address (NodeLocal DNSCache), which the default ranges block.
  If steps must reach an in-cluster object store, allow just that with
  `networkPolicy.allowReleaseNamespace`, as the umbrella chart does for MinIO.
- **Docker runner**: set `STORMCHASER_STEP_NETWORK` on the runner to a Docker
  network that has no route to NATS or PostgreSQL; step containers join it
  instead of the runner's own network. A network on the same host can still
  reach any port the control plane publishes on the host, so do not publish
  NATS or PostgreSQL beyond loopback on runner hosts. The bundled compose
  file does both: steps join a `steps` network that only object storage is
  attached to, and the PostgreSQL, NATS and OPA ports are published on
  `127.0.0.1` (`STORMCHASER_CONTROL_PLANE_BIND`).

`scripts/setup.sh` turns this isolation **off** for local development, because
the dogfood workflows run the test suite and deploy charts from inside steps.
Set `STORMCHASER_ISOLATE_STEPS=true` before running it to keep isolation on.

Also restrict the monitoring port (8222): it serves connection, user and
subject details in plaintext. On Kubernetes keep it bound for probes and limit
it with a NetworkPolicy; elsewhere bind it to localhost.

### 4. Availability and storage

- Run at least three clustered nodes and give the `stormchaser` stream three
  replicas. Stormchaser does not set a replica count, so a stream it creates
  gets the server default of one; create the stream yourself, or update it,
  to raise that.
- The `stormchaser` stream has a maximum message age of one week by default.
  Change it with `STORMCHASER_NATS_STREAM_MAX_AGE_HOURS` on the engine and
  runners (`0` for no limit). The engine applies it at start-up to a stream that
  has no limit; an age you set yourself (for example
  `nats stream edit stormchaser --max-age 72h`) is left alone. Choose an age
  longer than your longest-running step and longer than a task may wait for a
  runner: a task message that ages out before a runner takes it is lost.
- The engine erases each task message once its step has finished or its run has
  been archived. With restricted NATS permissions the engine's user needs
  `$JS.API.STREAM.MSG.GET.stormchaser`, `$JS.API.STREAM.MSG.DELETE.stormchaser`
  and, to set the age, `$JS.API.STREAM.UPDATE.stormchaser`. If it lacks them the
  engine logs a warning and messages stay until they age out.
- Use persistent, adequately sized file storage. Set a container memory limit
  above the JetStream memory-store size, and `GOMEMLIMIT` a little below the
  limit.
- For encryption at rest, NATS recommends filesystem-level encryption of the
  store directory over its built-in option.

### 5. Deploying against your own NATS with Helm

```bash
kubectl create secret generic nats-control -n stormchaser \
  --from-literal=user=stormchaser --from-literal=password='<engine password>'
kubectl create secret generic nats-api -n stormchaser \
  --from-literal=user=stormchaser-api --from-literal=password='<API password>'
kubectl create secret generic nats-query -n stormchaser \
  --from-literal=user=stormchaser-query --from-literal=password='<query password>'
kubectl create secret generic nats-runner -n stormchaser \
  --from-literal=user=stormchaser-runner --from-literal=password='<runner password>'

helm install stormchaser-orchestration ./deploy/charts/stormchaser-orchestration \
  --namespace stormchaser \
  --set global.nats.url="tls://nats.example.internal:4222" \
  --set global.nats.authSecret=nats-control \
  --set global.nats.apiAuthSecret=nats-api \
  --set global.nats.queryAuthSecret=nats-query

helm install stormchaser-runner ./deploy/charts/stormchaser-runner-k8s \
  --namespace stormchaser \
  --set config.natsUrl="tls://nats.example.internal:4222" \
  --set config.natsAuthSecret=nats-runner
```

With the umbrella chart, set `nats.enabled=false` and
`natsAuth.generate=false`, and point `stormchaser-orchestration.global.nats`
and `stormchaser-runner-k8s.config` at your server and Secrets.

### Checklist

- [ ] NATS rejects anonymous connections.
- [ ] The engine, the API, the query service and the runners each use their
      own credentials.
- [ ] The API and query users have the allow lists above; only the engine can
      publish `step.scheduled` subjects.
- [ ] The runner user has the allow and deny lists above, with a deny entry
      for every engine shard.
- [ ] TLS is on for clients and routes; components set `NATS_TLS_REQUIRED`.
- [ ] Step containers and pods cannot reach port 4222.
- [ ] Port 8222 is not reachable outside the NATS hosts or namespace.
- [ ] Three or more nodes; the `stormchaser` stream has three replicas.
- [ ] The store directory is on persistent, encrypted storage.
- [ ] The `stormchaser` stream has a maximum age.
- [ ] You have decided between a dedicated cluster and a shared one, and
      everyone who can read or publish on it is trusted with workflow secrets.
