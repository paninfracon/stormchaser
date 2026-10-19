# GitCheckout Step

The `GitCheckout` step clones one ref of a Git repository, usually into storage that later steps mount. The engine turns it into a `RunContainer` step that runs `git` in the `alpine/git` image (pinned by digest; the operator can replace it with `STORMCHASER_GIT_IMAGE`), so it runs on a runner like any container step.

The checkout is shallow and fetches no file contents until they are checked out (`git fetch --depth <depth> --filter=blob:none`). The repository, ref, destination and paths reach the script as environment variables and arguments, never as part of its text.

## DSL Specification

The `spec` block for `GitCheckout` supports the following fields:

* `repo` (string, required): The repository URL. Only `https://` and `ssh://` URLs, and scp-style `git@host:org/repo.git`, are accepted; the operator can change the list with `STORMCHASER_GIT_ALLOWED_SCHEMES`. Local paths and `file://` are refused.
* `ref` (string, optional): The branch, tag or commit to fetch. Defaults to `main`.
* `depth` (integer, optional): Fetch depth. Defaults to 1.
* `destination` (string, optional): The directory to check out into, in the container. Defaults to `.`, the container's working directory. Point it into a mounted volume to keep the checkout.
* `sparse_checkout` (list of strings, optional): Paths to check out, using `git sparse-checkout`. Without it the whole tree is checked out.
* `storage_mounts` (list of objects, optional): Storage to mount, as for [RunContainer](RunContainer.md).
* `connection` (string, optional): The name of a `git` connection with credentials for the repository. The run's user must be allowed to use it. A connection with a `username` in its config sends its credential as that user's password; one without sends it as a bearer token; one with an `ssh_key` in its config uses the key for SSH. The credentials are masked in logs.

A spec without `repo` is not turned into a container step, and no runner takes it.

## Example

```hcl
step "checkout" "GitCheckout" {
  spec = {
    repo           = "https://github.com/acme/app.git"
    ref            = "${inputs.branch}"
    destination    = "/workspace/source"
    storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
  }
  next = ["build"]
}
```
