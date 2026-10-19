# RestApi Step

The `RestApi` step makes an HTTP or HTTPS call. The engine runs it itself; no runner is involved. The request body is a MiniJinja template, and outputs can be taken from the response with JSON pointers or regular expressions.

Requests go through the engine's outbound guard: hosts on private and internal networks are refused unless the operator lists them in `STORMCHASER_SSRF_ALLOWED_HOSTS`, and redirects are not followed.

## DSL Specification

The `spec` block for `RestApi` supports the following fields:

* `url` (string, required): The endpoint to send the request to. With a `connection`, a path such as `/invoices` is appended to the connection's `base_url`. The URL is not a Jinja template; use `${...}` to put values in it.
* `connection` (string, optional): The name of an `http_api` connection whose headers and bearer token are added to the request. See [Using a connection](#using-a-connection).
* `method` (string, optional): `GET` (the default), `POST`, `PUT`, `DELETE` or `PATCH`. Any other value is sent as `GET`.
* `headers` (object, optional): Key-value pairs of HTTP headers.
* `body` (string, optional): The request body, a MiniJinja template with `inputs`, `steps` (`steps.<name>.outputs.<key>`) and `run` (`run.id`) in its context.
* `timeout` (string, optional): How long to wait for a response, such as `"15s"` or `"1m"`. Defaults to 30 seconds.
* `extractors` (list of objects, optional): Rules for extracting outputs from the response.

### Output Extractors

An output extractor allows you to capture parts of the response body and expose them as step outputs for subsequent steps to use.

An extractor object has the following fields:

* `name` (string, required): The name of the output variable to create.
* `format` (string, optional): `"json"` or `"regex"`. Without it, an extractor with only `json_pointer` is a JSON one and one with only `regex` a regex one. An extractor whose kind cannot be told, or a `regex` one with no `regex`, is skipped.
* `json_pointer` (string, optional): For a JSON extractor, a JSON Pointer (`"/data/items/0/id"`) or a dot-separated path (`"data.items.0.id"`). Without it the whole body is taken. A path that matches nothing sets no output.
* `regex` (string, optional): For a regex extractor, the regular expression to match against the response body. The first match is used.
* `group` (number, optional): For a regex extractor, the capture group to take. Defaults to 1.
* `sensitive` (boolean, optional): Marks the extracted output as sensitive when it is persisted.

Whatever the extractors, the whole response body is kept in the output `response`: the parsed JSON, or `{"text": "<body>"}` when it is not JSON. Bodies over 10 MiB fail the step. A status outside 2xx fails the step, with the response body in the error.

> [!WARNING]
> The `response` output always includes the full parsed body and is not automatically marked sensitive. If the response may include secrets, avoid persisting/printing `response` directly and use `extractors` with `sensitive = true` for the specific fields that need to flow to later steps.

## Using a connection

An `http_api` connection supplies a `base_url` (or, in older connections, `url`), optional `headers` and an optional bearer token, so the workflow does not have to carry the credential itself. The connection's headers are added to the step's, replacing any of the same name, and its token is sent as `Authorization: Bearer <token>` unless the request already has an `Authorization` header. The run's user must be allowed to use the connection. A connection that does not exist fails the step; one of another type is ignored.

The connection's headers and token are only sent to the connection's own server. The request URL must be a path relative to `base_url`, or an absolute URL with the same scheme, host and port as `base_url`; any other URL fails the step before a request is made. A connection that has headers or a token but no `base_url` cannot be used by this step. A connection with neither places no restriction on the URL.

```hcl
step "list_invoices" "RestApi" {
  spec = {
    connection = "billing-api"
    url        = "/invoices?status=open"
  }
}
```

## Example

```hcl
step "fetch_user_data" "RestApi" {
  spec = {
    url    = "https://api.example.com/users/${inputs.user_id}"
    method = "GET"
    headers = {
      "Accept"        = "application/json"
      "Authorization" = "Bearer ${secrets.API_TOKEN}"
    }
    timeout = "15s"

    extractors = [
      {
        name         = "user_email"
        format       = "json"
        json_pointer = "/data/email"
      },
      {
        name      = "session_token"
        format    = "regex"
        regex     = "Token is ([A-Z0-9]+)"
        group     = 1
        sensitive = true
      }
    ]
  }
}
```
