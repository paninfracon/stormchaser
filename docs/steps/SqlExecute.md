# SqlExecute Step

The `SqlExecute` step runs one SQL statement on a database connection from the
engine, and reports the number of rows it affected. The engine runs it itself; no
runner is involved. Only PostgreSQL connections are supported; any other type fails
the step.

## DSL Specification

The `spec` block for `SqlExecute` supports the following fields:

* `connection` (string, required): The name of a `postgres` connection. The run's
  user must be allowed to use it, and its configuration must have a `url`.
* `query` (string, required): The statement to run. It is a MiniJinja template, with
  `inputs`, `steps` and `run` in its context.
* `params` (list, optional): Values for the statement's `$1`, `$2`, ... placeholders.
  They are sent to the database separately from the statement, so a value is never
  read as SQL. Strings are sent as `text`, whole numbers as `bigint`, other numbers
  as `double precision`, booleans as `boolean`, `null` as a `text` null, and lists
  and objects as `jsonb`. Where a column needs another type, cast the placeholder in
  the statement (`$1::int`, `$2::uuid`).

Any value that comes from an input, a step output or another outside source belongs
in `params`. Written into `query`, whether with `${...}` or `{{ ... }}`, it becomes
part of the SQL, and a value such as `x' OR '1'='1` changes what the statement does.

### Outputs

* `result`: An object whose `rows_affected` is the number of rows the statement affected or returned. A later step reads it as `${steps.<name>.outputs.result.rows_affected}`. Rows a query returns are not kept.

## Example

```hcl
step "record_deploy" "SqlExecute" {
  spec = {
    connection = "deployments-db"
    query      = "INSERT INTO deploys (service, version, run_id) VALUES ($1, $2, $3::uuid)"
    params     = ["${inputs.service}", "${inputs.version}", "${run.id}"]
  }
}
```
